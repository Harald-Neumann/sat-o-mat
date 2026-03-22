use axum::Json;
use axum::extract::State;
use serde::Serialize;
use utoipa::ToSchema;

use super::AppState;

#[derive(Debug, Serialize, ToSchema)]
pub struct StationInfo {
    pub name: String,
}

/// Get station information.
#[utoipa::path(
    get,
    path = "/station",
    tag = super::STATION_TAG,
    responses(
        (status = 200, description = "Station info", body = StationInfo),
    ),
)]
pub async fn get_station(
    State(state): State<AppState>,
) -> Json<StationInfo> {
    Json(StationInfo {
        name: state.config.station_name.clone(),
    })
}
