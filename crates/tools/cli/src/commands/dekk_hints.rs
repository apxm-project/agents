//! User-facing Dekk command hints emitted by the APXM CLI.
//!
//! Keeping them centralized avoids reintroducing command-string literals at
//! call sites. A hint reachable only from a feature-gated command path carries
//! the same `cfg` as that path, so every hint here has a live caller.

pub const APXM_ENV_HINT: &str = "dekk agents ...";
/// Printed only by the stubs that stand in for driver-gated commands.
#[cfg(not(feature = "driver"))]
pub const BUILD: &str = "dekk agents build";
pub const INSTALL_NO_INTERACTIVE: &str = "dekk agents install --no-interactive";
pub const DOCTOR: &str = "dekk agents doctor";
/// Printed only by the driver-gated `backend` command.
#[cfg(feature = "driver")]
pub const BACKEND_ADD_MODEL: &str = "dekk agents backend add-model <name>";
pub const BACKEND_ADD_GENERIC: &str =
    "dekk agents backend add <name> --type <cloud|onprem|local> --protocol <protocol>";
/// Printed only by the driver-gated `backend` command.
#[cfg(feature = "driver")]
pub const BACKEND_ADD_OPENAI: &str =
    "dekk agents backend add openai --type cloud --protocol openai";
