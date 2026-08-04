//! Per-activation readiness conformance.
//!
//! Every case here proves one clause of the readiness transition contract: the
//! closed lifecycle, the claim-before-write ordering, the single path into
//! `runnable`, the closed cancellation race, and the activity-credit accounting
//! that backs local quiescence.

use apxm_execution::{
    NodeLifecycle, NodeOccurrenceId, OperandSlotDecl, OperandValue, ReadinessError,
    ReadinessKernel, ReadinessKernelBuilder, ReadinessTransition,
};

fn node(id: &str) -> NodeOccurrenceId {
    NodeOccurrenceId::new(id).expect("node occurrence identity")
}

fn slot(name: &str, type_ref: &str) -> OperandSlotDecl {
    OperandSlotDecl {
        slot: name.to_string(),
        type_ref: type_ref.to_string(),
    }
}

fn value(value_id: &str, type_ref: &str) -> OperandValue {
    OperandValue {
        value_id: value_id.to_string(),
        type_ref: type_ref.to_string(),
    }
}

/// Publish `slot` on `target` through the claim protocol.
fn publish(
    kernel: &mut ReadinessKernel,
    target: &NodeOccurrenceId,
    slot_name: &str,
    type_ref: &str,
) {
    let claim = kernel.claim_slot(target, slot_name).expect("slot claim");
    kernel
        .publish_operand(&claim, value("value.1", type_ref))
        .expect("operand publication");
}

/// A -> C and B -> C, where C declares one typed slot per predecessor.
fn diamond_join() -> ReadinessKernel {
    ReadinessKernelBuilder::new()
        .node(node("a"), Vec::new(), vec![node("c")])
        .node(node("b"), Vec::new(), vec![node("c")])
        .node(
            node("c"),
            vec![slot("left", "apxm.Text"), slot("right", "apxm.Text")],
            Vec::new(),
        )
        .build()
        .expect("diamond join kernel")
}

#[test]
fn empty_node_occurrence_identity_is_refused() {
    assert_eq!(
        NodeOccurrenceId::new("  "),
        Err(ReadinessError::EmptyNodeOccurrenceId)
    );
}

#[test]
fn predecessor_counts_are_derived_from_declared_edges() {
    let kernel = diamond_join();
    assert_eq!(kernel.remaining_predecessors(&node("a")), Ok(0));
    assert_eq!(kernel.remaining_predecessors(&node("b")), Ok(0));
    assert_eq!(kernel.remaining_predecessors(&node("c")), Ok(2));
    assert_eq!(kernel.zero_predecessor_nodes(), vec![node("a"), node("b")]);
}

#[test]
fn a_successor_edge_to_an_undeclared_node_is_refused_at_build() {
    let error = ReadinessKernelBuilder::new()
        .node(node("a"), Vec::new(), vec![node("ghost")])
        .build()
        .expect_err("unknown successor");
    assert_eq!(
        error,
        ReadinessError::UnknownSuccessor {
            node: node("a"),
            successor: node("ghost"),
        }
    );
}

#[test]
fn a_duplicate_node_occurrence_is_refused_at_build() {
    let error = ReadinessKernelBuilder::new()
        .node(node("a"), Vec::new(), Vec::new())
        .node(node("a"), Vec::new(), Vec::new())
        .build()
        .expect_err("duplicate occurrence");
    assert_eq!(
        error,
        ReadinessError::DuplicateNodeOccurrence { node: node("a") }
    );
}

#[test]
fn a_duplicate_operand_slot_is_refused_at_build() {
    let error = ReadinessKernelBuilder::new()
        .node(
            node("a"),
            vec![slot("in", "apxm.Text"), slot("in", "apxm.Text")],
            Vec::new(),
        )
        .build()
        .expect_err("duplicate slot");
    assert_eq!(
        error,
        ReadinessError::DuplicateOperandSlot {
            node: node("a"),
            slot: "in".to_string(),
        }
    );
}

