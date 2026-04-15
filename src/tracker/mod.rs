use std::time::Duration;

use chrono::Utc;
use clap::Args;
use lox_space::{frames::providers::DefaultRotationProvider, units::SPEED_OF_LIGHT};
use tokio::{sync::broadcast, time::sleep};
use tracing::info;

use crate::{
    config::Config,
    predict::PredictDb,
    tracker::{
        update::Update,
        utils::{Frequency, Output},
    },
};

mod rotctl;
mod update;
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
    let (exit_tx, mut exit_rx) = broadcast::channel(1);
    let (update_tx, _) = broadcast::channel(1);

    for out in args.out.into_iter() {
        match out {
            Output::Rotctl(addr) => {
                tokio::spawn(rotctl::run(addr, update_tx.subscribe()));
            }
            _ => todo!(),
        }
    }

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
        let state_body_frame = state
            .try_to_frame(gs.body_fixed_frame(), &DefaultRotationProvider)
            .unwrap();
        let observables = gs.location().observables_dyn(state_body_frame);

        // Compute Doppler corrected frequencies if present
        let range_rate = observables.range_rate();
        let tx_frequency_hertz = doppler_correct(args.tx_freq, range_rate, true);
        let rx_frequency_hertz = doppler_correct(args.rx_freq, range_rate, false);

        // Create and send tracker update
        let update = Update {
            timestamp: now,
            azimuth_degrees: observables.azimuth().to_degrees(),
            elevation_degrees: observables.elevation().to_degrees(),
            range_meters: observables.range(),
            range_rate_meters_per_second: observables.range_rate(),
            tx_frequency_hertz,
            rx_frequency_hertz,
        };
        info!(
            ?name,
            ?range_rate,
            range = ?observables.range(),
            "az={:.2} el={:.2}",
            update.azimuth_degrees,
            update.elevation_degrees
        );
        let _ = update_tx.send(update);
    }

    let _ = exit_tx.send(());
}

fn doppler_correct(
    base_freq: Option<Frequency>,
    range_rate_meters_per_second: f64,
    is_uplink: bool,
) -> Option<u64> {
    base_freq.map(|Frequency(freq)| {
        let carrier_lambda = SPEED_OF_LIGHT / (freq as f64);
        let f_shift =
            if is_uplink { 1.0 } else { -1.0 } * range_rate_meters_per_second / carrier_lambda;
        freq.saturating_add_signed(f_shift as i64)
    })
}
