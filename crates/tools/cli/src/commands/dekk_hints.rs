//! User-facing Dekk command hints emitted by the APXM CLI.
//!
//! Some hints are only referenced from feature-gated command paths. Keeping
//! them centralized avoids reintroducing command-string literals at call sites.

#![allow(dead_code)]

pub const APXM_ENV_HINT: &str = "dekk agents ...";
#[cfg(not(feature = "driver"))]
pub const BUILD: &str = "dekk agents build";
pub const INSTALL_NO_INTERACTIVE: &str = "dekk agents install --no-interactive";
pub const DOCTOR: &str = "dekk agents doctor";
pub const BACKEND_LIST: &str = "dekk agents backend list";
pub const BACKEND_ADD_MODEL: &str = "dekk agents backend add-model <name>";
pub const BACKEND_ADD_GENERIC: &str =
    "dekk agents backend add <name> --type <cloud|onprem|local> --protocol <protocol>";
pub const BACKEND_ADD_OPENAI: &str =
    "dekk agents backend add openai --type cloud --protocol openai";
pub const BACKEND_ADD_OLLAMA: &str = "dekk agents backend add ollama --protocol ollama";
pub const TOOL_ADD_WITH_DESCRIPTION: &str = "dekk agents tool add <name> --description \"...\"";
pub const VLLM_ENABLE_SERVED_MODEL: &str = "dekk agents vllm enable <SERVED_MODEL_ID>";
