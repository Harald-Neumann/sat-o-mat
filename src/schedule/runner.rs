use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use chrono::{DateTime, Utc};
use tokio::process::Command;
use tokio::spawn;
use tokio::sync::broadcast;
use tokio::sync::broadcast::Receiver;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::time::{Duration, sleep};
use tracing::{info, warn};

use crate::schedule::parser::{OnFail, Schedule, Step, TimeSpec};
use crate::schedule::utils::{resolve_time, resolve_variables, substitute_variables};

// --- Error ---

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to create artifact directory: {0}")]
    ArtifactDir(std::io::Error),
    #[error("IO error during variable resolution: {0}")]
    VariableResolveIo(std::io::Error),
    #[error("Invalid variable in time spec")]
    InvalidVariableInTimeSpec,
    #[error("Invalid time spec")]
    InvalidTimeSpec(serde_yaml::Error),
}

// --- Public types ---

pub struct RunConfig {
    pub artifact_base: PathBuf,
}

#[derive(Debug)]
pub struct RunOutcome {
    pub artifact_dir: PathBuf,
    pub step_outcomes: Vec<StepOutcome>,
    pub aborted: bool,
    pub cleanup_error: Option<String>,
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
    SpawnError(String),
    ExitSignalReceived,
}

// --- Public API ---

pub async fn run(schedule: Schedule, config: RunConfig) -> Result<RunOutcome, Error> {
    // Create artifact directory
    let dir_name = Utc::now().format("%Y%m%d_%H%M%S").to_string();
    let artifact_dir = config.artifact_base.join(&dir_name);
    tokio::fs::create_dir_all(&artifact_dir)
        .await
        .map_err(Error::ArtifactDir)?;

    info!(dir = %artifact_dir.display(), "created artifact directory");

    // Resolve variables (evaluate ${...} shell commands)
    let mut vars = schedule.variables;
    resolve_variables(&mut vars, &artifact_dir)
        .await
        .map_err(Error::VariableResolveIo)?;

    // Resolve start/end timestamps
    vars.entry("start".into())
        .or_insert_with(|| Utc::now().to_rfc3339());

    let start_time = DateTime::parse_from_rfc3339(&vars["start"])
        .expect("start variable must be a valid RFC 3339 timestamp")
        .with_timezone(&Utc);

    let end_time = match vars.get("end") {
        Some(v) => Some(
            serde_yaml::from_str::<TimeSpec>(v)
                .map(|time_spec| resolve_time(&time_spec, &vars))
                .map_err(Error::InvalidTimeSpec)?
                .ok_or(Error::InvalidVariableInTimeSpec)?,
        ),
        None => None,
    };
    //serde_yaml::from_str(&vars["end"])

    // If start is in the future, wait
    sleep_until(start_time).await;

    // Run main steps with end-time deadline, then kill
    let step_outcomes = run_steps(schedule.steps, &vars, &artifact_dir, end_time).await;

    // Cleanup, then kill
    let cleanup_outcomes = run_steps(schedule.cleanup, &vars, &artifact_dir, None).await;

    let aborted = step_outcomes
        .iter()
        .any(|o| matches!(o, StepOutcome::Abort { .. }));
    let cleanup_aborted = cleanup_outcomes
        .iter()
        .any(|o| matches!(o, StepOutcome::Abort { .. }));

    Ok(RunOutcome {
        artifact_dir,
        step_outcomes,
        aborted,
        cleanup_error: cleanup_aborted.then(|| "cleanup step failed".into()),
    })
}

async fn run_steps(
    steps: Vec<Step>,
    vars: &HashMap<String, String>,
    cwd: &Path,
    end_time: Option<DateTime<Utc>>,
) -> Vec<StepOutcome> {
    let mut outcomes = Vec::new();
    let (outcome_tx, mut outcome_rx) = mpsc::unbounded_channel();
    let (exit_tx, mut exit_rx) = broadcast::channel(1);

    info!(?steps, ?end_time);

    // Spawner task
    let spawner = {
        let exit_tx = exit_tx.clone();
        let vars = vars.clone();
        let cwd = cwd.to_path_buf();

        // Returns a future which resolves to a list of still-running JoinHandles
        // for steps that have not completed yet.
        spawn(async move {
            let mut handles = Vec::new();
            for step in steps {
                // If step.time is set, resolve and sleep until it
                let step_start = if let Some(ref time_spec) = step.time {
                    resolve_time(time_spec, &vars)
                } else {
                    None
                };

                // Wait until the step start time is reached (if configured),
                // while checking if the exit signal has been sent.
                let should_execute_step =
                    wait_for_step_start_or_abort(step_start, &mut exit_rx).await;
                if !should_execute_step {
                    break;
                }

                // Substitute variables in step.cmd
                let cmd = substitute_variables(&step.cmd, &vars);
                info!(cmd = %cmd, wait = step.wait, "executing step");

                let abort_on_fail = !matches!(&step.on_fail, OnFail::Continue);
                let max_attempts = match &step.on_fail {
                    OnFail::Retry(n) => *n,
                    _ => 1,
                };

                // Spawn the command for this step, with retries
                let step_handle = spawn(run_step(
                    cmd.clone(),
                    abort_on_fail,
                    max_attempts,
                    cwd.to_path_buf(),
                    exit_tx.subscribe(),
                    outcome_tx.clone(),
                ));

                if step.wait {
                    // Wait for the current step to finish executing
                    let did_abort = step_handle.await.unwrap();
                    if did_abort {
                        break;
                    }
                } else {
                    // Add it to the list of still-running handles
                    handles.push(step_handle);
                }
            }
            handles
        })
    };

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
                    info!("all senders exited, breaking");
                    break;
                }
                let outcome = outcome.unwrap();
                outcomes.push(outcome.clone());

                if let StepOutcome::Abort { cmd, reason } = outcome {
                    warn!(?cmd, ?reason, "aborted, sending exit signal");
                    let _ = exit_tx.send(());
                } else {
                    info!(outcome = ?outcome, "got outcome");
                }
            }
        }
    }

    // Wait until all tasks and childs have exited
    let handles = spawner.await.unwrap();
    for h in handles {
        h.await.unwrap();
    }

    // Return collected StepOutcomes
    outcomes
}

