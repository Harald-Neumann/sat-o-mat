use std::fs;
use std::io::Read;
use std::path::PathBuf;

mod api;
mod config;
mod frontend;
mod predict;
mod scheduler;
mod server;
mod task;
mod tracker;

use clap::{Parser, Subcommand};
use tracing::level_filters::LevelFilter;
use tracing::{error, info};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, fmt};

use crate::predict::PredictDb;
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

    Server {
        host: String,
        port: u32,
    },

    /// Tracks an object in space and publishes information about the observables
    /// (azimuth, elevation, range and range rate) relative to the ground location
    /// specified in the configuration.
    ///
    /// If TX/RX frequencies are given, Doppler corrected frequencies are also calculated
    /// and published.
    ///
    /// Reads orbit information from STDIN in any of the supported formats ({3,T}LE, CCSDS OMM).
    Tracker(tracker::TrackerArgs),
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
        Commands::Server { host, port } => {
            server::run(config, host, port).await?;
        }
        Commands::Tracker(args) => {
            // Read orbit info (TLE, OMM, ...) from stdin
            let mut orbit_info = String::new();
            let mut stdin = std::io::stdin();
            info!("waiting for orbit info from stdin");
            stdin.read_to_string(&mut orbit_info).unwrap();

            // Create PredictDb
            let mut pdb = PredictDb::new();
            match pdb.add(&orbit_info) {
                0 => {
                    todo!();
                }
                1 => {
                    let (object, _) = pdb.first().unwrap();
                    info!(?object, "loaded one orbit");
                }
                2.. => {
                    error!("more than one orbit loaded, please specify which object to track");
                    return Ok(());
                }
            }

            // Run tracker
            tracker::run(args, &pdb, &config).await;
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
