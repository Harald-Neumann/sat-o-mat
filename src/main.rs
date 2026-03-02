use std::fs;
use std::path::PathBuf;

use sat_o_mat::task::format::Task;
use sat_o_mat::task::runner::{RunConfig, run};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1] == "runner" {
        if let Err(err) = run_runner(&args[2]).await {
            eprintln!("runner error: {err}");
            std::process::exit(1);
        }
        return;
    }

    eprintln!("usage: sat-o-mat runner <task.yaml>");
    std::process::exit(2);
}

async fn run_runner(task_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let yaml = fs::read_to_string(task_path)?;
    let task = Task::from_yaml_str(&yaml)?;
    let config = RunConfig {
        artifact_base: PathBuf::from("artifacts"),
    };
    let outcome = run(task, config).await?;

    println!("aborted: {}", outcome.aborted());
    println!("artifact_dir: {}", outcome.artifact_dir.display());
    println!("steps: {}", outcome.step_outcomes.len());
    println!("outcomes: {:?}", outcome.step_outcomes);

    Ok(())
}
