//! HTTP error responses for the GUI API.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};

pub struct AppError(pub StatusCode, pub String);

impl AppError {
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self(StatusCode::NOT_FOUND, msg.into())
    }

    pub fn forbidden(msg: impl Into<String>) -> Self {
        Self(StatusCode::FORBIDDEN, msg.into())
    }

    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self(StatusCode::BAD_REQUEST, msg.into())
    }

    pub fn internal(msg: impl Into<String>) -> Self {
        Self(StatusCode::INTERNAL_SERVER_ERROR, msg.into())
    }

    pub fn bad_gateway(msg: impl Into<String>) -> Self {
        Self(StatusCode::BAD_GATEWAY, msg.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({ "error": self.1 });
        (self.0, Json(body)).into_response()
    }
}

impl<E: std::fmt::Display> From<E> for AppError {
    fn from(err: E) -> Self {
        AppError(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
    }
}

pub type ApiResult<T> = Result<T, AppError>;
