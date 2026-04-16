use std::{collections::HashMap, fs, io, path::PathBuf};

use chrono::{DateTime, Utc};
use lox_space::{
    analysis::{assets::AssetId, visibility::DynPass},
    bodies::DynOrigin,
    frames::{
        DynFrame,
        providers::DefaultRotationProvider,
        rotations::{DynRotationError, Rotation, TryRotation},
    },
    orbits::{
        events::{DetectFn, EventsToIntervals, IntervalDetector, RootFindingDetector},
        orbits::DynTrajectory,
        propagators::{
            OrbitSource,
            sgp4::{Sgp4, Sgp4Error},
        },
    },
    prelude::{
        Cartesian, GroundStation, Interval, Orbit, Pass, Propagator, Spacecraft, Tai, TimeDelta,
    },
    time::{
        Time,
        deltas::ToDelta,
        intervals::TimeInterval,
        time_scales::{DynTimeScale, TimeScale},
    },
};
use sgp4::Elements;
use tracing::{info, warn};

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
    ) -> HashMap<AssetId, DynTrajectory> {
        let interval = Interval::new(start.into(), end.into());

        self.spacecraft
            .values()
            .filter_map(|sc| match self.predict(interval, sc) {
                Ok(t) => Some((sc.id().clone(), t)),
                Err(_) => None,
            })
            .collect()
    }

    pub fn predict_passes(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
        gs: &GroundStation,
    ) -> HashMap<AssetId, Vec<DynPass>> {
        let tai_start: Time<Tai> = start.into();
        let tai_end: Time<Tai> = end.into();
        let interval = Interval::new(tai_start, tai_end);
        let target_frame = gs.body_fixed_frame();

        // Providers are keyed by trajectory frame via linear scan. In
        // practice all SGP4 trajectories share `DynFrame::Teme`, so we
        // build one provider per request rather than once per satellite.
        let mut providers: Vec<(DynFrame, CachedRotationProvider)> = Vec::new();

        self.predict_trajectories(start, end)
            .iter()
            .map(|(sc, trajectory)| {
                let from = trajectory.reference_frame();
                if !providers.iter().any(|(f, _)| *f == from) {
                    providers.push((
                        from,
                        CachedRotationProvider::build(
                            tai_start,
                            tai_end,
                            30.0,
                            from,
                            target_frame,
                        ),
                    ));
                }
                let provider = &providers.iter().find(|(f, _)| *f == from).unwrap().1;

                let detector = EventsToIntervals::new(RootFindingDetector::new(
                    SimpleElevationDetector {
                        gs,
                        trajectory,
                        provider,
                    },
                    TimeDelta::from_seconds(60),
                ));

                let passes: Vec<Pass<_>> = detector
                    .detect(interval)
                    .unwrap()
                    .into_iter()
                    .map(|pass_interval| {
                        sample_pass(pass_interval, 20, gs, trajectory, provider).unwrap()
                    })
                    .collect();

                (sc.clone(), passes)
            })
            .collect()
    }
}

/// A [`TryRotation`] provider that precomputes a rotation between two
/// frames on a regular time grid and serves linearly interpolated lookups.
///
/// Earth rotation varies smoothly at ~15"/s, so a 30-second grid with
/// linear interpolation yields angular error <<1" — well below pass
/// geometry tolerances — while skipping the IAU 1980 nutation evaluation
/// on every sample.
///
/// Because this implements [`TryRotation<DynFrame, DynFrame, DynTimeScale>`],
/// it can be passed directly to [`CartesianOrbit::try_to_frame`] as a
/// drop-in replacement for [`DefaultRotationProvider`].
pub struct CachedRotationProvider {
    start_delta: TimeDelta,
    step_seconds: f64,
    from: DynFrame,
    to: DynFrame,
    rotations: Vec<Rotation>,
}

impl CachedRotationProvider {
    pub fn build(
        start: Time<Tai>,
        end: Time<Tai>,
        step_seconds: f64,
        from: DynFrame,
        to: DynFrame,
    ) -> Self {
        let total = time_delta_seconds(end - start);
        let n = (total / step_seconds).ceil() as usize + 2;
        let rotations: Vec<Rotation> = (0..n)
            .map(|i| {
                let t = start + TimeDelta::from_seconds_f64(i as f64 * step_seconds);
                DefaultRotationProvider.try_rotation(from, to, t).unwrap()
            })
            .collect();
        Self {
            start_delta: start.to_delta(),
            step_seconds,
            from,
            to,
            rotations,
        }
    }

