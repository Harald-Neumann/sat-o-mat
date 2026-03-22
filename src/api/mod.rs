pub mod auth;
pub mod error;
mod station;
mod tasks;

use std::path::PathBuf;
use std::sync::Arc;

use utoipa::{
    Modify, OpenApi,
    openapi::security::{ApiKey, ApiKeyValue, SecurityScheme},
};
use utoipa_axum::router::OpenApiRouter;

use crate::config::Config;

const TASKS_TAG: &str = "tasks";
const STATION_TAG: &str = "station";

#[derive(Clone)]
pub struct AppState {
    pub tasks_path: PathBuf,
    pub config: Arc<Config>,
}

// --- OpenAPI ---

#[derive(OpenApi)]
#[openapi(
    tags(
        (name = TASKS_TAG, description = "Tasks API"),
        (name = STATION_TAG, description = "Station API")
    ),
    modifiers(&SecurityAddon)
)]
struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "api_key",
                SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("api_key"))),
            )
        }
    }
}

// --- Router ---

pub fn router(config: &Config) -> OpenApiRouter {
    let state = AppState {
        tasks_path: config.tasks_path.clone(),
        config: Arc::new(config.clone()),
    };

    OpenApiRouter::with_openapi(ApiDoc::openapi())
        .nest(
            "/api",
            OpenApiRouter::new()
                .routes(utoipa_axum::routes!(station::get_station))
                .routes(utoipa_axum::routes!(tasks::list_tasks))
                .routes(utoipa_axum::routes!(
                    tasks::get_task,
                    tasks::put_task,
                    tasks::delete_task
                )),
        )
        .with_state(state)
}
