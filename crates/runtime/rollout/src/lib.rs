//! Per-thread JSONL rollout transcript storage for the apxm runtime.
//!
//! This crate is the foundational durability layer the runtime builds on top of:
//! every event
//! the in-memory `RunEventBus` records is mirrored here so `/v1/runs/...`
//! survives a restart and the regulatory replay story has a source of
//! truth.

pub mod index;
pub mod line;
pub mod loader;
pub mod paths;
pub mod recorder;

pub use index::{IndexDb, IndexError, ThreadIndexEntry, rebuild_index_from_disk};
pub use line::{
    AssistantMessagePayload, CacheCreationBreakdown, CompactedPayload, ContentBlock,
    EventMsgPayload, RolloutLine, RolloutMeta, RolloutPayload, SessionMetaPayload, SpilledPayload,
    ToolResultPayload, ToolUsePayload, TurnContextPayload, Usage, UserMessagePayload,
};
pub use loader::{ConversationTree, LoadError, LoadStats, load_rollout, reconstruct_history};
pub use paths::{RolloutPaths, blob_path_for, rollout_path_for, subagent_path_for};
pub use recorder::{
    PartialMeta, RolloutRecorder, RolloutRecorderConfig, RolloutWriteError, now_rfc3339,
};

/// Wire-schema version. Bump MAJOR for incompatible changes; readers reject
/// anything older than `MIN_SUPPORTED_VERSION`.
pub const SCHEMA_VERSION: &str = "1.0.0";

/// Anything older than this is rejected by [`load_rollout`] in a future
/// release that introduces a breaking change. Today we accept every 1.x.x.
pub const MIN_SUPPORTED_VERSION: &str = "1.0.0";

/// Default per-line spill threshold (64 KiB). Override via the
/// `APXM_ROLLOUT_SPILL_THRESHOLD_BYTES` env var.
pub const SPILL_THRESHOLD_BYTES: u64 = 65_536;