#[test]
fn only_the_last_predecessor_release_makes_a_node_runnable() {
    let mut kernel = diamond_join();
    publish(&mut kernel, &node("c"), "left", "apxm.Text");
    publish(&mut kernel, &node("c"), "right", "apxm.Text");

    kernel.admit(&node("a")).expect("admit a");
    kernel.start(&node("a")).expect("start a");
    let first = kernel.complete(&node("a")).expect("complete a");
    assert_eq!(
        first,
        vec![ReadinessTransition::StillBlocked {
            node: node("c"),
            remaining: 1,
        }]
    );
    assert_eq!(kernel.lifecycle(&node("c")), Ok(NodeLifecycle::Blocked));

    kernel.admit(&node("b")).expect("admit b");
    kernel.start(&node("b")).expect("start b");
    let second = kernel.complete(&node("b")).expect("complete b");
    assert_eq!(
        second,
        vec![ReadinessTransition::BecameRunnable { node: node("c") }]
    );
    assert_eq!(kernel.lifecycle(&node("c")), Ok(NodeLifecycle::Runnable));
}

#[test]
fn explicit_admission_is_refused_for_a_node_that_still_has_predecessors() {
    let mut kernel = diamond_join();
    let error = kernel.admit(&node("c")).expect_err("admission refused");
    assert_eq!(
        error,
        ReadinessError::AdmissionRequiresZeroPredecessors {
            node: node("c"),
            remaining: 2,
        }
    );
    assert_eq!(kernel.lifecycle(&node("c")), Ok(NodeLifecycle::Blocked));
    assert_eq!(kernel.activity_credits(), 0);
}

#[test]
fn a_refused_admission_leaks_no_activity_credit() {
    let mut kernel = diamond_join();
    for _ in 0..8 {
        assert!(kernel.admit(&node("c")).is_err());
    }
    assert!(kernel.is_locally_quiescent());
}

#[test]
fn a_decrement_with_no_remaining_predecessor_underflows_closed() {
    let mut kernel = diamond_join();
    let error = kernel
        .release_predecessor(&node("a"))
        .expect_err("underflow");
    assert_eq!(
        error,
        ReadinessError::PredecessorUnderflow { node: node("a") }
    );
    assert_eq!(kernel.remaining_predecessors(&node("a")), Ok(0));
    assert!(kernel.is_locally_quiescent());
}

#[test]
fn a_second_writer_on_the_same_slot_is_refused_before_it_can_publish() {
    let mut kernel = diamond_join();
    let first = kernel.claim_slot(&node("c"), "left").expect("first claim");
    let error = kernel
        .claim_slot(&node("c"), "left")
        .expect_err("second claim");
    assert_eq!(
        error,
        ReadinessError::SlotAlreadyClaimed {
            node: node("c"),
            slot: "left".to_string(),
        }
    );
    kernel
        .publish_operand(&first, value("value.1", "apxm.Text"))
        .expect("winner publishes");
}

#[test]
fn a_claim_after_publication_is_refused() {
    let mut kernel = diamond_join();
    publish(&mut kernel, &node("c"), "left", "apxm.Text");
    let error = kernel
        .claim_slot(&node("c"), "left")
        .expect_err("late claim");
    assert_eq!(
        error,
        ReadinessError::SlotAlreadyPublished {
            node: node("c"),
            slot: "left".to_string(),
        }
    );
}

#[test]
fn a_stale_claim_from_a_superseded_attempt_cannot_overwrite_a_live_slot() {
    let mut kernel = diamond_join();
    let stale = kernel.claim_slot(&node("c"), "left").expect("first claim");
    kernel
        .publish_operand(&stale, value("value.1", "apxm.Text"))
        .expect("first publication");

    // A superseded attempt replays its token against the same slot.
    let error = kernel
        .publish_operand(&stale, value("value.2", "apxm.Text"))
        .expect_err("stale replay");
    assert_eq!(
        error,
        ReadinessError::SlotAlreadyPublished {
            node: node("c"),
            slot: "left".to_string(),
        }
    );
}

#[test]
fn publication_against_an_unclaimed_slot_is_a_stale_claim() {
    let mut kernel = diamond_join();
    let claim = kernel.claim_slot(&node("c"), "left").expect("claim left");
    // The token names `left`; `right` was never claimed by anyone.
    let forged = kernel.claim_slot(&node("c"), "right").expect("claim right");
    drop(claim);
    let mut other = diamond_join();
    let error = other
        .publish_operand(&forged, value("value.1", "apxm.Text"))
        .expect_err("unclaimed target");
    assert_eq!(
        error,
        ReadinessError::StaleSlotClaim {
            node: node("c"),
            slot: "right".to_string(),
            presented: forged.generation(),
            live: None,
        }
    );
}

