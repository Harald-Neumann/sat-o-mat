use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::{fs, io};

use chrono::{DateTime, Utc};
use tokio::process::Command;
use tokio::sync::broadcast;
use tokio::sync::broadcast::Receiver;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::time::{Duration, sleep};
use tokio::{spawn, task};
use tracing::{info, warn};

use crate::task::format::{self, OnFail, Step, Task, TimeSpec};
use crate::task::utils::{resolve_time, resolve_variables, substitute_variables};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to create artifact directory: {0}")]
    ArtifactDir(std::io::Error),
    #[error("IO error during variable resolution: {0}")]
    VariableResolveIo(std::io::Error),
    #[error("Format error: {0}")]
    Format(format::Error),
    #[error("IO error: {0}")]
    Io(io::Error),
}

pub struct RunConfig {
    pub artifact_base: PathBuf,
}

#[derive(Debug)]
pub struct RunOutcome {
    pub artifact_dir: PathBuf,
    pub step_outcomes: Vec<StepOutcome>,
}

impl RunOutcome {
    pub fn aborted(&self) -> bool {
        self.step_outcomes
            .iter()
            .any(|o| matches!(o, StepOutcome::Abort { .. }))
    }
}

#[derive(Debug, Clone)]
pub enum StepOutcome {
    Completed { cmd: String, status: ExitStatus },
    Abort { cmd: String, reason: AbortReason },
    SpawnError { cmd: String, error: String },
}

#[derive(Debug, Clone)]
pub enum AbortReason {
    ExitStatus(ExitStatus),
    ExitSignalReceived,
    SpawnError(String),
}
impl From<&StepOutcome> for Option<AbortReason> {
    fn from(value: &StepOutcome) -> Self {
        match value {
            StepOutcome::Completed { cmd: _, status } if !status.success() => {
                Some(AbortReason::ExitStatus(*status))
            }
            StepOutcome::SpawnError { cmd: _, error } => {
                Some(AbortReason::SpawnError(error.clone()))
            }
            _ => None,
        }
    }
}

pub async fn run(task: Task, config: RunConfig) -> Result<RunOutcome, Error> {
    // Create artifact directory
    let artifact_dir = config.artifact_base;
    tokio::fs::create_dir_all(&artifact_dir)
        .await
        .map_err(Error::ArtifactDir)?;

    info!(?artifact_dir, "created artifact directory");

    // Resolve variables (evaluate ${...} shell commands)
    let mut task = task.clone();
    resolve_variables(&mut task.variables, &artifact_dir)
        .await
        .map_err(Error::VariableResolveIo)?;

    // Resolve start/end timestamps
    let start_time = task.get_time_variable("start").map_err(Error::Format)?;
    let end_time = task.get_time_variable("end").ok();

    // Write task with variables resolved to the artifacts directory
    let resolved_task_yaml = serde_yaml::to_string(&task).unwrap();
    fs::write(artifact_dir.join("task.yml"), resolved_task_yaml).map_err(Error::Io)?;

    // If start is in the future, wait
    sleep_until(start_time).await;

    // Run main steps with end-time deadline
    let step_outcomes = run_steps(task.steps, &task.variables, &artifact_dir, end_time).await;

    // Cleanup steps
    let _ = run_steps(task.cleanup, &task.variables, &artifact_dir, None).await;

    Ok(RunOutcome {
        artifact_dir,
        step_outcomes,
    })
}

/// Spawns a step runner and monitors the outcome of each task, returning a Vec of StepOutcomes.
async fn run_steps(
    steps: Vec<Step>,
    vars: &HashMap<String, String>,
    cwd: &Path,
    end_time: Option<DateTime<Utc>>,
) -> Vec<StepOutcome> {
    let mut outcomes = Vec::new();
    let (outcome_tx, mut outcome_rx) = mpsc::unbounded_channel();
    let (exit_tx, exit_rx) = broadcast::channel(1);

    info!(?steps, ?end_time);

    // Spawner task
    let spawner = spawn(spawn_steps(
        steps,
        vars.clone(),
        cwd.to_path_buf(),
        exit_tx.clone(),
        exit_rx,
        outcome_tx,
    ));

    // Monitor loop
    let deadline = sleep_until(end_time.unwrap_or(Utc::now()));
    let mut deadline_fired = false;
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline, if end_time.is_some() && !deadline_fired => {
                deadline_fired = true;
                info!("deadline reached. Sending exit signal.");
                let _ = exit_tx.send(());
            }
            outcome = outcome_rx.recv() => {
                if outcome.is_none() {
                    info!("all senders exited");
                    break;
                }
                let outcome = outcome.unwrap();
                outcomes.push(outcome.clone());

                if let StepOutcome::Abort { cmd, reason } = outcome {
                    warn!(?cmd, ?reason, "aborted, sending exit signal");
                    let _ = exit_tx.send(());
                } else {
                    info!(?outcome, "got outcome");
                }
            }
        }
    }

    // Wait until all tasks and children have exited
    let handles = spawner.await.unwrap();
    for h in handles {
        h.await.unwrap();
    }

    // Return collected StepOutcomes
    outcomes
}