    fn get(&self, delta: TimeDelta) -> Rotation {
        let dt = time_delta_seconds(delta - self.start_delta);
        let f = dt / self.step_seconds;
        let last = self.rotations.len() - 1;
        let i = (f.floor() as isize).clamp(0, last as isize - 1) as usize;
        let alpha = (f - i as f64).clamp(0.0, 1.0);
        lerp_rotation(&self.rotations[i], &self.rotations[i + 1], alpha)
    }
}

impl TryRotation<DynFrame, DynFrame, DynTimeScale> for CachedRotationProvider {
    type Error = DynRotationError;

    fn try_rotation(
        &self,
        origin: DynFrame,
        target: DynFrame,
        time: Time<DynTimeScale>,
    ) -> Result<Rotation, Self::Error> {
        if origin == target {
            return Ok(Rotation::IDENTITY);
        }
        let delta = time.to_delta();
        if origin == self.from && target == self.to {
            Ok(self.get(delta))
        } else if origin == self.to && target == self.from {
            Ok(self.get(delta).transpose())
        } else {
            DefaultRotationProvider.try_rotation(origin, target, time)
        }
    }
}

fn time_delta_seconds(td: TimeDelta) -> f64 {
    td.seconds().unwrap_or(0) as f64 + td.subsecond().unwrap_or(0.0)
}

fn lerp_rotation(a: &Rotation, b: &Rotation, t: f64) -> Rotation {
    let s = 1.0 - t;
    Rotation {
        m: a.m * s + b.m * t,
        dm: a.dm * s + b.dm * t,
    }
}

/// Equivalent to [`DynPass::from_interval`] but accepts a custom
/// rotation provider. This avoids the IAU 1980 nutation evaluation on
/// every sample by using [`CachedRotationProvider`].
///
/// When upstream adds a provider parameter to `Pass::from_interval`,
/// this function can be replaced by a direct call.
fn sample_pass(
    pass_interval: TimeInterval<Tai>,
    step_seconds: i64,
    gs: &GroundStation,
    trajectory: &DynTrajectory,
    provider: &CachedRotationProvider,
) -> Option<DynPass> {
    let step = TimeDelta::from_seconds(step_seconds);
    let body_fixed_frame = gs.body_fixed_frame();
    let mut times = Vec::new();
    let mut observables = Vec::new();

    for t in pass_interval.step_by(step) {
        let state = trajectory.interpolate_at(t.into_dyn());
        let state_bf = state.try_to_frame(body_fixed_frame, provider).unwrap();
        let obs = gs.location().observables_dyn(state_bf);

        let min_elev = gs.mask().min_elevation(obs.azimuth());
        if obs.elevation() >= min_elev {
            times.push(t.into_dyn());
            observables.push(obs);
        }
    }

    if times.is_empty() {
        return None;
    }

    let interval_dyn = Interval::new(
        pass_interval.start().into_dyn(),
        pass_interval.end().into_dyn(),
    );
    Pass::try_new(interval_dyn, times, observables).ok()
}

struct SimpleElevationDetector<'a> {
    pub gs: &'a GroundStation,
    pub trajectory: &'a DynTrajectory,
    pub provider: &'a CachedRotationProvider,
}

#[derive(thiserror::Error, Debug)]
enum ElevationDetectError {}

impl<'a, T: TimeScale + Into<DynTimeScale>> DetectFn<T> for SimpleElevationDetector<'a> {
    type Error = ElevationDetectError;

    fn eval(&self, time: Time<T>) -> Result<f64, Self::Error> {
        let state = self.trajectory.interpolate_at(time.into_dyn());
        let state_bf = state
            .try_to_frame(self.gs.body_fixed_frame(), self.provider)
            .unwrap();

        Ok(self
            .gs
            .location()
            .compute_observables(state_bf.position(), state_bf.velocity())
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