#[test]
fn a_type_mismatched_publication_is_refused_and_leaves_the_slot_claimed() {
    let mut kernel = diamond_join();
    let claim = kernel.claim_slot(&node("c"), "left").expect("claim");
    let error = kernel
        .publish_operand(&claim, value("value.1", "apxm.Bytes"))
        .expect_err("type mismatch");
    assert_eq!(
        error,
        ReadinessError::OperandTypeMismatch {
            node: node("c"),
            slot: "left".to_string(),
            declared: "apxm.Text".to_string(),
            published: "apxm.Bytes".to_string(),
        }
    );
    // The slot is still claimed by the same token, not published and not reopened.
    assert_eq!(
        kernel.claim_slot(&node("c"), "left"),
        Err(ReadinessError::SlotAlreadyClaimed {
            node: node("c"),
            slot: "left".to_string(),
        })
    );
}

#[test]
fn a_last_release_with_an_unpublished_slot_fails_closed_without_manufacturing_an_operand() {
    let mut kernel = ReadinessKernelBuilder::new()
        .node(node("a"), Vec::new(), vec![node("b")])
        .node(node("b"), vec![slot("in", "apxm.Text")], Vec::new())
        .build()
        .expect("kernel");

    kernel.admit(&node("a")).expect("admit a");
    kernel.start(&node("a")).expect("start a");
    let error = kernel
        .complete(&node("a"))
        .expect_err("operands incomplete");
    assert_eq!(
        error,
        ReadinessError::OperandsIncomplete {
            node: node("b"),
            slot: "in".to_string(),
        }
    );
    assert_eq!(kernel.lifecycle(&node("b")), Ok(NodeLifecycle::Blocked));
    // The predecessor count is untouched, so the exact publication that was
    // missing can still arrive and release the node.
    assert_eq!(kernel.remaining_predecessors(&node("b")), Ok(1));
    publish(&mut kernel, &node("b"), "in", "apxm.Text");
    assert_eq!(
        kernel.release_predecessor(&node("b")),
        Ok(ReadinessTransition::BecameRunnable { node: node("b") })
    );
    // The completed predecessor released its own credit even though its
    // fan-out was refused, so only the newly runnable node holds one.
    assert_eq!(kernel.activity_credits(), 1);
}

#[test]
fn a_runnable_node_carries_exactly_its_published_operands() {
    let mut kernel = diamond_join();
    publish(&mut kernel, &node("c"), "left", "apxm.Text");
    publish(&mut kernel, &node("c"), "right", "apxm.Text");
    for predecessor in ["a", "b"] {
        kernel.admit(&node(predecessor)).expect("admit");
        kernel.start(&node(predecessor)).expect("start");
        kernel.complete(&node(predecessor)).expect("complete");
    }

    let runnable = kernel.runnable_node(&node("c")).expect("runnable value");
    assert_eq!(runnable.node, node("c"));
    assert_eq!(
        runnable.operands.keys().cloned().collect::<Vec<_>>(),
        vec!["left".to_string(), "right".to_string()]
    );
    assert_eq!(runnable.operands["left"].type_ref, "apxm.Text");
}

#[test]
fn a_blocked_node_cannot_be_materialized_as_runnable() {
    let kernel = diamond_join();
    let error = kernel.runnable_node(&node("c")).expect_err("not runnable");
    assert_eq!(
        error,
        ReadinessError::IllegalTransition {
            node: node("c"),
            from: NodeLifecycle::Blocked,
            to: NodeLifecycle::Runnable,
        }
    );
}

#[test]
fn cancellation_racing_the_last_predecessor_keeps_the_node_cancelled() {
    let mut kernel = diamond_join();
    publish(&mut kernel, &node("c"), "left", "apxm.Text");
    publish(&mut kernel, &node("c"), "right", "apxm.Text");

    kernel.admit(&node("a")).expect("admit a");
    kernel.start(&node("a")).expect("start a");
    kernel.complete(&node("a")).expect("complete a");

    // Cancellation wins from blocked before the last predecessor lands.
    kernel.cancel(&node("c")).expect("cancel c");

    kernel.admit(&node("b")).expect("admit b");
    kernel.start(&node("b")).expect("start b");
    let transitions = kernel.complete(&node("b")).expect("complete b");
    assert_eq!(
        transitions,
        vec![ReadinessTransition::AlreadyTerminal {
            node: node("c"),
            lifecycle: NodeLifecycle::Cancelled,
        }]
    );
    assert_eq!(kernel.lifecycle(&node("c")), Ok(NodeLifecycle::Cancelled));
    // Nothing was resurrected and nothing is still queued.
    assert!(kernel.is_locally_quiescent());
}

