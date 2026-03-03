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
//! Each `Task` has a unique identifier given by its filename (without extension.)
//! The unique identifier typically has the following structure, although it is not mandatory:
//!
//! `task_template_name.<RFC3339 start timestamp>.<4 character UUID>`
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
};

use notify::{
    Watcher,
    event::{CreateKind, RemoveKind},
};
use thiserror::Error;
use tracing::{debug, error, info, warn};

use crate::Task;

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
pub fn run(base: &Path) -> Result<(), Error> {
    let active_path = base.join("Active");
    let failed_path = base.join("Failed");

    // Load Active Task definitions
    let mut tasks: HashMap<String, Task> = HashMap::new();
    for entry in std::fs::read_dir(&active_path)? {
        parse_task_or_move_to_failed(&mut tasks, &entry?.path(), &failed_path)?;
    }

    // Spawn Active directory watcher
    let tasks = Arc::new(Mutex::new(tasks));
    let _watcher_handle =
        thread::spawn(move || directory_watcher(tasks.clone(), &active_path, &failed_path));

    // Main loop: wait until one of the following happens:
    // - The set of Active Tasks has changed
    // - The time to start an Active Task has been reached -> spawn Runner

    Ok(())
}

fn directory_watcher(
    tasks: Arc<Mutex<HashMap<String, Task>>>,
    active_path: &Path,
    failed_path: &Path,
) -> Result<(), Error> {
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(tx)?;
    watcher.watch(active_path, notify::RecursiveMode::NonRecursive)?;

    for res in rx {
        let event = res.map_err(|e| {
            error!(?e, "watcher error");
            e
        })?;

        debug!(?event, "got event");
        let mut tasks = tasks.lock().unwrap();

        match event.kind {
            notify::EventKind::Create(CreateKind::File) => {
                let path = &event.paths[0];
                info!(?path, "file created");
                parse_task_or_move_to_failed(&mut tasks, path, failed_path)?;
            }
            notify::EventKind::Remove(RemoveKind::File) => {
                let path = &event.paths[0];
                info!(?path, "file removed");
                let unique_id = path_to_unique_id(path);
                tasks.remove(&unique_id);
            }
            _ => {
                debug!(?event, "unhandled event");
            }
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

fn path_to_unique_id(path: &Path) -> String {
    path.file_stem().unwrap().to_str().unwrap().to_string()
}