async fn run_step(
    cmd: String,
    abort_on_fail: bool,
    max_attempts: u32,
    cwd: PathBuf,
    mut exit_rx: Receiver<()>,
    tx: UnboundedSender<StepOutcome>,
) -> bool {
    let mut outcome: Option<StepOutcome> = None;

    for _i in 1..=max_attempts {
        // Try to spawn a child process for `cmd`
        let mut child = match spawn_command(&cmd, &cwd) {
            Ok(child) => {
                info!(child = ?child, cmd = cmd, "spawned child");
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

        // Wait for child to finish running or the exit signal
        tokio::select! {
            exit = child.wait() => {
                match exit {
                    Ok(status) => {
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
                        outcome = Some(StepOutcome::SpawnError {
                            cmd: cmd.clone(),
                            error: e.to_string()
                        });
                    }
                }
            }

            // Exit signal (abort or deadline)
            _ = exit_rx.recv() => {
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

    if let Some(StepOutcome::Completed { ref cmd, status }) = outcome
        && !status.success()
        && abort_on_fail
    {
        // If the last outcome was an unsuccessful exit and `abort_on_fail` is set,
        // change the outcome to aborted
        outcome = Some(StepOutcome::Abort {
            cmd: cmd.clone(),
            reason: AbortReason::ExitStatus(status),
        });
    };

    let aborted = matches!(outcome, Some(StepOutcome::Abort { .. }));

    // Send step outcome to monitor loop
    let _ = tx.send(outcome.unwrap());

    aborted
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
    use crate::schedule::parser::{OnFail, Schedule, Step, TimeSpec};
    use chrono::TimeDelta;
    use tracing_test::traced_test;

    // --- Integration tests ---

    fn make_schedule(steps: Vec<Step>, cleanup: Vec<Step>) -> Schedule {
        Schedule {
            //variables: HashMap::from([("end".into(), "T+3 seconds".into())]),
            variables: HashMap::new(),
            steps,
            cleanup,
        }
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

    async fn run_with_tempdir(schedule: Schedule) -> RunOutcome {
        let temp = tempfile::tempdir().expect("failed to create temp dir");
        let config = RunConfig {
            artifact_base: temp.path().to_path_buf(),
        };
        run(schedule, config).await.expect("run should succeed")
    }

    #[tokio::test]
    async fn simple_echo_step() {
        let schedule = make_schedule(vec![waited("echo hello")], vec![]);
        let outcome = run_with_tempdir(schedule).await;
        assert!(!outcome.aborted);
        assert_eq!(outcome.step_outcomes.len(), 1);
        assert!(matches!(
            &outcome.step_outcomes[0],
            StepOutcome::Completed { cmd, status } if cmd == "echo hello" && status.success()
        ));
    }

    #[tokio::test]
    #[traced_test]
    async fn wait_true_blocks_until_complete() {
        // Use a command that takes a moment but succeeds
        let schedule = make_schedule(vec![waited("sleep 0.1 && echo done")], vec![]);
        let outcome = run_with_tempdir(schedule).await;
        assert!(!outcome.aborted);
        assert!(matches!(
            &outcome.step_outcomes[0],
            StepOutcome::Completed { status, .. } if status.success()
        ));
    }

    #[tokio::test]
    #[traced_test]
    async fn on_fail_abort_stops_execution() {
        let schedule = make_schedule(vec![waited("false"), waited("echo should not run")], vec![]);
        let outcome = run_with_tempdir(schedule).await;
        assert!(outcome.aborted);
        // Only one step outcome (the failed one); second step was skipped
        assert_eq!(outcome.step_outcomes.len(), 1);
        assert!(matches!(
            &outcome.step_outcomes[0],
            StepOutcome::Abort { reason: AbortReason::ExitStatus(status), .. } if !status.success()
        ));
    }

    #[tokio::test]
    #[traced_test]
    async fn on_fail_continue_proceeds() {
        let schedule = make_schedule(
            vec![waited_continue("false"), waited("echo still running")],
            vec![],
        );
        let outcome = run_with_tempdir(schedule).await;
        assert!(!outcome.aborted);
        assert_eq!(outcome.step_outcomes.len(), 2);
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
    #[traced_test]
    async fn on_fail_retry_succeeds_eventually() {
        // This command fails the first time (file doesn't exist),
        // creates the file, then succeeds on retry.
        let schedule = Schedule {
            variables: HashMap::new(),
            steps: vec![waited_retry(
                "test -f attempt_marker || { touch attempt_marker; false; }",
                2,
            )],
            cleanup: vec![waited("rm -f attempt_marker")],
        };

        let outcome = run_with_tempdir(schedule).await;
        assert!(!outcome.aborted);
        assert_eq!(outcome.step_outcomes.len(), 1);
        assert!(matches!(
            &outcome.step_outcomes[0],
            StepOutcome::Completed { status, .. } if status.success()
        ));
    }

    #[tokio::test]
    async fn cleanup_always_runs_after_abort() {
        let schedule = make_schedule(vec![waited("false")], vec![waited("touch cleanup_ran")]);

        let temp = std::env::temp_dir().join(format!("sat-o-mat-cleanup-{}", std::process::id()));
        let config = RunConfig {
            artifact_base: temp.clone(),
        };
        let outcome = run(schedule, config).await.expect("run should succeed");
        assert!(outcome.aborted);
        // Cleanup should have run -- check for the file in the artifact dir
        assert!(outcome.artifact_dir.join("cleanup_ran").exists());

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[tokio::test]
    #[traced_test]
    async fn end_time_deadline_kills_long_step() {
        // End time 1 second from now; step sleeps 60 seconds
        let end = Utc::now() + TimeDelta::seconds(1);
        let schedule = Schedule {
            variables: HashMap::from([("end".into(), end.to_rfc3339())]),
            steps: vec![waited("sleep 60")],
            cleanup: vec![],
        };

        let start = std::time::Instant::now();
        let outcome = run_with_tempdir(schedule).await;
        let elapsed = start.elapsed();

        assert!(outcome.aborted);
        // Should complete in ~1s + 3s grace period, not 60s
        assert!(elapsed.as_secs() < 10);
    }

    #[tokio::test]
    #[traced_test]
    async fn shell_variable_resolution() {
        let schedule = Schedule {
            variables: HashMap::from([("greeting".into(), "${echo hello}".into())]),
            steps: vec![waited("echo $greeting")],
            cleanup: vec![],
        };
        let outcome = run_with_tempdir(schedule).await;
        assert!(!outcome.aborted);
        assert_eq!(outcome.step_outcomes.len(), 1);
    }

    #[tokio::test]
    async fn artifact_directory_is_created() {
        let schedule = make_schedule(vec![waited("pwd")], vec![]);

        let temp = std::env::temp_dir().join(format!("sat-o-mat-artifact-{}", std::process::id()));
        let config = RunConfig {
            artifact_base: temp.clone(),
        };
        let outcome = run(schedule, config).await.expect("run should succeed");
        assert!(outcome.artifact_dir.exists());
        assert!(outcome.artifact_dir.starts_with(&temp));

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[tokio::test]
    async fn background_step_returns_spawned() {
        let schedule = make_schedule(vec![step("sleep 0.1")], vec![]);
        let outcome = run_with_tempdir(schedule).await;
        assert!(!outcome.aborted);
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
    #[traced_test]
    async fn background_abort_on_fail_stops_execution() {
        // Background step exits immediately with failure; the next step is a
        // long wait that should be interrupted by the background failure.
        let schedule = make_schedule(vec![background_abort("false"), waited("sleep 60")], vec![]);

        let start = std::time::Instant::now();
        let outcome = run_with_tempdir(schedule).await;
        let elapsed = start.elapsed();

        assert!(outcome.aborted);
        // Should abort quickly, not wait 60s
        assert!(elapsed.as_secs() < 10);
    }

    #[tokio::test]
    async fn background_continue_on_fail_does_not_abort() {
        // Background step fails with on_fail: continue — execution should proceed.
        let schedule = make_schedule(
            vec![background_continue("false"), waited("echo still here")],
            vec![],
        );
        let outcome = run_with_tempdir(schedule).await;
        assert!(!outcome.aborted);
    }

    #[tokio::test]
    #[traced_test]
    async fn aborting_background_step_prevents_future_timed_step_spawn() {
        let start = Utc::now();
        let schedule = Schedule {
            variables: HashMap::from([("start".into(), start.to_rfc3339())]),
            steps: vec![
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
            cleanup: vec![],
        };

        let outcome = run_with_tempdir(schedule).await;
        assert!(outcome.aborted);
        assert_eq!(outcome.step_outcomes.len(), 1);
        assert!(!outcome.artifact_dir.join("spawned_after_abort").exists());
    }
}
