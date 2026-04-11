use std::time::Duration;

use chrono::Utc;
use clap::Args;
use tokio::{sync::broadcast, time::sleep};
use tracing::info;

use crate::{
    config::Config,
    predict::PredictDb,
    tracker::utils::{Frequency, Output},
};

mod utils;

#[derive(Args)]
pub struct TrackerArgs {
    #[arg(long, name = "tx")]
    pub tx_freq: Option<Frequency>,
    #[arg(long, name = "rx")]
    pub rx_freq: Option<Frequency>,
    #[arg(short, default_value = "1.0")]
    pub update_rate: f32,
    #[arg(short, long)]
    pub out: Vec<Output>,
}

/// Runs the tracker loop until stopped.
pub async fn run(args: TrackerArgs, pdb: &PredictDb, config: &Config) {
    // For each output
    //  Create output task
    //
    let (exit_tx, mut exit_rx) = broadcast::channel(1);

    // Get the spacecraft we are tracking
    // and the GS
    let (name, sc) = pdb
        .first()
        .expect("no object loaded for tracking, this should not be possible");
    let gs = config
        .ground_station
        .clone()
        .expect("ground station not configured");

    loop {
        tokio::select! {
            _ = exit_rx.recv() => {
                info!("exit received, stopping");
                break;
            }

            _ = sleep(Duration::from_secs_f32(args.update_rate)) => {
                // Sleep completed
            }
        }

        // Compute observables at the current time for the GS
        let now = Utc::now();
        let state = pdb.state_at(now, sc).unwrap();
        let observables = gs.location().observables_dyn(state);

        info!(?name, ?observables);
    }

    let _ = exit_tx.send(());
    // While not stop
    //  Predict observables of satellite
    //  Calculate Doppler-shifted TX/RX frequencies (if given)
    //  Publish tracker events
}