/// Spawns tasks at their configured time, waiting for completion if necessary, and sends outcomes
/// to `outcome_tx`.
///
/// Returns a Vec of JoinHandles for tasks that are not yet completed (i.e. non-waited tasks.)
async fn spawn_steps(
    steps: Vec<Step>,
    vars: HashMap<String, String>,
    cwd: PathBuf,
    exit_tx: broadcast::Sender<()>,
    mut exit_rx: Receiver<()>,
    outcome_tx: UnboundedSender<StepOutcome>,
) -> Vec<task::JoinHandle<StepOutcome>> {
    let mut handles = Vec::new();
    for step in steps {
        // If step.time is set, resolve it
        let step_start = step.time.and_then(|t| resolve_time(&t, &vars));

        // Wait until the step start time is reached (if configured),
        // while checking if the exit signal has been sent.
        let should_execute_step = wait_for_step_start_or_abort(step_start, &mut exit_rx).await;
        if !should_execute_step {
            break;
        }

        // Substitute variables in step.cmd
        let cmd = substitute_variables(&step.cmd, &vars);
        info!(cmd = %cmd, wait = step.wait, "executing step");

        let (abort_on_fail, max_attempts) = match &step.on_fail {
            OnFail::Continue => (false, 1),
            OnFail::Abort => (true, 1),
            OnFail::Retry(n) => (true, *n),
        };

        // Spawn the command for this step
        let step_handle = spawn(run_step(
            cmd.clone(),
            abort_on_fail,
            max_attempts,
            cwd.to_path_buf(),
            exit_tx.subscribe(),
            outcome_tx.clone(),
        ));

        if step.wait {
            // Wait for the current step to finish executing before continuing
            let outcome = step_handle.await.unwrap();
            if matches!(outcome, StepOutcome::Abort { .. }) {
                // If the outcome is an Abort, stop spawning new step tasks
                break;
            }
        } else {
            // Do not wait for the current step to finish executing.
            // Add it to the list of still-running handles
            handles.push(step_handle);
        }
    }
    handles
}

/// Executes the command for a specific step, retrying if configured,
/// and sends the `StepOutcome` to `tx`.
/// Returns the `StepOutcome`.
async fn run_step(
    cmd: String,
    abort_on_fail: bool,
    max_attempts: u32,
    cwd: PathBuf,
    mut exit_rx: Receiver<()>,
    tx: UnboundedSender<StepOutcome>,
) -> StepOutcome {
    let mut outcome: Option<StepOutcome> = None;

    for _i in 1..=max_attempts {
        // Try to spawn a child process for `cmd`
        let mut child = match spawn_command(&cmd, &cwd) {
            Ok(child) => {
                info!(pid = ?child.id(), cmd = cmd, "spawned child");
                child
            }
            Err(e) => {
                // Child failed to spawn
                warn!(?e, "spawn error");

                outcome = Some(StepOutcome::SpawnError {
                    cmd: cmd.clone(),
                    error: e.to_string(),
                });
                continue;
            }
        };

        tokio::select! {
            exit = child.wait() => {
                // Child finished running
                match exit {
                    Ok(status) => {
                        // Successful exit
                        info!(?cmd, ?status, "child exited");
                        outcome = Some(StepOutcome::Completed {
                            cmd: cmd.clone(),
                            status
                        });

                        // Child has finished successfully, break retry loop
                        if status.success() {
                            break;
                        }
                    }
                    Err(e) => {
                        // Child did not spawn successfully
                        warn!(?cmd, ?e, "child failed to spawn");
                        outcome = Some(StepOutcome::SpawnError {
                            cmd: cmd.clone(),
                            error: e.to_string()
                        });
                    }
                }
            }

            _ = exit_rx.recv() => {
                // Exit signal (abort or deadline)
                info!(child = ?child, "exit signal received, killing child");
                child.kill().await.unwrap();
                outcome = Some(StepOutcome::Abort {
                    cmd: cmd.clone(),
                    reason: AbortReason::ExitSignalReceived
                });
                break;
            }
        }
    }

    let mut outcome = outcome.expect("a step outcome should be deterimned by this point");

    if abort_on_fail && let Some(reason) = (&outcome).into() {
        // If the last outcome was an unsuccessful exit and `abort_on_fail` is set,
        // change the outcome to aborted
        outcome = StepOutcome::Abort {
            cmd: cmd.clone(),
            reason,
        };
    };

    // Send step outcome to monitor loop
    let _ = tx.send(outcome.clone());

    outcome
}

