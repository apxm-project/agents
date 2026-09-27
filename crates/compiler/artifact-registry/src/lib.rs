//! Shared, owner-local reference authority for digest-addressed artifacts.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use apxm_program::ExecutableArtifact;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const SCHEMA: &str = "apxm.artifact-registry.v1";
const MAX_REGISTRY_BYTES: u64 = 8 * 1024 * 1024;
const MAX_REFS_PER_ARTIFACT: usize = 10_000;
const MAX_INVENTORY_ENTRIES: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disposition {
    pub artifact_digest: String,
    pub state: String,
    pub live_references: u32,
    pub runtime_references: u32,
    pub unmanaged: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Reference {
    claim: String,
    released: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ArtifactEntry {
    external: BTreeMap<String, Reference>,
    runtime: BTreeMap<String, Reference>,
    managed: bool,
    purged: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegistryState {
    schema_version: String,
    entries: BTreeMap<String, ArtifactEntry>,
}

impl Default for RegistryState {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA.to_owned(),
            entries: BTreeMap::new(),
        }
    }
}

/// Both compiler and Runtime use this same artifact-directory lock.
pub struct ArtifactRegistry {
    root: PathBuf,
}

impl ArtifactRegistry {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn with_state<T>(
        &self,
        mutate: impl FnOnce(&mut RegistryState) -> Result<(T, bool), String>,
    ) -> Result<T, String> {
        fs::create_dir_all(&self.root).map_err(|error| error.to_string())?;
        reject_symlink(&self.root)?;
        let lock_path = self.root.join("artifact-registry.v1.lock");
        reject_symlink_if_present(&lock_path)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(lock_path)
            .map_err(|error| error.to_string())?;
        lock.lock_exclusive().map_err(|error| error.to_string())?;
        let mut state = self.load()?;
        let (result, changed) = mutate(&mut state)?;
        if changed {
            self.persist(&state)?;
        }
        Ok(result)
    }

    fn load(&self) -> Result<RegistryState, String> {
        let path = self.root.join("artifact-registry.v1.json");
        reject_symlink_if_present(&path)?;
        let file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(RegistryState::default());
            }
            Err(error) => return Err(error.to_string()),
        };
        if file.metadata().map_err(|error| error.to_string())?.len() > MAX_REGISTRY_BYTES {
            return Err("artifact registry exceeds its size bound".to_owned());
        }
        let mut bytes = Vec::new();
        file.take(MAX_REGISTRY_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_REGISTRY_BYTES {
            return Err("artifact registry exceeds its size bound".to_owned());
        }
        let state: RegistryState =
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
        if state.schema_version != SCHEMA {
            return Err("artifact registry schema mismatch".to_owned());
        }
        Ok(state)
    }

    fn persist(&self, state: &RegistryState) -> Result<(), String> {
        let bytes = serde_json::to_vec(state).map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_REGISTRY_BYTES {
            return Err("artifact registry exceeds its size bound".to_owned());
        }
        let temporary = self
            .root
            .join(format!(".artifact-registry-{}", Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| error.to_string())?;
        }
        let result = (|| {
            file.write_all(&bytes).map_err(|error| error.to_string())?;
            file.sync_all().map_err(|error| error.to_string())?;
            fs::rename(&temporary, self.root.join("artifact-registry.v1.json"))
                .map_err(|error| error.to_string())?;
            File::open(&self.root)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| error.to_string())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    fn blob_path(&self, digest: &str) -> Result<PathBuf, String> {
        if !apxm_core::grammar::is_digest(digest) {
            return Err("invalid artifact digest".to_owned());
        }
        Ok(self.root.join(digest.replace(':', "-")))
    }

    fn blob_exists(&self, digest: &str) -> Result<bool, String> {
        let path = self.blob_path(digest)?;
        match fs::symlink_metadata(path) {
            Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => Ok(true),
            Ok(_) => Err("artifact path is not a regular file".to_owned()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.to_string()),
        }
    }

    /// Commit a blob under the same lock that governs reference collection.
    pub fn publish(&self, digest: &str, bytes: &[u8]) -> Result<(), String> {
        ExecutableArtifact::decode_for_execution(bytes, digest)
            .map_err(|_| "artifact digest mismatch")?;
        self.with_state(|state| {
            let path = self.blob_path(digest)?;
            if self.blob_exists(digest)? {
                if fs::read(&path).map_err(|error| error.to_string())? != bytes {
                    return Err("artifact path contains different bytes".to_owned());
                }
            } else {
                let mut file = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&path)
                    .map_err(|error| error.to_string())?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    file.set_permissions(fs::Permissions::from_mode(0o600))
                        .map_err(|error| error.to_string())?;
                }
                file.write_all(bytes).map_err(|error| error.to_string())?;
                file.sync_all().map_err(|error| error.to_string())?;
            }
            let entry = state.entries.entry(digest.to_owned()).or_default();
            if entry.purged {
                *entry = ArtifactEntry::default();
            }
            Ok(((), true))
        })
    }

    fn require_blob(&self, digest: &str) -> Result<(), String> {
        if self.blob_exists(digest)? {
            Ok(())
        } else {
            Err("unknown_artifact".to_owned())
        }
    }

    /// Retain a global external reference; exact retry returns its first claim.
    pub fn retain(
        &self,
        digest: &str,
        reference_id: &str,
    ) -> Result<(String, Disposition), String> {
        validate_reference(reference_id)?;
        self.with_state(|state| {
            self.require_blob(digest)?;
            let entry = state.entries.entry(digest.to_owned()).or_default();
            if entry.purged {
                return Err("artifact_purged".to_owned());
            }
            if let Some(prior) = entry.external.get(reference_id) {
                if prior.released {
                    return Err("reference_released".to_owned());
                }
                let claim = prior.claim.clone();
                return Ok(((claim, disposition(digest, entry)), false));
            }
            if entry.external.len() >= MAX_REFS_PER_ARTIFACT {
                return Err("artifact_reference_capacity_exhausted".to_owned());
            }
            let claim = format!("artifact-owner-{}", Uuid::new_v4());
            entry.external.insert(
                reference_id.to_owned(),
                Reference {
                    claim: claim.clone(),
                    released: false,
                },
            );
            Ok(((claim, disposition(digest, entry)), true))
        })
    }

    pub fn release(
        &self,
        digest: &str,
        reference_id: &str,
        claim: &str,
    ) -> Result<Disposition, String> {
        validate_reference(reference_id)?;
        self.with_state(|state| {
            let entry = state.entries.get_mut(digest).ok_or("unknown_artifact")?;
            let reference = entry
                .external
                .get_mut(reference_id)
                .ok_or("unknown_reference")?;
            if reference.claim != claim {
                return Err("owner_mismatch".to_owned());
            }
            let changed = !reference.released;
            reference.released = true;
            Ok((disposition(digest, entry), changed))
        })
    }

    /// Runtime registers before persisting an instance; crash orphans block collection.
    pub fn retain_runtime(
        &self,
        digest: &str,
        instance_id: &str,
        claim: &str,
    ) -> Result<(), String> {
        validate_reference(instance_id)?;
        self.with_state(|state| {
            self.require_blob(digest)?;
            let entry = state.entries.entry(digest.to_owned()).or_default();
            if entry.purged {
                return Err("artifact_purged".to_owned());
            }
            let claim_digest = format!("sha256:{:x}", Sha256::digest(claim.as_bytes()));
            if let Some(prior) = entry.runtime.get(instance_id) {
                return if prior.claim == claim_digest && !prior.released {
                    Ok(((), false))
                } else {
                    Err("runtime_reference_conflict".to_owned())
                };
            }
            if entry.runtime.len() >= MAX_REFS_PER_ARTIFACT {
                return Err("artifact_reference_capacity_exhausted".to_owned());
            }
            entry.runtime.insert(
                instance_id.to_owned(),
                Reference {
                    claim: claim_digest,
                    released: false,
                },
            );
            Ok(((), true))
        })
    }

    /// Runtime releases only after the instance tombstone and payload purge commit.
    pub fn release_runtime(
        &self,
        digest: &str,
        instance_id: &str,
        claim: &str,
    ) -> Result<(), String> {
        self.with_state(|state| {
            let entry = state.entries.get_mut(digest).ok_or("unknown_artifact")?;
            let reference = entry
                .runtime
                .get_mut(instance_id)
                .ok_or("unknown_reference")?;
            let claim_digest = format!("sha256:{:x}", Sha256::digest(claim.as_bytes()));
            if reference.claim != claim_digest {
                return Err("owner_mismatch".to_owned());
            }
            let changed = !reference.released;
            reference.released = true;
            Ok(((), changed))
        })
    }

    /// A trusted publisher supplies one complete global snapshot of its refs.
    /// The registry verifies exact equality; it never treats one owner as all owners.
    pub fn seal_inventory(
        &self,
        digest: &str,
        reference_ids: &[String],
    ) -> Result<Disposition, String> {
        if reference_ids.len() > MAX_REFS_PER_ARTIFACT {
            return Err("artifact_reference_capacity_exhausted".to_owned());
        }
        let supplied = reference_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        if supplied.len() != reference_ids.len() {
            return Err("duplicate_reference".to_owned());
        }
        for reference in reference_ids {
            validate_reference(reference)?;
        }
        self.with_state(|state| {
            self.require_blob(digest)?;
            let entry = state.entries.entry(digest.to_owned()).or_default();
            let registered = entry
                .external
                .iter()
                .filter(|(_, value)| !value.released)
                .map(|(id, _)| id.as_str())
                .collect::<BTreeSet<_>>();
            if supplied != registered {
                return Err("inventory_mismatch".to_owned());
            }
            let changed = !entry.managed;
            entry.managed = true;
            Ok((disposition(digest, entry), changed))
        })
    }

    pub fn inspect(&self, digest: &str) -> Result<Disposition, String> {
        self.with_state(|state| {
            let exists = self.blob_exists(digest)?;
            let entry = state.entries.get(digest);
            if !exists && entry.is_none() {
                return Err("unknown_artifact".to_owned());
            }
            Ok((
                entry.map_or_else(
                    || Disposition {
                        artifact_digest: digest.to_owned(),
                        state: "retained".to_owned(),
                        live_references: 0,
                        runtime_references: 0,
                        unmanaged: true,
                    },
                    |value| disposition(digest, value),
                ),
                false,
            ))
        })
    }

    /// Enumerate every registry entry and digest-shaped blob under one lock.
    /// Unregistered legacy blobs remain unmanaged and cannot be collected.
    pub fn inventory(&self) -> Result<Vec<Disposition>, String> {
        self.with_state(|state| {
            let mut digests = state.entries.keys().cloned().collect::<BTreeSet<_>>();
            for child in fs::read_dir(&self.root).map_err(|error| error.to_string())? {
                let child = child.map_err(|error| error.to_string())?;
                let name = child.file_name().to_string_lossy().into_owned();
                let Some(suffix) = name.strip_prefix("sha256-") else {
                    continue;
                };
                let digest = format!("sha256:{suffix}");
                if !apxm_core::grammar::is_digest(&digest) {
                    continue;
                }
                if !child
                    .file_type()
                    .map_err(|error| error.to_string())?
                    .is_file()
                {
                    return Err("artifact path is not a regular file".to_owned());
                }
                digests.insert(digest);
                if digests.len() > MAX_INVENTORY_ENTRIES {
                    return Err("artifact inventory exceeds its size bound".to_owned());
                }
            }
            if digests.len() > MAX_INVENTORY_ENTRIES {
                return Err("artifact inventory exceeds its size bound".to_owned());
            }
            let result = digests
                .into_iter()
                .map(|digest| {
                    state.entries.get(&digest).map_or_else(
                        || Disposition {
                            artifact_digest: digest.clone(),
                            state: "retained".to_owned(),
                            live_references: 0,
                            runtime_references: 0,
                            unmanaged: true,
                        },
                        |entry| disposition(&digest, entry),
                    )
                })
                .collect();
            Ok((result, false))
        })
    }

    /// Only a sealed, globally unreferenced blob may be unlinked.
    pub fn collect(
        &self,
        digest: &str,
        runtime_live_instances: u32,
    ) -> Result<Disposition, String> {
        self.with_state(|state| {
            let Some(entry) = state.entries.get_mut(digest) else {
                if self.blob_exists(digest)? {
                    return Ok((
                        Disposition {
                            artifact_digest: digest.to_owned(),
                            state: "retained".to_owned(),
                            live_references: 0,
                            runtime_references: 0,
                            unmanaged: true,
                        },
                        false,
                    ));
                }
                return Err("unknown_artifact".to_owned());
            };
            let before = disposition(digest, entry);
            if before.unmanaged
                || before.live_references != 0
                || before.runtime_references != 0
                || runtime_live_instances != 0
            {
                return Ok((before, false));
            }
            let path = self.blob_path(digest)?;
            if self.blob_exists(digest)? {
                fs::remove_file(path).map_err(|error| error.to_string())?;
            }
            let changed = !entry.purged;
            entry.purged = true;
            Ok((disposition(digest, entry), changed))
        })
    }
}

