use axum::{http::StatusCode, response::IntoResponse, response::Response, Json};
use serde_json::json;

pub enum AppError {
    NotFound(String),
    BadRequest(String),
    Upstream(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg),
            AppError::Upstream(msg) => (StatusCode::BAD_GATEWAY, msg),
        };

        if status.is_server_error() {
            tracing::error!(%status, %message, "request failed");
        } else {
            tracing::warn!(%status, %message, "request failed");
        }

        (status, Json(json!({ "error": message }))).into_response()
    }
}
