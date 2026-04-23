use apxm_core::error::RuntimeError;
use axum::{Json, response::IntoResponse};
use tracing::error;

#[derive(Debug)]
pub(crate) struct ApiError {
    pub(crate) status: axum::http::StatusCode,
    pub(crate) message: String,
}

impl ApiError {
    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: axum::http::StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: axum::http::StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }

    pub(crate) fn runtime(error: RuntimeError) -> Self {
        error!(error = %error, "runtime error");
        Self {
            status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }

    pub(crate) fn internal_message(message: impl Into<String>) -> Self {
        Self {
            status: axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let body = Json(serde_json::json!({
            "error": self.message
        }));
        (self.status, body).into_response()
    }
}
