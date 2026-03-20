use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

pub enum ApiError {
    Unauthorized,
    Forbidden,
    NotFound,
    BadRequest(String),
    Conflict(String),
    Internal,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self {
            ApiError::Unauthorized => {
                (StatusCode::UNAUTHORIZED, "missing or invalid API key").into_response()
            }
            ApiError::Forbidden => {
                (StatusCode::FORBIDDEN, "insufficient permissions").into_response()
            }
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found").into_response(),
            ApiError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg).into_response(),
            ApiError::Conflict(msg) => (StatusCode::CONFLICT, msg).into_response(),
            ApiError::Internal => {
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
            }
        }
    }
}