#[test]
fn failure_racing_the_last_predecessor_keeps_the_node_failed() {
    let mut kernel = diamond_join();
    publish(&mut kernel, &node("c"), "left", "apxm.Text");
    publish(&mut kernel, &node("c"), "right", "apxm.Text");
    for predecessor in ["a", "b"] {
        kernel.admit(&node(predecessor)).expect("admit");
        kernel.start(&node(predecessor)).expect("start");
        if predecessor == "b" {
            kernel.fail(&node("c")).expect("fail c");
        }
        kernel.complete(&node(predecessor)).expect("complete");
    }
    assert_eq!(kernel.lifecycle(&node("c")), Ok(NodeLifecycle::Failed));
    assert!(kernel.is_locally_quiescent());
}

#[test]
fn a_terminal_node_cannot_be_cancelled_or_failed_again() {
    let mut kernel = ReadinessKernelBuilder::new()
        .node(node("a"), Vec::new(), Vec::new())
        .build()
        .expect("kernel");
    kernel.admit(&node("a")).expect("admit");
    kernel.cancel(&node("a")).expect("cancel");
    assert_eq!(
        kernel.cancel(&node("a")),
        Err(ReadinessError::IllegalTransition {
            node: node("a"),
            from: NodeLifecycle::Cancelled,
            to: NodeLifecycle::Cancelled,
        })
    );
    assert_eq!(
        kernel.fail(&node("a")),
        Err(ReadinessError::IllegalTransition {
            node: node("a"),
            from: NodeLifecycle::Cancelled,
            to: NodeLifecycle::Failed,
        })
    );
}

#[test]
fn cancelling_a_node_never_releases_its_successors() {
    let mut kernel = diamond_join();
    publish(&mut kernel, &node("c"), "left", "apxm.Text");
    publish(&mut kernel, &node("c"), "right", "apxm.Text");
    kernel.cancel(&node("a")).expect("cancel a");
    assert_eq!(kernel.remaining_predecessors(&node("c")), Ok(2));
    assert_eq!(kernel.lifecycle(&node("c")), Ok(NodeLifecycle::Blocked));
}

#[test]
fn a_node_cannot_start_twice_or_start_from_blocked() {
    let mut kernel = ReadinessKernelBuilder::new()
        .node(node("a"), Vec::new(), Vec::new())
        .build()
        .expect("kernel");
    assert_eq!(
        kernel.start(&node("a")),
        Err(ReadinessError::IllegalTransition {
            node: node("a"),
            from: NodeLifecycle::Blocked,
            to: NodeLifecycle::Running,
        })
    );
    kernel.admit(&node("a")).expect("admit");
    kernel.start(&node("a")).expect("start");
    assert_eq!(
        kernel.start(&node("a")),
        Err(ReadinessError::IllegalTransition {
            node: node("a"),
            from: NodeLifecycle::Running,
            to: NodeLifecycle::Running,
        })
    );
}

#[test]
fn completing_a_node_that_never_started_is_refused() {
    let mut kernel = ReadinessKernelBuilder::new()
        .node(node("a"), Vec::new(), Vec::new())
        .build()
        .expect("kernel");
    kernel.admit(&node("a")).expect("admit");
    assert_eq!(
        kernel.complete(&node("a")),
        Err(ReadinessError::IllegalTransition {
            node: node("a"),
            from: NodeLifecycle::Runnable,
            to: NodeLifecycle::Completed,
        })
    );
}

#[test]
fn an_unknown_node_occurrence_is_refused_on_every_entry_point() {
    let mut kernel = diamond_join();
    let ghost = node("ghost");
    let expected = ReadinessError::UnknownNodeOccurrence {
        node: ghost.clone(),
    };
    assert_eq!(kernel.lifecycle(&ghost), Err(expected.clone()));
    assert_eq!(kernel.remaining_predecessors(&ghost), Err(expected.clone()));
    assert_eq!(kernel.claim_slot(&ghost, "left"), Err(expected.clone()));
    assert_eq!(kernel.admit(&ghost), Err(expected.clone()));
    assert_eq!(kernel.release_predecessor(&ghost), Err(expected.clone()));
    assert_eq!(kernel.start(&ghost), Err(expected.clone()));
    assert_eq!(kernel.cancel(&ghost), Err(expected.clone()));
    assert_eq!(kernel.runnable_node(&ghost), Err(expected));
    assert!(kernel.is_locally_quiescent());
}

