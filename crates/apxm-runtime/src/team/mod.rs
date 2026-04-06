//! Agent Team support for coordinated multi-agent execution.
//!
//! Teams are higher-level abstractions that package multiple agent spawns with
//! predefined roles and prompts. A team definition resides in `~/.apxm/teams.toml`
//! and can be instantiated by a `spawn_team` call, which expands to N `spawn_agent`
//! operations.

mod registry;

pub use registry::{TeamDefinition, TeamMember, TeamRegistry};
