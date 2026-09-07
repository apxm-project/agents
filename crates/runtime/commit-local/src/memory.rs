//! In-memory owner-local ExecutionCommitPort.

use std::sync::{Arc, Mutex};

use apxm_kernel::{
    CommittedContinuation, ExecutionCommitPort, ExecutionCommitRequest, ExecutionCommitResult,
    PreparedSessionOutputRef, ProgramInstanceRef,
};
use apxm_runtime_protocol::{
    ContentReadResult, ContentRef, ExecutionObservation, ExecutionReadRequest, ExecutionReadResult,
    ReadContext,
};
use async_trait::async_trait;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::store::{
    CommitLocalError, CommitLocalStore, DenyReadAccess, PreparedOutputRef, ReadAccessHook,
    SessionOutputPreparation,
};

/// Single-writer in-memory commit adapter for owner-local conformance.
pub struct InMemoryExecutionCommit {
    store: Mutex<CommitLocalStore>,
    auth_key: [u8; 32],
    read_hook: Mutex<Arc<dyn ReadAccessHook>>,
}

impl InMemoryExecutionCommit {
    #[must_use]
    pub fn new() -> Self {
        Self {
            store: Mutex::new(CommitLocalStore::new()),
            auth_key: Sha256::digest(b"apxm-commit-local-memory-v2").into(),
            read_hook: Mutex::new(Arc::new(DenyReadAccess)),
        }
    }

    /// Construct an in-memory adapter with a composition-owned read hook.
    #[must_use]
    pub fn with_read_access_hook(hook: Arc<dyn ReadAccessHook>) -> Self {
        Self {
            store: Mutex::new(CommitLocalStore::new()),
            auth_key: Sha256::digest(b"apxm-commit-local-memory-v2").into(),
            read_hook: Mutex::new(hook),
        }
    }

    /// Replace only the authorization binding; durable in-memory state is
    /// intentionally preserved when a composition root is assembled late.
    pub fn set_read_access_hook(&self, hook: Arc<dyn ReadAccessHook>) {
        *self.read_hook.lock().expect("memory read hook lock") = hook;
    }

    /// Bind the composition-owned scope to outputs staged by this adapter.
    pub fn set_default_access_scope_ref(&self, reference: String) {
        self.store
            .lock()
            .expect("commit-local memory lock")
            .set_default_access_scope_ref(reference);
    }

    /// Read opaque composition metadata from the in-memory store.
    #[must_use]
    pub fn runtime_metadata(&self) -> Option<Value> {
        self.store
            .lock()
            .expect("commit-local memory lock")
            .runtime_metadata()
    }

    /// Replace opaque composition metadata in the in-memory store.
    pub fn set_runtime_metadata(&self, metadata: Option<Value>) {
        self.store
            .lock()
            .expect("commit-local memory lock")
            .set_runtime_metadata(metadata);
    }

    #[must_use]
    pub fn invocation_status(
        &self,
        invocation: &str,
    ) -> Option<apxm_runtime_protocol::ProgramInvocationStatus> {
        self.store
            .lock()
            .expect("commit-local memory lock")
            .invocation_status(invocation)
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

    /// Read observations, inspection, committed content, or evidence through
    /// the typed scope-bound contract.
    pub fn read_execution(
        &self,
        request: ExecutionReadRequest,
    ) -> Result<ExecutionReadResult, CommitLocalError> {
        self.read_execution_with_live(request, &[])
    }

    /// Read with a bounded process-local live observation overlay.
    pub fn read_execution_with_live(
        &self,
        request: ExecutionReadRequest,
        live_observations: &[ExecutionObservation],
    ) -> Result<ExecutionReadResult, CommitLocalError> {
        let hook = self.read_hook.lock().expect("memory read hook lock");
        self.store
            .lock()
            .expect("commit-local memory lock")
            .read_execution_with_live(request, &self.auth_key, hook.as_ref(), live_observations)
    }

    /// Read one committed output with a scope-bound typed context.
    pub fn read_committed_output(
        &self,
        context: ReadContext,
        content_ref: ContentRef,
    ) -> Result<ContentReadResult, CommitLocalError> {
        match self.read_execution(ExecutionReadRequest::ContentRead {
            context,
            content_ref,
        })? {
            ExecutionReadResult::Content { content } => Ok(content),
            _ => Err(CommitLocalError::InvalidRead(
                "content read returned the wrong result kind".into(),
            )),
        }
    }

    /// Read one committed output through the distinct output.read operation.
    pub fn read_output(
        &self,
        context: ReadContext,
        output_ref: apxm_runtime_protocol::OutputRef,
    ) -> Result<ContentReadResult, CommitLocalError> {
        match self.read_execution(ExecutionReadRequest::OutputRead {
            context,
            output_ref,
        })? {
            ExecutionReadResult::Output { output } => Ok(output),
            _ => Err(CommitLocalError::InvalidRead(
                "output read returned the wrong result kind".into(),
            )),
        }
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
    async fn prepare_output(
        &self,
        preparation: apxm_kernel::SessionOutputPreparation,
    ) -> Result<PreparedSessionOutputRef, String> {
        let preparation = SessionOutputPreparation::from_kernel(preparation)
            .map_err(|error| error.to_string())?;
        let prepared = self
            .prepare_output(preparation)
            .map_err(|error| error.to_string())?
            .to_kernel();
        prepared.validate().map_err(str::to_owned)?;
        Ok(prepared)
    }

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
