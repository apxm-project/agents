//! Exact inference driver/profile/binding contracts.
//!
//! One model effect resolves to one admitted driver binding. An unavailable
//! binding is a typed failure. There is no ranking, alias, first-available
//! selection, silent substitution, dynamic routing, or fallback target.

use serde::{Deserialize, Serialize};

use crate::identity::{ModelTargetRef, ResolvedModelBinding};
use apxm_program::grammar::is_digest;

/// Schema identity for the frozen driver-binding contract.
pub const INFERENCE_DRIVER_BINDING_SCHEMA: &str = "apxm.inference-driver-binding.v1";

/// Explicit availability of one exact inference driver binding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriverAvailability {
    Available,
    Unavailable,
}

/// Exact driver/profile/binding admission for one model effect.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InferenceDriverBinding {
    pub schema_version: String,
    pub driver_id: String,
    pub inference_profile_ref: String,
    pub model_target_ref: String,
    pub model_target_digest: String,
    pub model_deployment_ref: String,
    pub exact_port_binding_digest: String,
    pub port_contract_digest: String,
    pub composition_digest: String,
    pub availability: DriverAvailability,
}

/// Why an exact driver binding cannot be used. Every variant fails closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriverBindingError {
    TargetMismatch {
        authored: String,
        admitted: String,
    },
    DigestMismatch {
        field: &'static str,
    },
    InvalidDigest(&'static str),
    EmptyField(&'static str),
    Unavailable {
        driver_id: String,
        inference_profile_ref: String,
    },
    UnknownField,
}

impl std::fmt::Display for DriverBindingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TargetMismatch { authored, admitted } => write!(
                f,
                "driver binding target {admitted} does not match authored target {authored}"
            ),
            Self::DigestMismatch { field } => {
                write!(
                    f,
                    "driver binding {field} does not match admitted resolution"
                )
            }
            Self::InvalidDigest(field) => {
                write!(f, "driver binding field {field} is not a sha256 digest")
            }
            Self::EmptyField(field) => write!(f, "driver binding field {field} is empty"),
            Self::Unavailable {
                driver_id,
                inference_profile_ref,
            } => write!(
                f,
                "inference driver {driver_id} profile {inference_profile_ref} is unavailable"
            ),
            Self::UnknownField => write!(f, "driver binding rejected unknown wire fields"),
        }
    }
}

impl std::error::Error for DriverBindingError {}

impl InferenceDriverBinding {
    /// Construct one exact available driver binding from an admitted resolution.
    pub fn from_resolved(
        driver_id: impl Into<String>,
        inference_profile_ref: impl Into<String>,
        resolved: &ResolvedModelBinding,
    ) -> Result<Self, DriverBindingError> {
        let binding = Self {
            schema_version: INFERENCE_DRIVER_BINDING_SCHEMA.to_string(),
            driver_id: driver_id.into(),
            inference_profile_ref: inference_profile_ref.into(),
            model_target_ref: resolved.model_target.reference.0.clone(),
            model_target_digest: resolved.model_target.target_digest.clone(),
            model_deployment_ref: resolved.model_deployment_ref.0.clone(),
            exact_port_binding_digest: resolved.binding_digest().to_string(),
            port_contract_digest: resolved.port_contract_digest().to_string(),
            composition_digest: resolved.composition_digest.clone(),
            availability: DriverAvailability::Available,
        };
        binding.validate_shape()?;
        Ok(binding)
    }

    /// Mark the exact binding unavailable without changing its target identity.
    #[must_use]
    pub fn unavailable(mut self) -> Self {
        self.availability = DriverAvailability::Unavailable;
        self
    }

    /// Validate wire shape without checking authored-target membership.
    pub fn validate_shape(&self) -> Result<(), DriverBindingError> {
        if self.schema_version != INFERENCE_DRIVER_BINDING_SCHEMA {
            return Err(DriverBindingError::EmptyField("schema_version"));
        }
        for (field, value) in [
            ("driver_id", self.driver_id.as_str()),
            ("inference_profile_ref", self.inference_profile_ref.as_str()),
            ("model_target_ref", self.model_target_ref.as_str()),
            ("model_deployment_ref", self.model_deployment_ref.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(DriverBindingError::EmptyField(field));
            }
        }
        for (field, digest) in [
            ("model_target_digest", self.model_target_digest.as_str()),
            (
                "exact_port_binding_digest",
                self.exact_port_binding_digest.as_str(),
            ),
            ("port_contract_digest", self.port_contract_digest.as_str()),
            ("composition_digest", self.composition_digest.as_str()),
        ] {
            if !is_digest(digest) {
                return Err(DriverBindingError::InvalidDigest(field));
            }
        }
        Ok(())
    }

    /// Authorize this binding for one authored target and admitted resolution.
    ///
    /// # Errors
    ///
    /// Returns [`DriverBindingError`] when the binding is unavailable, targets
    /// disagree, digests disagree, or the shape is invalid. Never substitutes
    /// another target or driver.
    pub fn authorize(
        &self,
        authored_target: &ModelTargetRef,
        resolved: &ResolvedModelBinding,
    ) -> Result<(), DriverBindingError> {
        self.validate_shape()?;
        if self.model_target_ref != authored_target.0 {
            return Err(DriverBindingError::TargetMismatch {
                authored: authored_target.0.clone(),
                admitted: self.model_target_ref.clone(),
            });
        }
        if self.model_target_ref != resolved.model_target.reference.0 {
            return Err(DriverBindingError::TargetMismatch {
                authored: authored_target.0.clone(),
                admitted: self.model_target_ref.clone(),
            });
        }
        if self.model_target_digest != resolved.model_target.target_digest {
            return Err(DriverBindingError::DigestMismatch {
                field: "model_target_digest",
            });
        }
        if self.model_deployment_ref != resolved.model_deployment_ref.0 {
            return Err(DriverBindingError::DigestMismatch {
                field: "model_deployment_ref",
            });
        }
        if self.exact_port_binding_digest != resolved.binding_digest() {
            return Err(DriverBindingError::DigestMismatch {
                field: "exact_port_binding_digest",
            });
        }
        if self.port_contract_digest != resolved.port_contract_digest() {
            return Err(DriverBindingError::DigestMismatch {
                field: "port_contract_digest",
            });
        }
        if self.composition_digest != resolved.composition_digest {
            return Err(DriverBindingError::DigestMismatch {
                field: "composition_digest",
            });
        }
        if self.availability == DriverAvailability::Unavailable {
            return Err(DriverBindingError::Unavailable {
                driver_id: self.driver_id.clone(),
                inference_profile_ref: self.inference_profile_ref.clone(),
            });
        }
        Ok(())
    }
}