async fn wait_for_step_start_or_abort(
    step_start: Option<DateTime<Utc>>,
    abort_rx: &mut Receiver<()>,
) -> bool {
    let step_start = sleep_until(step_start.unwrap_or(Utc::now()));

    tokio::select! {
        _ = step_start => {
            info!("step start time reached");
            true
        }
        _ = abort_rx.recv() => {
            info!("waiting aborted due to exit signal");
            false
        }
    }
}

/// Create a sleep future that resolves at `target` (or immediately if already past).
fn sleep_until(target: DateTime<Utc>) -> tokio::time::Sleep {
    let dur = (target - Utc::now()).to_std().unwrap_or(Duration::ZERO);
    sleep(dur)
}

/// Run `sh -c "cmd"` with CWD set to the given directory.
fn spawn_command(cmd: &str, cwd: &Path) -> std::io::Result<tokio::process::Child> {
    Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .spawn()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task::format::{OnFail, Step, Task, TimeSpec};
    use chrono::TimeDelta;

    fn init_tracing() {
        tracing_subscriber::fmt()
            .with_test_writer()
            .with_env_filter("sat_o_mat=debug")
            .try_init()
            .ok();
    }

    // --- Integration tests ---

    fn make_task(steps: Vec<Step>, cleanup: Vec<Step>) -> Task {
        Task::new(HashMap::new(), steps, cleanup)
    }

    fn step(cmd: &str) -> Step {
        Step {
            cmd: cmd.into(),
            time: None,
            wait: false,
            on_fail: OnFail::Abort,
        }
    }

    fn waited(cmd: &str) -> Step {
        Step {
            cmd: cmd.into(),
            time: None,
            wait: true,
            on_fail: OnFail::Abort,
        }
    }

    fn waited_continue(cmd: &str) -> Step {
        Step {
            cmd: cmd.into(),
            time: None,
            wait: true,
            on_fail: OnFail::Continue,
        }
    }

    fn waited_retry(cmd: &str, n: u32) -> Step {
        Step {
            cmd: cmd.into(),
            time: None,
            wait: true,
            on_fail: OnFail::Retry(n),
        }
    }

    async fn run_with_tempdir(task: Task) -> RunOutcome {
        init_tracing();
        let temp = tempfile::tempdir().expect("failed to create temp dir");
        let config = RunConfig {
            artifact_base: temp.path().to_path_buf(),
        };
        run(task, config).await.expect("run should succeed")
    }

    #[tokio::test]
    async fn simple_echo_step() {
        let task = make_task(vec![waited("echo hello")], vec![]);
        let outcome = run_with_tempdir(task).await;
        assert_eq!(outcome.step_outcomes.len(), 1);
        assert!(matches!(
            &outcome.step_outcomes[0],
            StepOutcome::Completed { cmd, status } if cmd == "echo hello" && status.success()
        ));
    }

    #[tokio::test]
    async fn wait_true_blocks_until_complete() {
        // Use a command that takes a moment but succeeds
        let task = make_task(vec![waited("sleep 0.1 && echo done")], vec![]);
        let outcome = run_with_tempdir(task).await;
        assert!(matches!(
            &outcome.step_outcomes[0],
            StepOutcome::Completed { status, .. } if status.success()
        ));
    }

    #[tokio::test]
    async fn on_fail_abort_stops_execution() {
        let task = make_task(vec![waited("false"), waited("echo should not run")], vec![]);
        let outcome = run_with_tempdir(task).await;
        // Only one step outcome (the failed one); second step was skipped
        assert_eq!(outcome.step_outcomes.len(), 1);
        assert!(matches!(
            &outcome.step_outcomes[0],
            StepOutcome::Abort { reason: AbortReason::ExitStatus(status), .. } if !status.success()
        ));
    }

    #[tokio::test]
    async fn on_fail_continue_proceeds() {
        let task = make_task(
            vec![waited_continue("false"), waited("echo still running")],
            vec![],
        );
        let outcome = run_with_tempdir(task).await;
        assert!(matches!(
            &outcome.step_outcomes[0],
            StepOutcome::Completed { status, .. } if !status.success()
        ));
        assert!(matches!(
            &outcome.step_outcomes[1],
            StepOutcome::Completed { status, .. } if status.success()
        ));
    }

    #[tokio::test]
    async fn on_fail_retry_succeeds_eventually() {
        // This command fails the first time (file doesn't exist),
        // creates the file, then succeeds on retry.
        let task = Task::new(
            HashMap::new(),
            vec![waited_retry(
                "test -f attempt_marker || { touch attempt_marker; false; }",
                2,
            )],
            vec![waited("rm -f attempt_marker")],
        );

        let outcome = run_with_tempdir(task).await;
        assert_eq!(outcome.step_outcomes.len(), 1);
        assert!(matches!(
            &outcome.step_outcomes[0],
            StepOutcome::Completed { status, .. } if status.success()
        ));
    }

    #[tokio::test]
    async fn cleanup_always_runs_after_abort() {
        init_tracing();
        let task = make_task(vec![waited("false")], vec![waited("touch cleanup_ran")]);

        let temp = std::env::temp_dir().join(format!("sat-o-mat-cleanup-{}", std::process::id()));
        let config = RunConfig {
            artifact_base: temp.clone(),
        };
        let outcome = run(task, config).await.expect("run should succeed");
        assert!(outcome.aborted());
        // Cleanup should have run -- check for the file in the artifact dir
        assert!(outcome.artifact_dir.join("cleanup_ran").exists());

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[tokio::test]
    async fn end_time_deadline_kills_long_step() {
        // End time 1 second from now; step sleeps 60 seconds
        let end = Utc::now() + TimeDelta::seconds(1);
        let task = Task::new(
            HashMap::from([("end".into(), end.to_rfc3339())]),
            vec![waited("sleep 60")],
            vec![],
        );

        let start = std::time::Instant::now();
        let outcome = run_with_tempdir(task).await;
        let elapsed = start.elapsed();

        assert!(outcome.aborted());
        // Should complete in ~1s + 3s grace period, not 60s
        assert!(elapsed.as_secs() < 10);
    }

    #[tokio::test]
    async fn shell_variable_resolution() {
        let task = Task::new(
            HashMap::from([("greeting".into(), "${echo hello}".into())]),
            vec![waited("echo $greeting")],
            vec![],
        );
        let outcome = run_with_tempdir(task).await;
        assert!(!outcome.aborted());
        assert_eq!(outcome.step_outcomes.len(), 1);
    }

    #[tokio::test]
    async fn artifact_directory_is_created() {
        init_tracing();
        let task = make_task(vec![waited("pwd")], vec![]);

        let temp = std::env::temp_dir().join(format!("sat-o-mat-artifact-{}", std::process::id()));
        let config = RunConfig {
            artifact_base: temp.clone(),
        };
        let outcome = run(task, config).await.expect("run should succeed");
        assert!(outcome.artifact_dir.exists());
        assert!(outcome.artifact_dir.starts_with(&temp));

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[tokio::test]
    async fn background_step_returns_spawned() {
        let task = make_task(vec![step("sleep 0.1")], vec![]);
        let outcome = run_with_tempdir(task).await;
        assert_eq!(outcome.step_outcomes.len(), 1);
        assert!(matches!(
            &outcome.step_outcomes[0],
            StepOutcome::Completed { .. }
        ));
    }

    fn background_abort(cmd: &str) -> Step {
        Step {
            cmd: cmd.into(),
            time: None,
            wait: false,
            on_fail: OnFail::Abort,
        }
    }

    fn background_continue(cmd: &str) -> Step {
        Step {
            cmd: cmd.into(),
            time: None,
            wait: false,
            on_fail: OnFail::Continue,
        }
    }

    #[tokio::test]
    async fn background_abort_on_fail_stops_execution() {
        // Background step exits immediately with failure; the next step is a
        // long wait that should be interrupted by the background failure.
        let task = make_task(vec![background_abort("false"), waited("sleep 60")], vec![]);

        let start = std::time::Instant::now();
        let outcome = run_with_tempdir(task).await;
        let elapsed = start.elapsed();

        assert!(outcome.aborted());
        // Should abort quickly, not wait 60s
        assert!(elapsed.as_secs() < 10);
    }

    #[tokio::test]
    async fn background_continue_on_fail_does_not_abort() {
        // Background step fails with on_fail: continue — execution should proceed.
        let task = make_task(
            vec![background_continue("false"), waited("echo still here")],
            vec![],
        );
        let outcome = run_with_tempdir(task).await;
        assert!(!outcome.aborted());
    }

    #[tokio::test]
    async fn aborting_background_step_prevents_future_timed_step_spawn() {
        let task = Task::new(
            HashMap::new(),
            vec![
                background_abort("false"),
                Step {
                    cmd: "touch spawned_after_abort".into(),
                    time: Some(TimeSpec::Relative {
                        variable: "start".into(),
                        offset: TimeDelta::seconds(1),
                    }),
                    wait: true,
                    on_fail: OnFail::Abort,
                },
            ],
            vec![],
        );

        let outcome = run_with_tempdir(task).await;
        assert!(outcome.aborted());
        assert_eq!(outcome.step_outcomes.len(), 1);
        assert!(!outcome.artifact_dir.join("spawned_after_abort").exists());
    }
}
