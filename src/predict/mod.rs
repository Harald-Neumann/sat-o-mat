use std::{collections::HashMap, fs, io, path::PathBuf};

use chrono::{DateTime, Utc};
use lox_space::{
    analysis::{
        assets::AssetId,
        visibility::{DynPass, ElevationMask},
    },
    bodies::DynOrigin,
    core::coords::LonLatAlt,
    frames::DynFrame,
    orbits::{
        events::{EventsToIntervals, IntervalDetector, RootFindingDetector},
        orbits::DynTrajectory,
        propagators::{
            OrbitSource,
            sgp4::{Sgp4, Sgp4Error},
        },
    },
    prelude::{
        Cartesian, GroundStation, Interval, Orbit, Pass, Propagator, Spacecraft, Tai, TimeDelta,
    },
    time::{Time, intervals::TimeInterval, time_scales::DynTimeScale},
};
use sgp4::Elements;
use tracing::{info, warn};

use utils::{CachedRotationProvider, SimpleElevationDetector};

mod utils;

pub struct PredictDb {
    spacecraft: HashMap<String, Spacecraft>,
}

#[derive(thiserror::Error, Clone, Debug)]
pub enum Error {
    #[error("unsupported orbit type {0}")]
    UnsupportedOrbitSource(String),
    #[error("SGP4 error: {0}")]
    Sgp4(String),
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

    pub fn first(&self) -> Option<(&String, &Spacecraft)> {
        self.spacecraft.iter().next()
    }

    fn add_from_elements(&mut self, el: &Elements) -> Result<(), Sgp4Error> {
        let sgp4 = Sgp4::new(el.clone())?;
        let source = OrbitSource::Sgp4(sgp4);
        let name = el
            .object_name
            .clone()
            .unwrap_or(format!("ID {}", el.norad_id));

        info!(?name, "loaded spacecraft (SGP4)");
        self.spacecraft
            .insert(name.clone(), Spacecraft::new(name.clone(), source));

        Ok(())
    }

