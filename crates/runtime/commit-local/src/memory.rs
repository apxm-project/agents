//! In-memory owner-local ExecutionCommitPort.

use std::sync::Mutex;

use apxm_kernel::{
    CommittedContinuation, ExecutionCommitPort, ExecutionCommitRequest, ExecutionCommitResult,
    ProgramInstanceRef,
};
use async_trait::async_trait;
use serde_json::Value;

use crate::store::{
    CommitLocalError, CommitLocalStore, PreparedOutputRef, SessionOutputPreparation,
};

/// Single-writer in-memory commit adapter for owner-local conformance.
pub struct InMemoryExecutionCommit {
    store: Mutex<CommitLocalStore>,
}

impl InMemoryExecutionCommit {
    #[must_use]
    pub fn new() -> Self {
        Self {
            store: Mutex::new(CommitLocalStore::new()),
        }
    }

    /// Failure-injection hook used by owner-local conformance suites.
    pub fn inject_outcome_unknown(&self, commit_id: impl Into<String>) {
        self.store
            .lock()
            .expect("commit-local memory lock")
            .inject_outcome_unknown(commit_id);
    }

    pub fn prepare_output(
        &self,
        preparation: SessionOutputPreparation,
    ) -> Result<PreparedOutputRef, CommitLocalError> {
        self.store
            .lock()
            .expect("commit-local memory lock")
            .prepare_output(preparation)
    }

    pub fn read_output(&self, output_ref: &str) -> Option<Vec<u8>> {
        self.store
            .lock()
            .expect("commit-local memory lock")
            .read_output(output_ref)
    }

    pub fn reclaim_prepared_output(&self, output_ref: &str) -> Result<bool, CommitLocalError> {
        self.store
            .lock()
            .expect("commit-local memory lock")
            .reclaim_prepared_output(output_ref)
    }
}

impl Default for InMemoryExecutionCommit {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ExecutionCommitPort for InMemoryExecutionCommit {
    async fn commit(&self, request: ExecutionCommitRequest) -> ExecutionCommitResult {
        match self
            .store
            .lock()
            .expect("commit-local memory lock")
            .commit(&request)
        {
            Ok(result) => result,
            Err(err) => ExecutionCommitResult::OutcomeUnknown {
                reconciliation_ref: format!("reconcile:error:{err}"),
            },
        }
    }

    async fn current_version(&self, program_instance_ref: &ProgramInstanceRef) -> u64 {
        self.store
            .lock()
            .expect("commit-local memory lock")
            .current_version(program_instance_ref)
    }

    async fn load_continuation(&self, program_instance_ref: &ProgramInstanceRef) -> Option<Value> {
        self.store
            .lock()
            .expect("commit-local memory lock")
            .load_continuation(program_instance_ref)
    }

    async fn load_continuation_with_integrity(
        &self,
        program_instance_ref: &ProgramInstanceRef,
    ) -> Option<CommittedContinuation> {
        self.store
            .lock()
            .expect("commit-local memory lock")
            .load_continuation_with_integrity(program_instance_ref)
    }
}
