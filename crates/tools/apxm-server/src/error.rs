use apxm_core::error::RuntimeError;
use axum::{Json, response::IntoResponse};
use tracing::error;

use crate::types::errors::{ApiFaultCode, TypedError};

#[derive(Debug)]
pub(crate) struct ApiError {
    pub(crate) status: axum::http::StatusCode,
    pub(crate) message: String,
    code: ApiFaultCode,
    recovery_hint: Option<String>,
}

impl ApiError {
    fn with_code(
        code: ApiFaultCode,
        message: impl Into<String>,
        recovery_hint: Option<String>,
    ) -> Self {
        let message = message.into();
        Self {
            status: code.http_status(),
            message,
            code,
            recovery_hint,
        }
    }

    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self::with_code(ApiFaultCode::BadRequest, message, None)
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::with_code(ApiFaultCode::NotFound, message, None)
    }

    /// Inference/admission slot unavailable — client should retry later.
    pub(crate) fn too_many_requests(message: impl Into<String>) -> Self {
        Self::with_code(
            ApiFaultCode::TooManyRequests,
            message,
            Some("Retry after server capacity frees up.".to_string()),
        )
    }

    pub(crate) fn unprocessable(message: impl Into<String>) -> Self {
        Self::with_code(ApiFaultCode::UnprocessableEntity, message, None)
    }

    /// Admission policy rejected the request (capability grants, spawn policy, etc.).
    pub(crate) fn admission_denied(message: impl Into<String>) -> Self {
        Self::with_code(
            ApiFaultCode::AdmissionDenied,
            message,
            Some("Adjust capability grants or admission policy.".to_string()),
        )
    }

    pub(crate) fn conflict(message: impl Into<String>) -> Self {
        Self::with_code(ApiFaultCode::Conflict, message, None)
    }

    pub(crate) fn runtime(error: RuntimeError) -> Self {
        error!(error = %error, "runtime error");
        let typed = TypedError::from_runtime(&error);
        Self {
            status: typed.http_status(),
            message: typed.message.clone(),
            code: ApiFaultCode::from_wire(&typed.code).unwrap_or(ApiFaultCode::RuntimeError),
            recovery_hint: typed.recovery_hint.clone(),
        }
    }

    pub(crate) fn internal_message(message: impl Into<String>) -> Self {
        Self::with_code(ApiFaultCode::InternalError, message, None)
    }

    pub(crate) fn typed(typed: TypedError) -> Self {
        Self {
            status: typed.http_status(),
            message: typed.message.clone(),
            code: ApiFaultCode::from_wire(&typed.code).unwrap_or(ApiFaultCode::InternalError),
            recovery_hint: typed.recovery_hint.clone(),
        }
    }

    pub(crate) fn into_typed(self) -> TypedError {
        self.to_typed()
    }

    pub(crate) fn typed_ref(&self) -> TypedError {
        self.to_typed()
    }

    fn to_typed(&self) -> TypedError {
        TypedError::new(self.code, self.message.clone(), self.recovery_hint.clone())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let status = self.status;
        let body = self.to_typed();
        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;
    use http_body_util::BodyExt;

    async fn response_json(error: ApiError) -> serde_json::Value {
        let response = error.into_response();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        serde_json::from_slice(&bytes).expect("typed error json")
    }

    #[tokio::test]
    async fn runtime_error_returns_typed_envelope() {
        let json = response_json(ApiError::runtime(RuntimeError::InvalidTask {
            reason: "bad task".to_string(),
        }))
        .await;

        assert_eq!(json["class"], "program_fault");
        assert_eq!(json["code"], "invalid_task");
        assert!(json["message"].is_string());
    }

    #[tokio::test]
    async fn admission_saturation_maps_to_too_many_requests() {
        let json = response_json(ApiError::too_many_requests("capacity saturated")).await;

        assert_eq!(json["class"], "program_fault");
        assert_eq!(json["code"], "too_many_requests");
    }

    #[tokio::test]
    async fn admission_denied_maps_to_program_fault() {
        let json = response_json(ApiError::admission_denied("spawn not allowed")).await;

        assert_eq!(json["class"], "program_fault");
        assert_eq!(json["code"], "admission_denied");
    }

    #[tokio::test]
    async fn typed_constructor_round_trips_fault_code() {
        let json = response_json(ApiError::typed(TypedError::program_fault(
            ApiFaultCode::NotFound,
            "missing checkpoint",
            None,
        )))
        .await;

        assert_eq!(json["class"], "program_fault");
        assert_eq!(json["code"], "not_found");
    }
}
