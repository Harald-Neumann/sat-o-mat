use std::path::PathBuf;

use chrono::{TimeZone, Utc};
use std::time::Duration;

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use lox_space::{
    analysis::visibility::ElevationMask,
    bodies::DynOrigin,
    core::coords::LonLatAlt,
    prelude::{GroundLocation, GroundStation},
};
use sat_o_mat::predict::PredictDb;

fn ground_station() -> GroundStation {
    let coords = LonLatAlt::from_degrees(13.4, 52.52, 100.0).unwrap();
    let location = GroundLocation::try_new(coords, DynOrigin::Earth).unwrap();
    let mask = ElevationMask::with_fixed_elevation(0.0);
    GroundStation::new("GS", location, mask)
}

fn bench_predict_passes(c: &mut Criterion) {
    let tle = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/tle/nanoff.txt"),
    )
    .expect("failed to read nanoff.txt");

    let mut db = PredictDb::new();
    assert!(db.add_tle(&tle) == 5);

    let gs = ground_station();
    let start = Utc.with_ymd_and_hms(2026, 1, 15, 0, 0, 0).unwrap();
    let end = Utc.with_ymd_and_hms(2026, 1, 16, 0, 0, 0).unwrap();

    c.bench_function("predict_passes_nanoff_1day", |b| {
        b.iter(|| db.predict_passes(black_box(start), black_box(end), black_box(&gs)))
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default().sample_size(10);
    targets = bench_predict_passes
}
criterion_main!(benches);