    pub fn add_tle(&mut self, text: &str) -> usize {
        sgp4::parse_3les(text)
            .inspect_err(|e| warn!(?e, "error parsing TLE file"))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|el| match self.add_from_elements(&el) {
                Ok(_) => Some(()),
                Err(e) => {
                    warn!(?e, "error in elements");
                    None
                }
            })
            .count()
    }

    pub fn add_omm(&mut self, omm: &str) -> usize {
        match serde_json::from_str(omm) {
            Ok(el) => {
                if let Err(e) = self.add_from_elements(&el) {
                    warn!(?e, "error in elements");
                    0
                } else {
                    1
                }
            }
            Err(e) => {
                warn!(?e, "error parsing CCSDS OMM");
                0
            }
        }
    }

    pub fn add_tles(&mut self, dir: &PathBuf) -> Result<usize, io::Error> {
        let mut added = 0;
        for entry in fs::read_dir(dir)?.filter_map(|e| e.ok()) {
            let path = entry.path();
            added += self.add_tle(&fs::read_to_string(path)?);
        }

        Ok(added)
    }

    pub fn add(&mut self, info: &str) -> usize {
        let mut added = 0;
        added += self.add_tle(info);
        added += self.add_omm(info);

        added
    }

    pub fn state_at(
        &self,
        time: DateTime<Utc>,
        sc: &Spacecraft,
    ) -> Result<Orbit<Cartesian, DynTimeScale, DynOrigin, DynFrame>, Error> {
        let name = sc.id().clone().to_string();
        match sc.orbit() {
            OrbitSource::Sgp4(sgp4) => Ok(sgp4
                .state_at(time.into())
                .map_err(|e| Error::Sgp4(e.to_string()))?
                .into_dyn()),
            _ => {
                warn!(?name, "unsupported orbit type");
                Err(Error::UnsupportedOrbitSource(name))
            }
        }
    }

    pub fn predict(
        &self,
        interval: TimeInterval<Tai>,
        sc: &Spacecraft,
    ) -> Result<DynTrajectory, Error> {
        let name = sc.id().clone().to_string();
        match sc.orbit() {
            OrbitSource::Sgp4(sgp4) => Ok(sgp4
                .propagate(interval)
                .map_err(|e| Error::Sgp4(e.to_string()))?
                .into_dyn()),
            _ => {
                warn!(?name, "unsupported orbit type");
                Err(Error::UnsupportedOrbitSource(name))
            }
        }
    }

    pub fn predict_trajectories(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        target_frame: DynFrame,
        provider: Option<&mut CachedRotationProvider>,
    ) -> HashMap<AssetId, DynTrajectory> {
        let mut default_provider = CachedRotationProvider::new();
        let provider = provider.unwrap_or(&mut default_provider);

        let interval = Interval::new(start.into(), end.into());

        self.spacecraft
            .values()
            .filter_map(|sc| match self.predict(interval, sc) {
                Ok(trajectory) => {
                    // Valid trajectory

                    // Create rotation data cache (if it does not exist)
                    provider.ensure_cached_rotation_data(
                        trajectory.reference_frame(),
                        target_frame,
                        Interval::new(trajectory.start_time(), trajectory.end_time()),
                    );

                    // Transform trajectory to requested frame using cache
                    let t = trajectory.into_frame(target_frame, provider).unwrap();

                    Some((sc.id().clone(), t))
                }
                Err(_) => None,
            })
            .collect()
    }

    pub fn predict_passes(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        gs: &GroundStation,
        provider: Option<&mut CachedRotationProvider>,
    ) -> HashMap<AssetId, Vec<DynPass>> {
        let tai_start: Time<Tai> = start.into();
        let tai_end: Time<Tai> = end.into();
        let interval = Interval::new(tai_start, tai_end);
        let frame = gs.body_fixed_frame();

        self.predict_trajectories(start, end, frame, provider)
            .iter()
            .map(|(sc, trajectory)| {
                let detector = EventsToIntervals::new(RootFindingDetector::new(
                    SimpleElevationDetector { gs, trajectory },
                    TimeDelta::from_seconds(60),
                ));

                let passes: Vec<Pass<_>> = detector
                    .detect(interval)
                    .unwrap()
                    .into_iter()
                    .filter_map(|pass_interval| {
                        DynPass::from_interval(
                            // todo: pass_interval.into_dyn()
                            Interval::new(
                                pass_interval.start().into_dyn(),
                                pass_interval.end().into_dyn(),
                            ),
                            TimeDelta::from_seconds(20),
                            gs.location(),
                            &ElevationMask::with_fixed_elevation(0.0),
                            trajectory,
                            gs.body_fixed_frame(),
                        )
                    })
                    .collect();

                (sc.clone(), passes)
            })
            .collect()
    }

    pub fn predict_ground_track(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        provider: Option<&mut CachedRotationProvider>,
    ) -> HashMap<AssetId, Vec<(Time<Tai>, LonLatAlt)>> {
        let frame = DynFrame::Iau(DynOrigin::Earth);

        self.predict_trajectories(start, end, frame, provider)
            .iter()
            .map(|(sc, trajectory)| {
                (
                    sc.clone(),
                    trajectory
                        .times()
                        .into_iter()
                        .zip(trajectory.states())
                        .filter_map(|(t, state)| {
                            state
                                .try_to_ground_location()
                                .map(|gl| (t.with_scale(Tai), gl.coordinates()))
                                .ok()
                        })
                        .collect(),
                )
            })
            .collect()
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

        let trajectories = db.predict_trajectories(start, end, DynFrame::J2000, None);
        assert_eq!(trajectories.len(), db.len());
    }

    #[test]
    fn predict_passes_returns_passes_for_loaded_tles() {
        let mut db = PredictDb::new();
        db.add_tles(&tle_dir()).unwrap();

        let gs = test_ground_station();
        let start = Utc.with_ymd_and_hms(2026, 1, 15, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap();

        let passes = db.predict_passes(start, end, &gs, None);
        assert!(!passes.is_empty());
    }

    #[test]
    fn predict_passes_observables_have_positive_elevation() {
        let mut db = PredictDb::new();
        db.add_tles(&tle_dir()).unwrap();

        let gs = test_ground_station();
        let start = Utc.with_ymd_and_hms(2026, 1, 15, 0, 0, 0).unwrap();
        let end = Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap();

        let passes = db.predict_passes(start, end, &gs, None);
        for (_id, sat_passes) in &passes {
            for pass in sat_passes {
                for obs in pass.observables() {
                    assert!(obs.elevation() >= 0.0,);
                }
            }
        }
    }
}
