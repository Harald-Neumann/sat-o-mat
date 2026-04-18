use lox_space::{
    frames::{
        DynFrame,
        providers::DefaultRotationProvider,
        rotations::{DynRotationError, Rotation, TryRotation},
    },
    orbits::{events::DetectFn, orbits::DynTrajectory},
    prelude::{GroundStation, Interval, Tai, TimeDelta},
    time::{
        Time,
        deltas::ToDelta,
        time_scales::{DynTimeScale, TimeScale},
    },
};

struct CachedRotationData {
    start: Time<Tai>,
    end: Time<Tai>,
    start_delta: TimeDelta,
    step_seconds: f64,
    from: DynFrame,
    to: DynFrame,
    rotations: Vec<Rotation>,
}

impl CachedRotationData {
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
            start,
            end,
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

pub struct CachedRotationProvider {
    data: Vec<CachedRotationData>,
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
impl CachedRotationProvider {
    pub fn new() -> Self {
        Self { data: vec![] }
    }

    pub fn ensure_cached_rotation_data(
        &mut self,
        origin: DynFrame,
        target: DynFrame,
        interval: Interval<Time<DynTimeScale>>,
    ) {
        // TODO smarter check for the range
        // also support partially cached intervals
        //
        let start_found = self
            .get_cached_rotation_data(origin, target, interval.start())
            .is_some();
        let end_found = self
            .get_cached_rotation_data(origin, target, interval.end())
            .is_some();

        if !start_found || !end_found {
            self.data.push(CachedRotationData::build(
                interval.start().with_scale(Tai),
                interval.end().with_scale(Tai),
                30.0,
                origin,
                target,
            ));
        }
    }

    fn get_cached_rotation_data(
        &self,
        origin: DynFrame,
        target: DynFrame,
        time: Time<DynTimeScale>,
    ) -> Option<&CachedRotationData> {
        self.data.iter().find(|p| {
            p.from == origin
                && p.to == target
                && p.start.into_dyn() <= time
                && p.end.into_dyn() >= time
        })
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
        let Some(data) = self.get_cached_rotation_data(origin, target, time) else {
            // No cached data, return error
            panic!();
        };

        let delta = time.to_delta();
        Ok(data.get(delta))
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
pub(super) struct SimpleElevationDetector<'a> {
    pub gs: &'a GroundStation,
    pub trajectory: &'a DynTrajectory,
}

#[derive(thiserror::Error, Debug)]
pub(super) enum ElevationDetectError {}

impl<'a, T: TimeScale + Into<DynTimeScale>> DetectFn<T> for SimpleElevationDetector<'a> {
    type Error = ElevationDetectError;

    fn eval(&self, time: Time<T>) -> Result<f64, Self::Error> {
        let state = self.trajectory.interpolate_at(time.into_dyn());
        let state_bf = state
            .try_to_frame(self.gs.body_fixed_frame(), &DefaultRotationProvider)
            .unwrap();

        Ok(self
            .gs
            .location()
            .compute_observables(state_bf.position(), state_bf.velocity())
            .elevation())
    }
}
