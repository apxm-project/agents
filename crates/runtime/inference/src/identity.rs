//! Model-effect identity and exact binding validation.
//!
//! A v1 model effect names exactly one [`ModelTargetRef`]. Admission materializes
//! one exact `ModelTargetRef -> ModelDeploymentRef -> ExactPortBindingRef`
//! mapping; [`ResolvedModelBinding`] references that binding by digest and never
//! duplicates adapter selection. Runtime receives the invocation's exact
//! materialized bindings and validates the one that matches each authored
//! target; there is no resolver, default, ambient alias, or first-available
//! selection.

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

/// Why an exact binding could not be validated. Every variant fails closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BindingError {
    /// The admitted binding is for a different target than the authored effect.
    TargetMismatch {
        authored: ModelTargetRef,
        admitted: ModelTargetRef,
    },
    /// Admission does not contain the authored target.
    MissingTarget(ModelTargetRef),
    /// Admission contains more than one binding for the authored target.
    DuplicateTarget(ModelTargetRef),
    /// An admitted mapping carried a binding digest that is not a sha256 value.
    InvalidBindingDigest(ModelTargetRef),
}

impl std::fmt::Display for BindingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TargetMismatch { authored, admitted } => {
                write!(
                    f,
                    "admitted model binding target {} does not match authored target {}",
                    admitted.0, authored.0
                )
            }
            Self::MissingTarget(target) => {
                write!(f, "admission does not contain binding for {}", target.0)
            }
            Self::DuplicateTarget(target) => {
                write!(f, "admission contains duplicate bindings for {}", target.0)
            }
            Self::InvalidBindingDigest(t) => {
                write!(f, "admitted binding for {} has a non-sha256 digest", t.0)
            }
        }
    }
}

impl std::error::Error for BindingError {}

/// The exact model-target bindings admitted for one Program Invocation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelBindingAdmission {
    resolved_bindings: Vec<ResolvedModelBinding>,
}

impl ModelBindingAdmission {
    #[must_use]
    pub fn new(resolved_binding: ResolvedModelBinding) -> Self {
        Self {
            resolved_bindings: vec![resolved_binding],
        }
    }

    /// Admit an exact finite set of model bindings for one invocation.
    #[must_use]
    pub fn for_invocation(resolved_bindings: Vec<ResolvedModelBinding>) -> Self {
        Self { resolved_bindings }
    }

    /// Return the one materialized binding after validating it against `target`.
    ///
    /// # Errors
    ///
    /// Returns a [`BindingError`] when the admitted bindings omit or duplicate
    /// the target, or when its binding digest is invalid.
    pub fn validate(&self, target: &ModelTargetRef) -> Result<ResolvedModelBinding, BindingError> {
        let mut matches = self
            .resolved_bindings
            .iter()
            .filter(|binding| &binding.model_target_ref == target);
        let Some(resolved_binding) = matches.next() else {
            return if self.resolved_bindings.len() == 1 {
                Err(BindingError::TargetMismatch {
                    authored: target.clone(),
                    admitted: self.resolved_bindings[0].model_target_ref.clone(),
                })
            } else {
                Err(BindingError::MissingTarget(target.clone()))
            };
        };
        if matches.next().is_some() {
            return Err(BindingError::DuplicateTarget(target.clone()));
        }
        if !is_digest(&resolved_binding.exact_port_binding.binding_digest) {
            return Err(BindingError::InvalidBindingDigest(target.clone()));
        }

        Ok(resolved_binding.clone())
    }
}
