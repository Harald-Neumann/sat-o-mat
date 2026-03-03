//! Manages sat-o-mat's [`crate::Task`] schedule.
//!
//! The schedule consists of a set of files in a specific directory structure corresponding to the
//! Task state.
//! A `Task` can be in the *Active*, *PendingApproval*, *Completed* or *Failed*
//! state.
//!
//! All `Task`s start in the *Active* or *PendingApproval* state.
//! `Task`s in the *PendingApproval* state move to the *Active* state when approved by an authorized
//! user (via the Web UI, API, or manually by moving the files.)
//! After an *Active* `Task` has finished executing, it is moved to the *Completed* or *Failed* state.
//!
//! Each `Task` has a unique identifier given by its filename.
//! The unique identifier typically has the following structure, although it is not mandatory:
//!
//! `task_template_name.<RFC3339 start timestamp>.<4 character UUID>.yaml`
//!
//! It is not possible for two `Task`s to have the same unique identifier.
//!
//! Example directory structure:
//!
//!

use std::{
    collections::HashMap,
    io,
    path::Path,
    string::FromUtf8Error,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use chrono::{DateTime, Utc};
use notify::{
    EventKind, Watcher,
    event::{CreateKind, RemoveKind},
};
use thiserror::Error;
use tokio::sync::Notify;
use tracing::{debug, error, info, warn};

use crate::Task;
use crate::task::runner::{self, RunConfig};

#[derive(Debug, Error)]
pub enum Error {
    #[error("IO error")]
    Io(#[from] io::Error),
    #[error("UTF-8 conversion error")]
    FromUtf8(#[from] FromUtf8Error),
    #[error("Invalid Task definition at {0}: {1}")]
    TaskFormat(String, crate::task::format::Error),
    #[error("Error starting directory watcher")]
    NotifyWatcher(#[from] notify::Error),
}

/// Monitors a directory structure containing Task descriptions and executes them at the corresponding time.
pub async fn run(base: &Path) -> Result<(), Error> {
    let active_path = base.join("Active");
    let failed_path = base.join("Failed");
    let completed_path = base.join("Completed");
    let artifact_base = base.join("Artifacts");

    // Create all directories if they do not exist.
    for dir in [&active_path, &failed_path, &completed_path, &artifact_base] {
        std::fs::create_dir_all(dir)?;
    }

    // Load Active Task definitions
    let mut tasks: HashMap<String, Task> = HashMap::new();
    for entry in std::fs::read_dir(&active_path)? {
        parse_task_or_move_to_failed(&mut tasks, &entry?.path(), &failed_path)?;
    }

    // Spawn Active directory watcher in a background OS thread (notify crate is sync)
    let tasks = Arc::new(Mutex::new(tasks));
    let notify = Arc::new(Notify::new());
    {
        let tasks = tasks.clone();
        let active_path = active_path.clone();
        let failed_path = failed_path.clone();
        let notify = notify.clone();
        thread::spawn(move || {
            if let Err(e) = directory_watcher(tasks, &active_path, &failed_path, notify) {
                error!(?e, "watcher exited with error");
            }
        });
    }

    // Main loop
    loop {
        // Find the next task that should be run by start time
        let next = {
            let tasks = tasks.lock().unwrap();
            next_to_run(&tasks)
        };

        // Calculate time to wait until the next Task should start
        let sleep_dur = match next {
            Some((_, start)) => (start - Utc::now()).to_std().unwrap_or_default(),
            None => Duration::from_secs(3600),
        };

        tokio::select! {
            _ = notify.notified() => {
                // The set of Active Tasks has changed.
                // Re-run the loop.
                continue;
            }
            _ = tokio::time::sleep(sleep_dur) => {}
        }

        // Get the unique ID of the next Task to run.
        let Some((unique_id, _)) = next else {
            // There is no next Task.
            // Re-run the loop to check again.
            continue;
        };
        // Remove the next task from the active map; skip if the watcher already removed it.
        let Some(task) = tasks.lock().unwrap().remove(&unique_id) else {
            continue;
        };

        let task_path = active_path.join(&unique_id);
        let failed_path = failed_path.clone();
        let completed_path = completed_path.clone();
        let task_stem = Path::new(&unique_id)
            .file_stem()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let config = RunConfig {
            artifact_base: artifact_base.join(task_stem),
        };

        info!(%unique_id, "spawning runner for task");
        tokio::spawn(async move {
            let outcome = runner::run(task, config).await;

            let dest = match &outcome {
                Ok(o) if !o.aborted() => &completed_path,
                _ => &failed_path,
            };

            if let Err(e) = tokio::fs::rename(&task_path, dest.join(&unique_id)).await {
                error!(?e, %unique_id, "failed to move task file after completion");
            }
        });
    }
}

fn directory_watcher(
    tasks: Arc<Mutex<HashMap<String, Task>>>,
    active_path: &Path,
    failed_path: &Path,
    notify: Arc<Notify>,
) -> Result<(), Error> {
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(tx)?;
    watcher.watch(active_path, notify::RecursiveMode::NonRecursive)?;

    for res in rx {
        let event = res?;
        debug!(?event, "got event");

        let mut tasks = tasks.lock().unwrap();
        let changed = match event.kind {
            EventKind::Create(CreateKind::File) => {
                let path = &event.paths[0];
                info!(?path, "file created");
                parse_task_or_move_to_failed(&mut tasks, path, failed_path)?;
                true
            }
            EventKind::Remove(RemoveKind::File) => {
                let path = &event.paths[0];
                info!(?path, "file removed");
                let unique_id = path_to_unique_id(path);
                tasks.remove(&unique_id);
                true
            }
            _ => {
                debug!(?event, "unhandled event");
                false
            }
        };

        if changed {
            notify.notify_one();
        }
    }

    Ok(())
}

fn parse_task_or_move_to_failed(
    tasks: &mut HashMap<String, Task>,
    path: &Path,
    failed_path: &Path,
) -> Result<(), Error> {
    let task_str = String::from_utf8(std::fs::read(path)?)?;
    let unique_id = path_to_unique_id(path);

    match Task::from_yaml_str(&task_str) {
        Ok(task) => {
            info!(?path, "loaded task");
            tasks.insert(unique_id, task);
        }
        Err(e) => {
            std::fs::rename(path, failed_path.join(path.file_name().unwrap()))?;
            warn!(
                ?path,
                ?e,
                "error reading task definition file, moved to Failed"
            );
        }
    }

    Ok(())
}

/// Returns the task with the earliest `start` time, or the task that should run immediately
/// if no `start` variable is set.
fn next_to_run(tasks: &HashMap<String, Task>) -> Option<(String, DateTime<Utc>)> {
    tasks
        .iter()
        .map(|(id, task)| {
            let start = task
                .variables
                .get("start")
                .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(Utc::now);
            (id.clone(), start)
        })
        .min_by_key(|(_, start)| *start)
}

fn path_to_unique_id(path: &Path) -> String {
    path.file_name().unwrap().to_str().unwrap().to_string()
}
