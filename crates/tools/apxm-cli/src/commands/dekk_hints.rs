//! User-facing Dekk command hints emitted by the APXM CLI.
//!
//! Some hints are only referenced from feature-gated command paths. Keeping
//! them centralized avoids reintroducing command-string literals at call sites.

#![allow(dead_code)]

pub const APXM_ENV_HINT: &str = "dekk apxm ...";
#[cfg(not(feature = "driver"))]
pub const BUILD: &str = "dekk apxm build";
pub const BUILD_GUI: &str = "dekk apxm build-gui";
pub const INSTALL_NO_INTERACTIVE: &str = "dekk apxm install --no-interactive";
pub const DOCTOR: &str = "dekk apxm doctor";
pub const BACKEND_LIST: &str = "dekk apxm backend list";
pub const BACKEND_SYNC_MODELS: &str = "dekk apxm backend sync-models <name>";
pub const BACKEND_ADD_GENERIC: &str =
    "dekk apxm backend add <name> --type <cloud|onprem|local> --protocol <protocol>";
pub const BACKEND_ADD_OPENAI: &str = "dekk apxm backend add openai --type cloud --protocol openai";
pub const BACKEND_ADD_OLLAMA: &str = "dekk apxm backend add ollama --protocol ollama";
pub const TOOL_ADD_WITH_DESCRIPTION: &str = "dekk apxm tool add <name> --description \"...\"";
#[cfg(not(feature = "driver"))]
pub const WORKFLOW_RUN: &str = "dekk apxm workflow run ...";
pub const VLLM_ENABLE_SERVED_MODEL: &str = "dekk apxm vllm enable <SERVED_MODEL_ID>";
