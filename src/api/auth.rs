use axum::extract::FromRequestParts;
use axum::http::request::Parts;

use crate::config::Permission;

use super::{AppState, error::ApiError};

/// Extracts and validates the API key from the `api_key` header.
pub struct AuthenticatedKey {
    pub permissions: Vec<Permission>,
}

impl AuthenticatedKey {
    pub fn has(&self, permission: Permission) -> bool {
        self.permissions.contains(&permission)
    }

    pub fn require(&self, permission: Permission) -> Result<(), ApiError> {
        if self.has(permission) {
            Ok(())
        } else {
            Err(ApiError::Forbidden)
        }
    }
}

impl FromRequestParts<AppState> for AuthenticatedKey {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let key_value = parts
            .headers
            .get("api_key")
            .and_then(|v| v.to_str().ok())
            .ok_or(ApiError::Unauthorized)?;

        let api_key = state
            .config
            .api
            .keys
            .iter()
            .find(|k| k.key == key_value)
            .ok_or(ApiError::Unauthorized)?;

        Ok(AuthenticatedKey {
            permissions: api_key.permissions.clone(),
        })
    }
}