fn disposition(digest: &str, entry: &ArtifactEntry) -> Disposition {
    Disposition {
        artifact_digest: digest.to_owned(),
        state: if entry.purged { "purged" } else { "retained" }.to_owned(),
        live_references: entry
            .external
            .values()
            .filter(|value| !value.released)
            .count() as u32,
        runtime_references: entry
            .runtime
            .values()
            .filter(|value| !value.released)
            .count() as u32,
        unmanaged: !entry.managed,
    }
}

fn validate_reference(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 256
        || value.bytes().any(|byte| !(0x21..=0x7e).contains(&byte))
    {
        return Err("invalid reference id".to_owned());
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("artifact directory is not real".to_owned());
    }
    Ok(())
}

fn reject_symlink_if_present(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            Err("artifact registry path is a symlink".to_owned())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, ArtifactRegistry, String) {
        let dir = tempfile::tempdir().expect("temporary artifact directory");
        let digest = format!("sha256:{}", "a".repeat(64));
        fs::write(dir.path().join(digest.replace(':', "-")), b"legacy blob").expect("fixture blob");
        let registry = ArtifactRegistry::new(dir.path().to_path_buf());
        (dir, registry, digest)
    }

    #[test]
    fn unmanaged_legacy_blob_is_enumerated_and_cannot_be_collected() {
        let (_dir, registry, digest) = fixture();
        assert_eq!(
            registry.inventory().unwrap(),
            vec![Disposition {
                artifact_digest: digest.clone(),
                state: "retained".to_owned(),
                live_references: 0,
                runtime_references: 0,
                unmanaged: true,
            }]
        );
        assert_eq!(registry.collect(&digest, 0).unwrap().state, "retained");
        assert!(
            registry
                .seal_inventory(&digest, &["missing".to_owned()])
                .is_err()
        );
        assert!(registry.blob_exists(&digest).unwrap());
    }

    #[test]
    fn exact_global_inventory_and_runtime_claim_preserve_shared_blob() {
        let (_dir, registry, digest) = fixture();
        let (a, _) = registry.retain(&digest, "owner-a").unwrap();
        let (b, _) = registry.retain(&digest, "owner-b").unwrap();
        assert_eq!(registry.retain(&digest, "owner-a").unwrap().0, a);
        assert!(
            registry
                .seal_inventory(&digest, &["owner-a".to_owned()])
                .is_err()
        );
        registry
            .seal_inventory(&digest, &["owner-a".to_owned(), "owner-b".to_owned()])
            .unwrap();
        registry
            .retain_runtime(&digest, "instance-b", "runtime-claim-b")
            .unwrap();
        registry.release(&digest, "owner-a", &a).unwrap();
        assert_eq!(registry.collect(&digest, 0).unwrap().live_references, 1);
        assert!(registry.blob_exists(&digest).unwrap());
        registry.release(&digest, "owner-b", &b).unwrap();
        assert_eq!(registry.collect(&digest, 0).unwrap().runtime_references, 1);
        registry
            .release_runtime(&digest, "instance-b", "runtime-claim-b")
            .unwrap();
        assert_eq!(registry.collect(&digest, 1).unwrap().state, "retained");
        assert_eq!(registry.collect(&digest, 0).unwrap().state, "purged");
        assert!(!registry.blob_exists(&digest).unwrap());
        assert_eq!(registry.inspect(&digest).unwrap().state, "purged");
        assert!(registry.retain(&digest, "owner-c").is_err());
    }

    #[test]
    fn wrong_claim_and_unknown_runtime_reference_fail_closed() {
        let (_dir, registry, digest) = fixture();
        let (a, _) = registry.retain(&digest, "owner-a").unwrap();
        assert_eq!(
            registry.release(&digest, "owner-a", "wrong").unwrap_err(),
            "owner_mismatch"
        );
        assert_eq!(
            registry
                .release_runtime(&digest, "missing", "claim")
                .unwrap_err(),
            "unknown_reference"
        );
        assert_eq!(registry.inspect(&digest).unwrap().live_references, 1);
        registry.release(&digest, "owner-a", &a).unwrap();
    }
}
