//! Frontend support for the APXM Python authoring layer.
//!
//! This crate provides a deterministic registry facade and Python code
//! generation helpers. It intentionally starts small: repo-owned constants,
//! AIS operation specs, and built-in ACP agent templates.

pub mod codegen;
pub mod registry;

pub use codegen::{
    GeneratedFrontendFiles, render_generated_files, render_generated_init, render_generated_python,
    write_generated_python,
};
pub use registry::{
    FrontendAgentTemplate, FrontendConstant, FrontendEmissionSpec, FrontendOperationSpec,
    agent_templates, emission_specs, graph_attr_constants, graph_metadata_constants,
    operation_specs,
};
