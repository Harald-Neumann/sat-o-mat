use std::collections::HashMap;

use axum::Json;
use axum::extract::{Query, State};
use chrono::{DateTime, Duration, Utc};
use lox_space::time::utc::transformations::ToUtc;
use serde::{Deserialize, Serialize};
use tracing::info;
use utoipa::{IntoParams, ToSchema};

use crate::{api::error::ApiError, predict::PredictDb};

use super::AppState;

#[derive(Debug, Deserialize, IntoParams)]
pub struct PredictQuery {
    /// Start time as RFC3339. Defaults to now.
    #[param(value_type = Option<String>)]
    pub start: Option<DateTime<Utc>>,
    /// End time as RFC3339. Defaults to start + 24h.
    #[param(value_type = Option<String>)]
    pub end: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PassPredictions {
    predictions: HashMap<String, Vec<ApiPass>>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiPass {
    /// Start time formatted as RFC3339
    start: String,
    /// End time formatted as RFC3339
    end: String,
    /// Azimuth angle in degrees
    azimuth: Vec<f64>,
    /// Elevation angle in degrees
    elevation: Vec<f64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct GroundTrackPredictions {
    predictions: HashMap<String, ApiGroundTrack>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiGroundTrack {
    /// Start time formatted as RFC3339
    start: String,
    /// Latitude in degrees
    latitude: Vec<f64>,
    /// Longitude in degrees
    longitude: Vec<f64>,
}

/// Get pass predictions.
#[utoipa::path(
    get,
    path = "/predict/passes",
    tag = super::PREDICT_TAG,
    params(PredictQuery),
    responses(
        (status = 200, description = "Pass predictions", body = PassPredictions),
        (status = 400, description = "Invalid parameters"),
    ),
)]
pub async fn get_passes(
    State(state): State<AppState>,
    Query(query): Query<PredictQuery>,
) -> Result<Json<PassPredictions>, ApiError> {
    let start = query.start.unwrap_or_else(Utc::now);
    let end = query.end.unwrap_or_else(|| start + Duration::hours(24));

    if end <= start {
        return Err(ApiError::BadRequest("end must be after start".to_string()));
    }

    let mut predict_db = PredictDb::new();
    let count = predict_db
        .add_tles(&state.config.tle_path)
        .map_err(|_| ApiError::Internal)?;
    info!(?count, "satellites loaded");

    let gs = state
        .config
        .ground_station
        .as_ref()
        .ok_or(ApiError::Internal)?;

    let predictions = predict_db
        .predict_passes(start, end, gs, None)
        .into_iter()
        .map(|(id, passes)| {
            let passes = passes
                .into_iter()
                .map(|pass| {
                    let interval = pass.interval();

                    let (azimuth, elevation) = pass
                        .observables()
                        .iter()
                        .map(|obs| (obs.azimuth().to_degrees(), obs.elevation().to_degrees()))
                        .collect();

                    ApiPass {
                        start: DateTime::<Utc>::try_from(interval.start().to_utc())
                            .unwrap()
                            .to_rfc3339(),
                        end: DateTime::<Utc>::try_from(interval.end().to_utc())
                            .unwrap()
                            .to_rfc3339(),
                        azimuth,
                        elevation,
                    }
                })
                .collect();

            (id.to_string(), passes)
        })
        .collect();

    Ok(Json(PassPredictions { predictions }))
}

/// Get ground track predictions.
#[utoipa::path(
    get,
    path = "/predict/ground_track",
    tag = super::PREDICT_TAG,
    params(PredictQuery),
    responses(
        (status = 200, description = "Ground track predictions", body = GroundTrackPredictions),
        (status = 400, description = "Invalid parameters"),
    ),
)]
pub async fn get_ground_track(
    State(state): State<AppState>,
    Query(query): Query<PredictQuery>,
) -> Result<Json<GroundTrackPredictions>, ApiError> {
    let start = query.start.unwrap_or_else(Utc::now);
    let end = query.end.unwrap_or_else(|| start + Duration::hours(24));

    if end <= start {
        return Err(ApiError::BadRequest("end must be after start".to_string()));
    }

    let mut predict_db = PredictDb::new();
    let count = predict_db
        .add_tles(&state.config.tle_path)
        .map_err(|_| ApiError::Internal)?;
    info!(?count, "satellites loaded");

    let predictions = predict_db
        .predict_ground_track(start, end, None)
        .into_iter()
        .map(|(id, track)| {
            let (lats, lons) = track
                .into_iter()
                .map(|(_, lla)| (lla.lat().to_degrees(), lla.lon().to_degrees()))
                .collect();

            (
                id.to_string(),
                ApiGroundTrack {
                    start: start.to_rfc3339(),
                    latitude: lats,
                    longitude: lons,
                },
            )
        })
        .collect();

    Ok(Json(GroundTrackPredictions { predictions }))
}
