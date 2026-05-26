//! Artifact loader and parser for compiled `.apxmobj` files.
//!
//! Reads the APXM binary wire format (version 1, magic `b"APXM"`) produced by
//! the MLIR compiler, validates integrity via BLAKE3 checksums, and deserializes
//! the execution DAG from the artifact container.

use std::io::{Read, Write};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use apxm_core::types::execution::ExecutionDag;
use blake3::Hasher;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod wire;

const MAGIC: &[u8; 4] = b"APXM";
const VERSION: u32 = 1;

// Wire format header layout: [MAGIC][VERSION][PAYLOAD_LEN][HASH][FLAGS]
const SIZE_MAGIC: usize = 4;
const SIZE_VERSION: usize = 4;
const SIZE_PAYLOAD_LEN: usize = 8;
const SIZE_HASH: usize = 32;
const SIZE_FLAGS: usize = 4;
const HEADER_SIZE: usize = SIZE_MAGIC + SIZE_VERSION + SIZE_PAYLOAD_LEN + SIZE_HASH + SIZE_FLAGS;

const OFFSET_VERSION: usize = SIZE_MAGIC;
const OFFSET_PAYLOAD_LEN: usize = OFFSET_VERSION + SIZE_VERSION;
const OFFSET_HASH: usize = OFFSET_PAYLOAD_LEN + SIZE_PAYLOAD_LEN;
const OFFSET_FLAGS: usize = OFFSET_HASH + SIZE_HASH;

pub mod section_kinds {
    pub const SKILL_MANIFEST_V1: &str = "apxm.skill_manifest.v1";
}

#[derive(Debug, Error)]
pub enum ArtifactError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] Box<bincode::ErrorKind>),
    #[error("Invalid artifact header")]
    InvalidHeader,
    #[error("Artifact version mismatch: {0}")]
    VersionMismatch(u32),
    #[error("Artifact hash mismatch")]
    HashMismatch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactMetadata {
    pub module_name: Option<String>,
    pub created_at: u64,
    pub compiler_version: String,
}