#[test]
fn an_undeclared_slot_is_refused() {
    let mut kernel = diamond_join();
    assert_eq!(
        kernel.claim_slot(&node("c"), "middle"),
        Err(ReadinessError::UnknownOperandSlot {
            node: node("c"),
            slot: "middle".to_string(),
        })
    );
}

#[test]
fn quiescence_is_never_observed_while_a_successor_release_is_in_flight() {
    // A fan-out node with three successors: the credit protocol must keep the
    // outstanding count above zero for the whole fan-out.
    let mut kernel = ReadinessKernelBuilder::new()
        .node(
            node("root"),
            Vec::new(),
            vec![node("x"), node("y"), node("z")],
        )
        .node(node("x"), Vec::new(), Vec::new())
        .node(node("y"), Vec::new(), Vec::new())
        .node(node("z"), Vec::new(), Vec::new())
        .build()
        .expect("fan-out kernel");

    assert!(kernel.is_locally_quiescent());
    kernel.admit(&node("root")).expect("admit root");
    assert_eq!(kernel.activity_credits(), 1);
    kernel.start(&node("root")).expect("start root");
    assert_eq!(kernel.activity_credits(), 1);

    let transitions = kernel.complete(&node("root")).expect("complete root");
    assert_eq!(transitions.len(), 3);
    assert!(
        transitions
            .iter()
            .all(|transition| matches!(transition, ReadinessTransition::BecameRunnable { .. }))
    );
    // Three runnable successors hold one credit each; the root released its own.
    assert_eq!(kernel.activity_credits(), 3);
    assert!(!kernel.is_locally_quiescent());
}

#[test]
fn an_activation_that_runs_to_completion_returns_to_quiescence() {
    let mut kernel = ReadinessKernelBuilder::new()
        .node(node("a"), Vec::new(), vec![node("b")])
        .node(node("b"), Vec::new(), vec![node("c")])
        .node(node("c"), Vec::new(), Vec::new())
        .build()
        .expect("chain kernel");

    kernel.admit(&node("a")).expect("admit a");
    for step in ["a", "b", "c"] {
        kernel.start(&node(step)).expect("start");
        kernel.complete(&node(step)).expect("complete");
    }
    assert_eq!(kernel.activity_credits(), 0);
    assert!(kernel.is_locally_quiescent());
    for step in ["a", "b", "c"] {
        assert_eq!(kernel.lifecycle(&node(step)), Ok(NodeLifecycle::Completed));
    }
}

#[test]
fn an_activation_cancelled_mid_flight_returns_to_quiescence() {
    let mut kernel = ReadinessKernelBuilder::new()
        .node(node("a"), Vec::new(), vec![node("b"), node("c")])
        .node(node("b"), Vec::new(), Vec::new())
        .node(node("c"), Vec::new(), Vec::new())
        .build()
        .expect("kernel");

    kernel.admit(&node("a")).expect("admit a");
    kernel.start(&node("a")).expect("start a");
    kernel.complete(&node("a")).expect("complete a");
    assert_eq!(kernel.activity_credits(), 2);

    kernel.cancel(&node("b")).expect("cancel b from runnable");
    kernel.start(&node("c")).expect("start c");
    kernel.cancel(&node("c")).expect("cancel c from running");
    assert!(kernel.is_locally_quiescent());
}

#[test]
fn readiness_is_deterministic_across_two_independent_replays() {
    // Readiness is recomputed from the immutable declaration, so a resumed
    // activation reaches the same runnable set in the same order.
    fn replay(order: [&str; 2]) -> Vec<NodeOccurrenceId> {
        let mut kernel = diamond_join();
        publish(&mut kernel, &node("c"), "right", "apxm.Text");
        publish(&mut kernel, &node("c"), "left", "apxm.Text");
        let mut became_runnable = Vec::new();
        for step in order {
            kernel.admit(&node(step)).expect("admit");
            kernel.start(&node(step)).expect("start");
            for transition in kernel.complete(&node(step)).expect("complete") {
                if let ReadinessTransition::BecameRunnable { node } = transition {
                    became_runnable.push(node);
                }
            }
        }
        became_runnable
    }

    assert_eq!(replay(["a", "b"]), vec![node("c")]);
    assert_eq!(replay(["b", "a"]), vec![node("c")]);
}
