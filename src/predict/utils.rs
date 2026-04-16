use lox_space::{
    analysis::visibility::DynPass,
    frames::{
        DynFrame,
        providers::DefaultRotationProvider,
        rotations::{DynRotationError, Rotation, TryRotation},
    },
    orbits::{events::DetectFn, orbits::DynTrajectory},
    prelude::{GroundStation, Interval, Pass, Tai, TimeDelta},
    time::{
        Time,
        deltas::ToDelta,
        intervals::TimeInterval,
        time_scales::{DynTimeScale, TimeScale},
    },
};

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
pub(super) fn sample_pass(
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

pub(super) struct SimpleElevationDetector<'a> {
    pub gs: &'a GroundStation,
    pub trajectory: &'a DynTrajectory,
    pub provider: &'a CachedRotationProvider,
}

#[derive(thiserror::Error, Debug)]
pub(super) enum ElevationDetectError {}

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