impl ArtifactMetadata {
    pub fn new(module_name: Option<String>, compiler_version: impl Into<String>) -> Self {
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            module_name,
            created_at,
            compiler_version: compiler_version.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArtifactSection {
    pub kind: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArtifactPayload {
    metadata: ArtifactMetadata,
    dags: Vec<wire::WireDag>,
    sections: Vec<ArtifactSection>,
}

#[derive(Debug, Clone)]
pub struct Artifact {
    metadata: ArtifactMetadata,
    dags: Vec<ExecutionDag>,
    sections: Vec<ArtifactSection>,
    flags: u32,
}

impl Artifact {
    pub fn new(metadata: ArtifactMetadata, dags: Vec<ExecutionDag>) -> Self {
        Self {
            metadata,
            dags,
            sections: Vec::new(),
            flags: 0,
        }
    }

    pub fn metadata(&self) -> &ArtifactMetadata {
        &self.metadata
    }

    /// Overwrite the embedded `created_at` timestamp. `ArtifactMetadata::new`
    /// stamps `SystemTime::now()`, which makes the wire bytes (and any BLAKE3
    /// over them) non-deterministic across rebuilds. Pin to a constant for
    /// reproducible artifact hashing.
    pub fn set_created_at(&mut self, created_at: u64) {
        self.metadata.created_at = created_at;
    }

    /// Get all DAGs in the artifact
    pub fn dags(&self) -> &[ExecutionDag] {
        &self.dags
    }

    /// Get the @entry DAG (first with is_entry=true)
    pub fn entry_dag(&self) -> Option<&ExecutionDag> {
        self.dags.iter().find(|d| d.metadata.is_entry)
    }

    /// Get non-entry DAGs (for flow registration)
    pub fn flow_dags(&self) -> impl Iterator<Item = &ExecutionDag> {
        self.dags.iter().filter(|d| !d.metadata.is_entry)
    }

    pub fn sections(&self) -> &[ArtifactSection] {
        &self.sections
    }

    pub fn section_data(&self, kind: &str) -> Option<&[u8]> {
        self.sections
            .iter()
            .find(|section| section.kind == kind)
            .map(|section| section.data.as_slice())
    }

    /// Append an extra section to the artifact.
    pub fn add_section(&mut self, section: ArtifactSection) {
        self.sections.push(section);
    }

    pub fn replace_section(&mut self, kind: impl Into<String>, data: Vec<u8>) {
        let kind = kind.into();
        if let Some(section) = self
            .sections
            .iter_mut()
            .find(|section| section.kind == kind)
        {
            section.data = data;
        } else {
            self.sections.push(ArtifactSection { kind, data });
        }
    }

    /// Consume artifact and return all DAGs
    pub fn into_dags(self) -> Vec<ExecutionDag> {
        self.dags
    }

    /// Consume artifact and return the explicit @entry DAG.
    pub fn into_entry_dag(self) -> Option<ExecutionDag> {
        self.dags.into_iter().find(|dag| dag.metadata.is_entry)
    }

    fn payload(&self) -> Result<Vec<u8>, Box<bincode::ErrorKind>> {
        let payload = ArtifactPayload {
            metadata: self.metadata.clone(),
            dags: self
                .dags
                .iter()
                .map(wire::WireDag::from_execution_dag)
                .collect(),
            sections: self.sections.clone(),
        };
        bincode::serialize(&payload)
    }

    pub fn payload_hash(&self) -> ArtifactResult<[u8; 32]> {
        let payload = self.payload()?;
        let mut hasher = Hasher::new();
        hasher.update(&payload);
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(hasher.finalize().as_bytes());
        Ok(bytes)
    }

    pub fn to_bytes(&self) -> ArtifactResult<Vec<u8>> {
        let payload = self.payload()?;
        let mut hasher = Hasher::new();
        hasher.update(&payload);
        let digest = hasher.finalize();

        let mut bytes = Vec::with_capacity(HEADER_SIZE + payload.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        bytes.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        bytes.extend_from_slice(digest.as_bytes());
        bytes.extend_from_slice(&self.flags.to_le_bytes());
        bytes.extend_from_slice(&payload);
        Ok(bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> ArtifactResult<Self> {
        if bytes.len() < HEADER_SIZE {
            return Err(ArtifactError::InvalidHeader);
        }

        if &bytes[..SIZE_MAGIC] != MAGIC {
            return Err(ArtifactError::InvalidHeader);
        }

        let version = u32::from_le_bytes(
            bytes[OFFSET_VERSION..OFFSET_VERSION + SIZE_VERSION]
                .try_into()
                .map_err(|_| ArtifactError::InvalidHeader)?,
        );
        if version != VERSION {
            return Err(ArtifactError::VersionMismatch(version));
        }

        let payload_len = u64::from_le_bytes(
            bytes[OFFSET_PAYLOAD_LEN..OFFSET_PAYLOAD_LEN + SIZE_PAYLOAD_LEN]
                .try_into()
                .map_err(|_| ArtifactError::InvalidHeader)?,
        ) as usize;
        let hash = &bytes[OFFSET_HASH..OFFSET_HASH + SIZE_HASH];
        let flags = u32::from_le_bytes(
            bytes[OFFSET_FLAGS..OFFSET_FLAGS + SIZE_FLAGS]
                .try_into()
                .map_err(|_| ArtifactError::InvalidHeader)?,
        );

        if bytes.len() < HEADER_SIZE + payload_len {
            return Err(ArtifactError::InvalidHeader);
        }

        let payload = &bytes[HEADER_SIZE..HEADER_SIZE + payload_len];
        let mut hasher = Hasher::new();
        hasher.update(payload);
        if hasher.finalize().as_bytes() != hash {
            return Err(ArtifactError::HashMismatch);
        }

        let ArtifactPayload {
            metadata,
            dags,
            sections,
        } = bincode::deserialize(payload)?;

        let dags = dags.into_iter().map(|d| d.into_execution_dag()).collect();

        Ok(Self {
            metadata,
            dags,
            sections,
            flags,
        })
    }

    pub fn write_to_path<P: AsRef<Path>>(&self, path: P) -> ArtifactResult<()> {
        let mut file = std::fs::File::create(path)?;
        let bytes = self.to_bytes()?;
        file.write_all(&bytes)?;
        Ok(())
    }

    pub fn read_from_path<P: AsRef<Path>>(path: P) -> ArtifactResult<Self> {
        let mut file = std::fs::File::open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes)
    }
}

pub type ArtifactResult<T> = std::result::Result<T, ArtifactError>;

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::AISOperationType;
    use apxm_core::types::execution::{DagMetadata, ExecutionDag, Node};
    use std::collections::HashMap;

    fn sample_dag() -> ExecutionDag {
        ExecutionDag {
            nodes: vec![Node {
                id: 1,
                op_type: AISOperationType::InvTool,
                attributes: HashMap::new(),
                input_tokens: vec![],
                output_tokens: vec![1],
                metadata: apxm_core::types::NodeMetadata {
                    name: Some("seed".into()),
                    priority: 0,
                    estimated_latency: None,
                    task_source_id: Some(42),
                },
            }],
            edges: vec![],
            entry_nodes: vec![1],
            exit_nodes: vec![1],
            metadata: DagMetadata {
                name: Some("test".into()),
                is_entry: true,
                parameters: vec![],
            },
        }
    }

    #[test]
    fn artifact_round_trip() {
        let dag = sample_dag();
        let metadata = ArtifactMetadata::new(Some("sample".into()), "test-compiler");
        let artifact = Artifact::new(metadata.clone(), vec![dag.clone()]);

        let bytes = artifact.to_bytes().expect("serialize artifact");
        let stored_len = u64::from_le_bytes(bytes[8..16].try_into().unwrap()) as usize;
        assert_eq!(stored_len, bytes.len() - HEADER_SIZE);
        let payload = &bytes[HEADER_SIZE..];
        let expected_payload = bincode::serialize(&ArtifactPayload {
            metadata: metadata.clone(),
            dags: vec![wire::WireDag::from_execution_dag(&dag)],
            sections: Vec::new(),
        })
        .unwrap();
        assert_eq!(payload, expected_payload.as_slice());
        let raw: ArtifactPayload = bincode::deserialize(payload).expect("payload deserialize");
        assert!(raw.metadata.module_name.as_deref() == Some("sample"));
        let decoded = Artifact::from_bytes(&bytes).expect("deserialize artifact");

        assert_eq!(decoded.metadata.module_name, metadata.module_name);
        assert_eq!(decoded.metadata.compiler_version, metadata.compiler_version);
        let decoded_dag = decoded
            .entry_dag()
            .expect("test artifact should have an entry DAG");
        assert_eq!(decoded_dag.nodes.len(), dag.nodes.len());
        assert_eq!(decoded_dag.nodes[0].metadata.task_source_id, Some(42));
        // Note: name is marked #[serde(skip)] so it's not serialized (runtime-only field)
        assert_eq!(decoded_dag.nodes[0].metadata.name, None);
    }

    #[test]
    fn detects_invalid_hash() {
        let dag = sample_dag();
        let metadata = ArtifactMetadata::new(Some("sample".into()), "test-compiler");
        let artifact = Artifact::new(metadata, vec![dag]);
        let mut bytes = artifact.to_bytes().expect("serialize artifact");
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        let err = Artifact::from_bytes(&bytes).expect_err("expected hash mismatch");
        assert!(matches!(err, ArtifactError::HashMismatch));
    }

    #[test]
    fn section_helpers_round_trip_skill_manifest_section() {
        let metadata = ArtifactMetadata::new(Some("sample".into()), "test-compiler");
        let mut artifact = Artifact::new(metadata, vec![sample_dag()]);
        artifact.replace_section(
            section_kinds::SKILL_MANIFEST_V1,
            br#"{"skill_id":"x"}"#.to_vec(),
        );

        let bytes = artifact.to_bytes().expect("serialize artifact");
        let decoded = Artifact::from_bytes(&bytes).expect("deserialize artifact");

        assert_eq!(
            decoded.section_data(section_kinds::SKILL_MANIFEST_V1),
            Some(br#"{"skill_id":"x"}"#.as_slice())
        );
    }

    #[test]
    fn replace_section_updates_existing_section() {
        let metadata = ArtifactMetadata::new(Some("sample".into()), "test-compiler");
        let mut artifact = Artifact::new(metadata, vec![sample_dag()]);
        artifact.replace_section(section_kinds::SKILL_MANIFEST_V1, b"old".to_vec());
        artifact.replace_section(section_kinds::SKILL_MANIFEST_V1, b"new".to_vec());

        assert_eq!(artifact.sections().len(), 1);
        assert_eq!(
            artifact.section_data(section_kinds::SKILL_MANIFEST_V1),
            Some(b"new".as_slice())
        );
    }
}
