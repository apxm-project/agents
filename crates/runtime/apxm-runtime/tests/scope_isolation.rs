//! Integration test for scope_id isolation in SessionManager.
//!
//! Two parallel session writes with the same `session_id` but different
//! `scope_id` values remain isolated — SessionManager retrieves them as
//! independent checkpoints.

use apxm_core::types::goal::{Goal, GoalId, GoalStatus};
use apxm_core::types::values::Value;
use apxm_runtime::aam::GoalTree;
use apxm_runtime::{AamCheckpoint, SessionManager};
use chrono::Utc;
use std::collections::HashMap;

fn checkpoint_with_belief(key: &str, value: &str) -> AamCheckpoint {
    let mut beliefs = HashMap::new();
    beliefs.insert(key.to_string(), Value::String(value.to_string()));

    AamCheckpoint {
        beliefs,
        goals: vec![Goal {
            id: GoalId::new(),
            description: format!("goal-for-{}", key),
            priority: 50,
            status: GoalStatus::Active,
            parent_id: None,
        }],
        capabilities: HashMap::new(),
        goal_tree: GoalTree::new(),
        timestamp: Utc::now(),
    }
}

#[test]
fn scope_isolation_same_session_different_scopes() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = SessionManager::new(dir.path().to_path_buf()).unwrap();

    let session_id = "shared-session";

    // Save two checkpoints under the same session but different scopes
    let cp_scope_a = checkpoint_with_belief("env", "scope-a-data");
    let cp_scope_b = checkpoint_with_belief("env", "scope-b-data");

    mgr.save_checkpoint(session_id, Some("scope-a"), &cp_scope_a)
        .unwrap();
    mgr.save_checkpoint(session_id, Some("scope-b"), &cp_scope_b)
        .unwrap();

    // Load scope-a and verify it has scope-a data
    let loaded_a = mgr
        .load_checkpoint(session_id, Some("scope-a"))
        .unwrap()
        .expect("scope-a checkpoint should exist");
    assert_eq!(
        loaded_a.beliefs.get("env"),
        Some(&Value::String("scope-a-data".to_string()))
    );

    // Load scope-b and verify it has scope-b data
    let loaded_b = mgr
        .load_checkpoint(session_id, Some("scope-b"))
        .unwrap()
        .expect("scope-b checkpoint should exist");
    assert_eq!(
        loaded_b.beliefs.get("env"),
        Some(&Value::String("scope-b-data".to_string()))
    );

    // Loading with no scope returns None (global scope was never written)
    let loaded_global = mgr.load_checkpoint(session_id, None).unwrap();
    assert!(
        loaded_global.is_none(),
        "global scope checkpoint should not exist"
    );
}

#[test]
fn scope_isolation_global_scope_is_independent() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = SessionManager::new(dir.path().to_path_buf()).unwrap();

    let session_id = "my-session";

    // Save to global scope (None) and a named scope
    let cp_global = checkpoint_with_belief("origin", "global");
    let cp_scoped = checkpoint_with_belief("origin", "scoped");

    mgr.save_checkpoint(session_id, None, &cp_global).unwrap();
    mgr.save_checkpoint(session_id, Some("child-scope"), &cp_scoped)
        .unwrap();

    // Each loads independently
    let loaded_global = mgr
        .load_checkpoint(session_id, None)
        .unwrap()
        .expect("global checkpoint should exist");
    assert_eq!(
        loaded_global.beliefs.get("origin"),
        Some(&Value::String("global".to_string()))
    );

    let loaded_scoped = mgr
        .load_checkpoint(session_id, Some("child-scope"))
        .unwrap()
        .expect("scoped checkpoint should exist");
    assert_eq!(
        loaded_scoped.beliefs.get("origin"),
        Some(&Value::String("scoped".to_string()))
    );
}

#[test]
fn scope_isolation_delete_scoped_preserves_global() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = SessionManager::new(dir.path().to_path_buf()).unwrap();

    let session_id = "del-test";

    let cp = checkpoint_with_belief("data", "value");
    mgr.save_checkpoint(session_id, None, &cp).unwrap();
    mgr.save_checkpoint(session_id, Some("temp-scope"), &cp)
        .unwrap();

    // Delete the scoped checkpoint
    mgr.delete_checkpoint(session_id, Some("temp-scope"))
        .unwrap();

    // Scoped is gone
    assert!(mgr
        .load_checkpoint(session_id, Some("temp-scope"))
        .unwrap()
        .is_none());

    // Global is still there
    assert!(mgr.load_checkpoint(session_id, None).unwrap().is_some());
}

#[test]
fn scope_isolation_overwrite_within_scope() {
    let dir = tempfile::tempdir().unwrap();
    let mgr = SessionManager::new(dir.path().to_path_buf()).unwrap();

    let session_id = "overwrite-test";

    let cp1 = checkpoint_with_belief("version", "v1");
    let cp2 = checkpoint_with_belief("version", "v2");

    mgr.save_checkpoint(session_id, Some("scope-x"), &cp1)
        .unwrap();
    mgr.save_checkpoint(session_id, Some("scope-x"), &cp2)
        .unwrap();

    let loaded = mgr
        .load_checkpoint(session_id, Some("scope-x"))
        .unwrap()
        .expect("checkpoint should exist");
    assert_eq!(
        loaded.beliefs.get("version"),
        Some(&Value::String("v2".to_string())),
        "second write should overwrite the first"
    );
}
