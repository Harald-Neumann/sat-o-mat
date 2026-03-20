use std::fs;
use std::path::PathBuf;

mod config;
mod scheduler;
mod task;

use clap::{Parser, Subcommand};
use tracing::info;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

use crate::task::format::Task;
use crate::task::runner::{RunConfig, run};

#[derive(Parser)]
#[command(name = "sat-o-mat")]
#[command(about = "An application to control satellite ground station hardware")]
struct Args {
    /// The config file. Defaults to $XDG_CONFIG_HOME/sat-o-mat/config.yaml
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run a Task
    Run {
        /// The task definition file
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .init();

    let config = config::load(args.config.as_ref())?;
    info!(?config);

    match args.command {
        Commands::Run { file } => {
            println!("running {:?}", file);
        }
    }

    Ok(())
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
