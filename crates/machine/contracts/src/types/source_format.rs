//! Canonical APXM file format classification.
//!
//! User-authored workflow source is AIR or a frontend source that emits AIR.
//! JSON remains a structured data format for metrics, sessions, manifests,
//! cache entries, API envelopes, and diagnostics; it is not a graph source.

use std::path::Path;

use crate::constants::extensions;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphSourceFormat {
    Air,
    PythonFrontend,
    TypeScriptFrontend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactFormat {
    ApxmObj,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApxmPathFormat {
    GraphSource(GraphSourceFormat),
    Artifact(ArtifactFormat),
    JsonData,
    Unknown,
}

impl ApxmPathFormat {
    pub fn from_path(path: &Path) -> Self {
        match path.extension().and_then(|ext| ext.to_str()) {
            Some(extensions::AIR) => Self::GraphSource(GraphSourceFormat::Air),
            Some(extensions::PYTHON) => Self::GraphSource(GraphSourceFormat::PythonFrontend),
            Some(extensions::TYPESCRIPT) => {
                Self::GraphSource(GraphSourceFormat::TypeScriptFrontend)
            }
            Some(extensions::ARTIFACT) => Self::Artifact(ArtifactFormat::ApxmObj),
            Some(extensions::JSON_DATA) => Self::JsonData,
            _ => Self::Unknown,
        }
    }

    pub fn is_air_source(self) -> bool {
        matches!(self, Self::GraphSource(GraphSourceFormat::Air))
    }

    pub fn is_python_frontend(self) -> bool {
        matches!(self, Self::GraphSource(GraphSourceFormat::PythonFrontend))
    }

    pub fn is_typescript_frontend(self) -> bool {
        matches!(
            self,
            Self::GraphSource(GraphSourceFormat::TypeScriptFrontend)
        )
    }

    pub fn is_json_data(self) -> bool {
        matches!(self, Self::JsonData)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_frontend_sources_and_json_data() {
        assert!(ApxmPathFormat::from_path(Path::new("flow.air")).is_air_source());
        assert!(ApxmPathFormat::from_path(Path::new("flow.py")).is_python_frontend());
        assert!(ApxmPathFormat::from_path(Path::new("flow.ts")).is_typescript_frontend());
        assert!(ApxmPathFormat::from_path(Path::new("metrics.json")).is_json_data());
    }
}
