use std::{collections::HashMap, fs, io, path::PathBuf};

use chrono::{DateTime, Utc};
use lox_space::{
    analysis::{assets::AssetId, visibility::DynPass},
    frames::providers::DefaultRotationProvider,
    orbits::{
        events::{DetectFn, EventsToIntervals, IntervalDetector, RootFindingDetector},
        orbits::DynTrajectory,
        propagators::{OrbitSource, sgp4::Sgp4},
    },
    prelude::{GroundStation, Interval, Pass, Propagator, Spacecraft, TimeDelta},
    time::{
        Time,
        time_scales::{DynTimeScale, TimeScale},
    },
};
use tracing::{info, warn};

pub struct PredictDb {
    spacecraft: HashMap<String, Spacecraft>,
}

impl PredictDb {
    pub fn new() -> Self {
        Self {
            spacecraft: HashMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.spacecraft.len()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.spacecraft.contains_key(name)
    }

    pub fn add_tle(&mut self, text: &str) -> usize {
        sgp4::parse_3les(text)
            .inspect_err(|e| warn!(?e, "error parsing TLE file"))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|el| match Sgp4::new(el.clone()) {
                Ok(sgp4) => {
                    let name = el.object_name.unwrap_or(format!("ID {}", el.norad_id));
                    Some((name, OrbitSource::Sgp4(sgp4)))
                }
                Err(e) => {
                    warn!(?e, "error in TLE elements");
                    None
                }
            })
            .map(|(name, source)| {
                self.spacecraft
                    .insert(name.clone(), Spacecraft::new(name.clone(), source));
                info!(?name, "loaded spacecraft from TLE");
            })
            .count()
    }

    pub fn add_tles(&mut self, dir: &PathBuf) -> Result<usize, io::Error> {
        let mut added = 0;
        for entry in fs::read_dir(dir)?.filter_map(|e| e.ok()) {
            let path = entry.path();
            added += self.add_tle(&fs::read_to_string(path)?);
        }

        Ok(added)
    }

    pub fn predict_trajectories(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> HashMap<AssetId, DynTrajectory> {
        let interval = Interval::new(start.into(), end.into());

        self.spacecraft
            .values()
            .filter_map(|sc| {
                let name = sc.id().clone();
                match sc.orbit() {
                    OrbitSource::Sgp4(sgp4) => {
                        Some((name, sgp4.propagate(interval).unwrap().into_dyn()))
                    }
                    _ => {
                        warn!(?name, "unsupported orbit type");
                        None
                    }
                }
            })
            .collect()
    }

    pub fn predict_passes(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        gs: &GroundStation,
    ) -> HashMap<AssetId, Vec<DynPass>> {
        let interval = Interval::new(start.into(), end.into());

        self.predict_trajectories(start, end)
            .iter()
            .map(|(sc, trajectory)| {
                // Detect passes
                let detector = EventsToIntervals::new(RootFindingDetector::new(
                    SimpleElevationDetector { gs, trajectory },
                    TimeDelta::from_seconds(30),
                ));

                let passes: Vec<Pass<_>> = detector
                    .detect(interval)
                    .unwrap()
                    .into_iter()
                    .map(|pass_interval| {
                        Pass::from_interval(
                            //interval.to_scale(DynTimeScale),
                            Interval::new(
                                pass_interval.start().into_dyn(),
                                pass_interval.end().into_dyn(),
                            ),
                            TimeDelta::from_seconds(1),
                            gs.location(),
                            gs.mask(),
                            trajectory,
                            gs.body_fixed_frame(),
                        )
                        .unwrap()
                    })
                    .collect();

                (sc.clone(), passes)
            })
            .collect()
    }
}

struct SimpleElevationDetector<'a> {
    pub gs: &'a GroundStation,
    pub trajectory: &'a DynTrajectory,
}

#[derive(thiserror::Error, Debug)]
enum ElevationDetectError {}

impl<'a, T: TimeScale + Into<DynTimeScale>> DetectFn<T> for SimpleElevationDetector<'a> {
    type Error = ElevationDetectError;

    fn eval(&self, time: Time<T>) -> Result<f64, Self::Error> {
        let pos = self.trajectory.interpolate_at(time.into_dyn());
        let pos = pos
            .try_to_frame(self.gs.body_fixed_frame(), &DefaultRotationProvider)
            .unwrap();

        Ok(self
            .gs
            .location()
            .compute_observables(pos.position(), pos.velocity())
            .elevation())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::TimeZone;
    use lox_space::{
        analysis::visibility::ElevationMask,
        bodies::DynOrigin,
        core::coords::LonLatAlt,
        prelude::{GroundLocation, GroundStation},
    };

    fn tle_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/tle")
    }

    fn test_ground_station() -> GroundStation {
        let coords = LonLatAlt::from_degrees(13.4, 52.52, 100.0).unwrap();
        let location = GroundLocation::try_new(coords, DynOrigin::Earth).unwrap();
        let mask = ElevationMask::with_fixed_elevation(0.0);
        GroundStation::new("GS", location, mask)
    }

    #[test]
    fn add_tles_loads_propagators_from_directory() {
        let mut db = PredictDb::new();
        db.add_tles(&tle_dir()).unwrap();

        assert_eq!(db.len(), 7);
        assert!(db.contains("NanoFF B LEOP D-Orbit"));
        assert!(db.contains("NanoFF B SatNOGS"));
        assert!(db.contains("NanoFF B Space-Track"));
        assert!(db.contains("NanoFF A GNSS TLE SatNOGS"));
        assert!(db.contains("NanoFF A Space-Track"));
        assert!(db.contains("NanoFF A"));
        assert!(db.contains("NanoFF B"));
    }

    #[test]
    fn add_tles_returns_error_for_nonexistent_directory() {
        let mut db = PredictDb::new();
        let dir = PathBuf::from("/nonexistent/path");
        assert!(db.add_tles(&dir).is_err());
    }

    #[test]
    fn add_tles_on_empty_dir_loads_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let mut db = PredictDb::new();
        db.add_tles(&tmp.path().to_path_buf()).unwrap();
        assert_eq!(db.len(), 0);
    }

    #[test]
    fn predict_trajectories_returns_trajectories_for_all_spacecraft() {
        let mut db = PredictDb::new();
        db.add_tles(&tle_dir()).unwrap();

        let start = Utc.with_ymd_and_hms(2026, 1, 15, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 1, 15, 1, 0, 0).unwrap();

        let trajectories = db.predict_trajectories(start, end);
        assert_eq!(trajectories.len(), db.len());
    }

    #[test]
    fn predict_passes_returns_passes_for_loaded_tles() {
        let mut db = PredictDb::new();
        db.add_tles(&tle_dir()).unwrap();

        let gs = test_ground_station();
        let start = Utc.with_ymd_and_hms(2026, 1, 15, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap();

        let passes = db.predict_passes(start, end, &gs);
        assert!(!passes.is_empty());
    }

    #[test]
    fn predict_passes_observables_have_positive_elevation() {
        let mut db = PredictDb::new();
        db.add_tles(&tle_dir()).unwrap();

        let gs = test_ground_station();
        let start = Utc.with_ymd_and_hms(2026, 1, 15, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap();

        let passes = db.predict_passes(start, end, &gs);
        for (_id, sat_passes) in &passes {
            for pass in sat_passes {
                for obs in pass.observables() {
                    assert!(obs.elevation() >= 0.0,);
                }
            }
        }
    }
}
