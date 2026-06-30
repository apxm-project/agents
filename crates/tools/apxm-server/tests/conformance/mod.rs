//! Conformance test suite for DIRECT, LINK-TOOLS, and LINK-RUNTIME execution modes.
//!
//! Each sub-module tests one tier archetype against the spec vectors defined in
//! the naming-and-infrastructure-plan §Tier Archetypes.

pub mod mock_gateway;

pub mod direct;
pub mod link_runtime;
pub mod link_tools;
