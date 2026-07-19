//! Model-effect identity and exact binding resolution.
//!
//! A v1 model effect names exactly one [`ModelTargetRef`]. Admission materializes
//! one exact `ModelTargetRef -> ModelDeploymentRef -> ExactPortBindingRef`
//! mapping; [`ResolvedModelBinding`] references that binding by digest and never
//! duplicates adapter selection. Resolution is exact and fails closed: there is
//! no default, ambient, alias, or first-available selection, so zero or multiple
//! matches are errors, never a silent pick.

use serde::{Deserialize, Serialize};

use apxm_program::grammar::is_digest;

/// The one exact model target authored by a `model.call` effect.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelTargetRef(pub String);

/// A materialized model deployment reference.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelDeploymentRef(pub String);

/// A reference to one immutable Exact Port Binding, identified by its digest.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExactPortBindingRef {
    pub binding_digest: String,
}

/// The single admitted resolution for one model target. It references the exact
/// binding digest and carries no adapter/provider selection of its own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedModelBinding {
    pub model_target_ref: ModelTargetRef,
    pub model_deployment_ref: ModelDeploymentRef,
    pub exact_port_binding: ExactPortBindingRef,
}

impl ResolvedModelBinding {
    /// The exact port binding digest this resolution references.
    #[must_use]
    pub fn binding_digest(&self) -> &str {
        &self.exact_port_binding.binding_digest
    }
}

/// One admitted exact mapping entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionEntry {
    pub model_target_ref: ModelTargetRef,
    pub model_deployment_ref: ModelDeploymentRef,
    pub exact_port_binding: ExactPortBindingRef,
}

/// Why an exact binding could not be resolved. Every variant fails closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindingError {
    /// No admitted mapping for the target. There is no first-available fallback.
    NoBinding(ModelTargetRef),
    /// More than one admitted mapping for the target. There is no alias or
    /// first-match tie-break.
    AmbiguousBinding(ModelTargetRef),
    /// An admitted mapping carried a binding digest that is not a sha256 value.
    InvalidBindingDigest(ModelTargetRef),
}

impl std::fmt::Display for BindingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoBinding(t) => write!(f, "no admitted binding for model target {}", t.0),
            Self::AmbiguousBinding(t) => {
                write!(f, "multiple admitted bindings for model target {}", t.0)
            }
            Self::InvalidBindingDigest(t) => {
                write!(f, "admitted binding for {} has a non-sha256 digest", t.0)
            }
        }
    }
}

impl std::error::Error for BindingError {}

/// The admitted set of exact model-target bindings for one invocation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ModelBindingAdmission {
    entries: Vec<AdmissionEntry>,
}

impl ModelBindingAdmission {
    #[must_use]
    pub fn new(entries: Vec<AdmissionEntry>) -> Self {
        Self { entries }
    }

    /// Resolve exactly one binding for `target`, failing closed on zero or
    /// multiple matches. No default, ambient, alias, or first-available path
    /// exists.
    ///
    /// # Errors
    ///
    /// Returns a [`BindingError`] when the mapping is absent, ambiguous, or
    /// carries an invalid digest.
    pub fn resolve(&self, target: &ModelTargetRef) -> Result<ResolvedModelBinding, BindingError> {
        let mut matches = self
            .entries
            .iter()
            .filter(|entry| &entry.model_target_ref == target);

        let Some(entry) = matches.next() else {
            return Err(BindingError::NoBinding(target.clone()));
        };
        if matches.next().is_some() {
            return Err(BindingError::AmbiguousBinding(target.clone()));
        }
        if !is_digest(&entry.exact_port_binding.binding_digest) {
            return Err(BindingError::InvalidBindingDigest(target.clone()));
        }

        Ok(ResolvedModelBinding {
            model_target_ref: entry.model_target_ref.clone(),
            model_deployment_ref: entry.model_deployment_ref.clone(),
            exact_port_binding: entry.exact_port_binding.clone(),
        })
    }
}
