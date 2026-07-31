//! The per-activation readiness kernel.
//!
//! The kernel owns graph dependency and control readiness for one leased
//! activation and nothing else. It decides *whether* a dynamic node occurrence
//! may run; it never decides *where* it runs, never touches durable storage, and
//! never performs an effect. Placement, work stealing, and durable activation
//! persistence are separate mechanisms with separate owners.
//!
//! Every node occurrence carries typed operand slots, a remaining-predecessor
//! count, and one closed lifecycle:
//!
//! ```text
//! blocked -> runnable -> running -> completed
//!    |          |          |
//!    +----------+----------+-> cancelled
//!    +----------+----------+-> failed
//! ```
//!
//! Two ordering rules make the transition into `runnable` unambiguous:
//!
//! - the exact edge slot is claimed before its winner writes the operand, and
//!   operand publication happens-before the one predecessor decrement; and
//! - for a non-zero predecessor count only the decrement from one to zero may
//!   attempt `blocked -> runnable`, while a zero-predecessor node uses the
//!   explicit [`ReadinessKernel::admit`] transition.
//!
//! Cancellation racing the last predecessor is a legal closed race: the
//! cancelled node stays cancelled and the winning decrement reports that nothing
//! became runnable, so a queued item is discarded lazily rather than resurrected.
//! Duplicate, late, type-mismatched, and stale-claim publication all fail closed
//! with a typed error.
//!
//! Local quiescence is accounted through one activity-credit protocol so a local
//! executor can stop spinning without ever observing a false empty. A credit is
//! *reserved* before a `blocked -> runnable` transition is attempted, and then
//! either transferred to the node that won the transition or released exactly
//! once by the caller that reserved it. A credit is therefore held by every
//! runnable or running node occurrence and by every successor release still in
//! flight, and never by anything else. Quiescence is explicitly *not* durable
//! no-work truth: only Server-owned managed durability can decide that.

use std::collections::BTreeMap;

/// The immutable identity of one dynamic node occurrence inside one activation.
///
/// It is minted with the plan that declared the node, never parsed from a
/// selector and never reconstructed from a payload.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeOccurrenceId(String);

impl NodeOccurrenceId {
    /// Construct one non-empty node-occurrence identity.
    ///
    /// # Errors
    ///
    /// Returns [`ReadinessError::EmptyNodeOccurrenceId`] for a blank identity.
    pub fn new(value: impl Into<String>) -> Result<Self, ReadinessError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(ReadinessError::EmptyNodeOccurrenceId);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for NodeOccurrenceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// The closed node-occurrence lifecycle. No other state exists, and no other
/// transition reaches [`NodeLifecycle::Runnable`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeLifecycle {
    Blocked,
    Runnable,
    Running,
    Completed,
    Cancelled,
    Failed,
}

impl NodeLifecycle {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Blocked => "blocked",
            Self::Runnable => "runnable",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    /// True once no further transition is legal.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }
}

impl std::fmt::Display for NodeLifecycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// One declared typed operand slot on a node occurrence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperandSlotDecl {
    pub slot: String,
    pub type_ref: String,
}

/// A published operand value reference. The kernel carries the reference and its
/// declared type only; Program payloads never enter readiness state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperandValue {
    pub value_id: String,
    pub type_ref: String,
}

/// The exclusive right to write one exact operand slot.
///
/// A token is minted by [`ReadinessKernel::claim_slot`] and consumed by
/// [`ReadinessKernel::publish_operand`]. It carries the claim generation so a
/// token from a superseded activation attempt fails closed instead of
/// overwriting a live slot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlotClaim {
    node: NodeOccurrenceId,
    slot: String,
    generation: u64,
}

impl SlotClaim {
    #[must_use]
    pub fn node(&self) -> &NodeOccurrenceId {
        &self.node
    }

    #[must_use]
    pub fn slot(&self) -> &str {
        &self.slot
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SlotState {
    Unclaimed,
    Claimed { generation: u64 },
    Published { value: OperandValue },
}

#[derive(Clone, Debug)]
struct NodeState {
    lifecycle: NodeLifecycle,
    remaining_predecessors: u32,
    slots: BTreeMap<String, (String, SlotState)>,
    successors: Vec<NodeOccurrenceId>,
}

impl NodeState {
    fn operands_published(&self) -> bool {
        self.slots
            .values()
            .all(|(_, state)| matches!(state, SlotState::Published { .. }))
    }
}

/// Why a readiness operation was refused. Every variant fails closed: the kernel
/// state is unchanged and no node becomes runnable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadinessError {
    EmptyNodeOccurrenceId,
    DuplicateNodeOccurrence {
        node: NodeOccurrenceId,
    },
    DuplicateOperandSlot {
        node: NodeOccurrenceId,
        slot: String,
    },
    UnknownNodeOccurrence {
        node: NodeOccurrenceId,
    },
    UnknownOperandSlot {
        node: NodeOccurrenceId,
        slot: String,
    },
    UnknownSuccessor {
        node: NodeOccurrenceId,
        successor: NodeOccurrenceId,
    },
    /// A second writer tried to claim a slot whose exact edge is already owned.
    SlotAlreadyClaimed {
        node: NodeOccurrenceId,
        slot: String,
    },
    /// A claim arrived after the slot was already published.
    SlotAlreadyPublished {
        node: NodeOccurrenceId,
        slot: String,
    },
    /// The presented claim does not match the live claim on the slot.
    StaleSlotClaim {
        node: NodeOccurrenceId,
        slot: String,
        presented: u64,
        live: Option<u64>,
    },
    /// The published value's declared type is not the slot's declared type.
    OperandTypeMismatch {
        node: NodeOccurrenceId,
        slot: String,
        declared: String,
        published: String,
    },
    /// A predecessor decrement arrived for a node with no remaining predecessors.
    PredecessorUnderflow {
        node: NodeOccurrenceId,
    },
    /// Explicit admission was attempted on a node that still has predecessors.
    AdmissionRequiresZeroPredecessors {
        node: NodeOccurrenceId,
        remaining: u32,
    },
    /// A node reached zero predecessors with an unpublished operand slot.
    OperandsIncomplete {
        node: NodeOccurrenceId,
        slot: String,
    },
    /// The requested lifecycle edge is not in the closed transition set.
    IllegalTransition {
        node: NodeOccurrenceId,
        from: NodeLifecycle,
        to: NodeLifecycle,
    },
}

impl std::fmt::Display for ReadinessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyNodeOccurrenceId => {
                f.write_str("node occurrence identity must not be empty")
            }
            Self::DuplicateNodeOccurrence { node } => {
                write!(f, "node occurrence {node} is declared more than once")
            }
            Self::DuplicateOperandSlot { node, slot } => {
                write!(f, "node occurrence {node} declares slot {slot} twice")
            }
            Self::UnknownNodeOccurrence { node } => {
                write!(f, "unknown node occurrence {node}")
            }
            Self::UnknownOperandSlot { node, slot } => {
                write!(f, "node occurrence {node} declares no slot {slot}")
            }
            Self::UnknownSuccessor { node, successor } => write!(
                f,
                "node occurrence {node} declares unknown successor {successor}"
            ),
            Self::SlotAlreadyClaimed { node, slot } => {
                write!(f, "slot {slot} on {node} is already claimed")
            }
            Self::SlotAlreadyPublished { node, slot } => {
                write!(f, "slot {slot} on {node} is already published")
            }
            Self::StaleSlotClaim {
                node,
                slot,
                presented,
                live,
            } => match live {
                Some(live) => write!(
                    f,
                    "stale claim {presented} for slot {slot} on {node}; live claim is {live}"
                ),
                None => write!(
                    f,
                    "stale claim {presented} for slot {slot} on {node}; the slot is unclaimed"
                ),
            },
            Self::OperandTypeMismatch {
                node,
                slot,
                declared,
                published,
            } => write!(
                f,
                "slot {slot} on {node} declares {declared} but publication carries {published}"
            ),
            Self::PredecessorUnderflow { node } => {
                write!(f, "node occurrence {node} has no remaining predecessor")
            }
            Self::AdmissionRequiresZeroPredecessors { node, remaining } => write!(
                f,
                "node occurrence {node} still has {remaining} predecessors and cannot be admitted"
            ),
            Self::OperandsIncomplete { node, slot } => write!(
                f,
                "node occurrence {node} reached zero predecessors with slot {slot} unpublished"
            ),
            Self::IllegalTransition { node, from, to } => {
                write!(f, "node occurrence {node} cannot move {from} -> {to}")
            }
        }
    }
}

impl std::error::Error for ReadinessError {}

/// The result of a predecessor release or an explicit admission.
///
/// `BecameRunnable` is the only report that authorizes local dispatch. Every
/// other report is a legal closed outcome that authorizes nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadinessTransition {
    /// This decrement or admission moved the node `blocked -> runnable`.
    BecameRunnable { node: NodeOccurrenceId },
    /// Predecessors remain; the node stays blocked.
    StillBlocked {
        node: NodeOccurrenceId,
        remaining: u32,
    },
    /// The node already lost its race to cancellation or failure. The winning
    /// lifecycle transition stands and any queued item is discarded lazily.
    AlreadyTerminal {
        node: NodeOccurrenceId,
        lifecycle: NodeLifecycle,
    },
}

/// A node occurrence the local executor may run.
///
/// The value is immutable and carries the exact occurrence identity plus its
/// published operands. It moves no authority: no lease, grant, approval,
/// credential, event-fulfillment right, or commit right travels with it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunnableNode {
    pub node: NodeOccurrenceId,
    pub operands: BTreeMap<String, OperandValue>,
}

/// Declarative construction of one activation's readiness state.
///
/// The builder is the kernel's only entry point, so a kernel can never observe a
/// half-declared graph: successor references are resolved once, at
/// [`ReadinessKernelBuilder::build`].
#[derive(Debug, Default)]
pub struct ReadinessKernelBuilder {
    nodes: Vec<(NodeOccurrenceId, Vec<OperandSlotDecl>, Vec<NodeOccurrenceId>)>,
}

impl ReadinessKernelBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    /// Declare one node occurrence with its typed operand slots and the exact
    /// successors it releases when it completes.
    ///
    /// The remaining-predecessor count is derived from the declared successor
    /// edges rather than supplied, so a node's count and its incoming edge set
    /// cannot disagree.
    #[must_use]
    pub fn node(
        mut self,
        node: NodeOccurrenceId,
        slots: Vec<OperandSlotDecl>,
        successors: Vec<NodeOccurrenceId>,
    ) -> Self {
        self.nodes.push((node, slots, successors));
        self
    }

    /// Resolve the declared graph into a kernel.
    ///
    /// # Errors
    ///
    /// Returns [`ReadinessError`] for a duplicate node occurrence, a duplicate
    /// operand slot, or a successor edge that names an undeclared node.
    pub fn build(self) -> Result<ReadinessKernel, ReadinessError> {
        let mut nodes: BTreeMap<NodeOccurrenceId, NodeState> = BTreeMap::new();
        for (node, slots, successors) in &self.nodes {
            if nodes.contains_key(node) {
                return Err(ReadinessError::DuplicateNodeOccurrence { node: node.clone() });
            }
            let mut declared: BTreeMap<String, (String, SlotState)> = BTreeMap::new();
            for slot in slots {
                if declared.contains_key(&slot.slot) {
                    return Err(ReadinessError::DuplicateOperandSlot {
                        node: node.clone(),
                        slot: slot.slot.clone(),
                    });
                }
                declared.insert(
                    slot.slot.clone(),
                    (slot.type_ref.clone(), SlotState::Unclaimed),
                );
            }
            nodes.insert(
                node.clone(),
                NodeState {
                    lifecycle: NodeLifecycle::Blocked,
                    remaining_predecessors: 0,
                    slots: declared,
                    successors: successors.clone(),
                },
            );
        }

        for (node, _, successors) in &self.nodes {
            for successor in successors {
                let Some(state) = nodes.get_mut(successor) else {
                    return Err(ReadinessError::UnknownSuccessor {
                        node: node.clone(),
                        successor: successor.clone(),
                    });
                };
                state.remaining_predecessors += 1;
            }
        }

        Ok(ReadinessKernel {
            nodes,
            claim_generation: 0,
            activity_credits: 0,
        })
    }
}

/// The readiness kernel for one leased activation.
///
/// It is a pure in-memory state machine over one activation's node occurrences.
/// It holds no durable handle, performs no I/O, and selects no worker.
#[derive(Debug)]
pub struct ReadinessKernel {
    nodes: BTreeMap<NodeOccurrenceId, NodeState>,
    claim_generation: u64,
    activity_credits: u64,
}

impl ReadinessKernel {
    /// The current lifecycle of one node occurrence.
    ///
    /// # Errors
    ///
    /// Returns [`ReadinessError::UnknownNodeOccurrence`] for an undeclared node.
    pub fn lifecycle(&self, node: &NodeOccurrenceId) -> Result<NodeLifecycle, ReadinessError> {
        Ok(self.state(node)?.lifecycle)
    }

    /// The remaining predecessor count for one node occurrence.
    ///
    /// # Errors
    ///
    /// Returns [`ReadinessError::UnknownNodeOccurrence`] for an undeclared node.
    pub fn remaining_predecessors(&self, node: &NodeOccurrenceId) -> Result<u32, ReadinessError> {
        Ok(self.state(node)?.remaining_predecessors)
    }

    /// The node occurrences with no predecessors, in deterministic order.
    ///
    /// These are exactly the nodes [`ReadinessKernel::admit`] accepts.
    #[must_use]
    pub fn zero_predecessor_nodes(&self) -> Vec<NodeOccurrenceId> {
        self.nodes
            .iter()
            .filter(|(_, state)| state.remaining_predecessors == 0)
            .map(|(node, _)| node.clone())
            .collect()
    }

    /// Claim the exclusive right to write one exact operand slot.
    ///
    /// The claim precedes the write so a duplicate writer is rejected before it
    /// can publish, rather than after it has already overwritten a value.
    ///
    /// # Errors
    ///
    /// Returns [`ReadinessError::SlotAlreadyClaimed`] for a second live writer,
    /// [`ReadinessError::SlotAlreadyPublished`] for a late writer, and the
    /// unknown-node/unknown-slot variants for an undeclared target.
    pub fn claim_slot(
        &mut self,
        node: &NodeOccurrenceId,
        slot: &str,
    ) -> Result<SlotClaim, ReadinessError> {
        let generation = self.claim_generation + 1;
        let state = self.state_mut(node)?;
        let Some((_, slot_state)) = state.slots.get_mut(slot) else {
            return Err(ReadinessError::UnknownOperandSlot {
                node: node.clone(),
                slot: slot.to_string(),
            });
        };
        match slot_state {
            SlotState::Published { .. } => Err(ReadinessError::SlotAlreadyPublished {
                node: node.clone(),
                slot: slot.to_string(),
            }),
            SlotState::Claimed { .. } => Err(ReadinessError::SlotAlreadyClaimed {
                node: node.clone(),
                slot: slot.to_string(),
            }),
            SlotState::Unclaimed => {
                *slot_state = SlotState::Claimed { generation };
                self.claim_generation = generation;
                Ok(SlotClaim {
                    node: node.clone(),
                    slot: slot.to_string(),
                    generation,
                })
            }
        }
    }

    /// Publish the operand value the claim authorizes.
    ///
    /// Publication happens-before the predecessor decrement that may release the
    /// node, so a node that becomes runnable always has every declared operand
    /// present.
    ///
    /// # Errors
    ///
    /// Returns [`ReadinessError::StaleSlotClaim`] when the presented claim is not
    /// the live claim, [`ReadinessError::SlotAlreadyPublished`] for a duplicate
    /// publication, and [`ReadinessError::OperandTypeMismatch`] when the value's
    /// declared type is not the slot's declared type.
    pub fn publish_operand(
        &mut self,
        claim: &SlotClaim,
        value: OperandValue,
    ) -> Result<(), ReadinessError> {
        let state = self.state_mut(&claim.node)?;
        let Some((declared_type, slot_state)) = state.slots.get_mut(&claim.slot) else {
            return Err(ReadinessError::UnknownOperandSlot {
                node: claim.node.clone(),
                slot: claim.slot.clone(),
            });
        };
        match slot_state {
            SlotState::Published { .. } => {
                return Err(ReadinessError::SlotAlreadyPublished {
                    node: claim.node.clone(),
                    slot: claim.slot.clone(),
                });
            }
            SlotState::Unclaimed => {
                return Err(ReadinessError::StaleSlotClaim {
                    node: claim.node.clone(),
                    slot: claim.slot.clone(),
                    presented: claim.generation,
                    live: None,
                });
            }
            SlotState::Claimed { generation } if *generation != claim.generation => {
                return Err(ReadinessError::StaleSlotClaim {
                    node: claim.node.clone(),
                    slot: claim.slot.clone(),
                    presented: claim.generation,
                    live: Some(*generation),
                });
            }
            SlotState::Claimed { .. } => {}
        }
        if *declared_type != value.type_ref {
            return Err(ReadinessError::OperandTypeMismatch {
                node: claim.node.clone(),
                slot: claim.slot.clone(),
                declared: declared_type.clone(),
                published: value.type_ref,
            });
        }
        *slot_state = SlotState::Published { value };
        Ok(())
    }

    /// Admit one zero-predecessor node occurrence into `runnable`.
    ///
    /// A node with predecessors is refused: the only path into `runnable` for
    /// such a node is the decrement from one to zero.
    ///
    /// # Errors
    ///
    /// Returns [`ReadinessError::AdmissionRequiresZeroPredecessors`] when the
    /// node still has predecessors, [`ReadinessError::OperandsIncomplete`] when a
    /// declared slot is unpublished, and [`ReadinessError::IllegalTransition`]
    /// when the node has already left `blocked`.
    pub fn admit(
        &mut self,
        node: &NodeOccurrenceId,
    ) -> Result<ReadinessTransition, ReadinessError> {
        self.reserved(node, Self::admit_zero_predecessor)
    }

    fn admit_zero_predecessor(
        &mut self,
        node: &NodeOccurrenceId,
    ) -> Result<ReadinessTransition, ReadinessError> {
        let state = self.state(node)?;
        if state.lifecycle.is_terminal() {
            return Ok(ReadinessTransition::AlreadyTerminal {
                node: node.clone(),
                lifecycle: state.lifecycle,
            });
        }
        if state.lifecycle != NodeLifecycle::Blocked {
            return Err(ReadinessError::IllegalTransition {
                node: node.clone(),
                from: state.lifecycle,
                to: NodeLifecycle::Runnable,
            });
        }
        if state.remaining_predecessors != 0 {
            return Err(ReadinessError::AdmissionRequiresZeroPredecessors {
                node: node.clone(),
                remaining: state.remaining_predecessors,
            });
        }
        self.release_into_runnable(node)
    }

    /// Record one predecessor completion for `node`.
    ///
    /// Only the decrement from one to zero may attempt `blocked -> runnable`. If
    /// cancellation or failure already won, the decrement still applies and the
    /// transition reports [`ReadinessTransition::AlreadyTerminal`], so the loser
    /// of the race is discarded lazily instead of resurrected.
    ///
    /// # Errors
    ///
    /// Returns [`ReadinessError::PredecessorUnderflow`] for a decrement with no
    /// remaining predecessor and [`ReadinessError::OperandsIncomplete`] when the
    /// last decrement arrives with a declared slot still unpublished.
    pub fn release_predecessor(
        &mut self,
        node: &NodeOccurrenceId,
    ) -> Result<ReadinessTransition, ReadinessError> {
        self.reserved(node, Self::decrement_predecessor)
    }

    /// Reserve exactly one activity credit, attempt one `blocked -> runnable`
    /// transition, and either transfer the reservation to the node that won the
    /// transition or release it exactly once.
    ///
    /// Reserving *before* the attempt is what keeps a release that is still in
    /// flight from being read as local quiescence, and releasing on every
    /// non-winning outcome — including the typed errors — is what keeps a
    /// refused transition from leaking a credit and wedging a worker awake.
    fn reserved(
        &mut self,
        node: &NodeOccurrenceId,
        attempt: fn(&mut Self, &NodeOccurrenceId) -> Result<ReadinessTransition, ReadinessError>,
    ) -> Result<ReadinessTransition, ReadinessError> {
        self.activity_credits += 1;
        let outcome = attempt(self, node);
        if !matches!(outcome, Ok(ReadinessTransition::BecameRunnable { .. })) {
            self.release_credit();
        }
        outcome
    }

    fn decrement_predecessor(
        &mut self,
        node: &NodeOccurrenceId,
    ) -> Result<ReadinessTransition, ReadinessError> {
        let state = self.state(node)?;
        if state.remaining_predecessors == 0 {
            return Err(ReadinessError::PredecessorUnderflow { node: node.clone() });
        }
        let remaining = state.remaining_predecessors - 1;
        let lifecycle = state.lifecycle;
        if remaining > 0 {
            self.state_mut(node)?.remaining_predecessors = remaining;
            return Ok(ReadinessTransition::StillBlocked {
                node: node.clone(),
                remaining,
            });
        }
        if lifecycle.is_terminal() {
            self.state_mut(node)?.remaining_predecessors = 0;
            return Ok(ReadinessTransition::AlreadyTerminal {
                node: node.clone(),
                lifecycle,
            });
        }
        if lifecycle != NodeLifecycle::Blocked {
            return Err(ReadinessError::IllegalTransition {
                node: node.clone(),
                from: lifecycle,
                to: NodeLifecycle::Runnable,
            });
        }
        // Operand completeness is checked before the counter is written, so a
        // last decrement that arrives with an unpublished slot leaves the
        // predecessor count untouched rather than stranding the node at zero.
        self.require_operands_published(node)?;
        self.state_mut(node)?.remaining_predecessors = 0;
        self.release_into_runnable(node)
    }

    /// Move one runnable node occurrence into `running`.
    ///
    /// # Errors
    ///
    /// Returns [`ReadinessError::IllegalTransition`] unless the node is runnable.
    pub fn start(&mut self, node: &NodeOccurrenceId) -> Result<(), ReadinessError> {
        self.transition(node, NodeLifecycle::Running, &[NodeLifecycle::Runnable])
    }

    /// Complete one running node occurrence and release each declared successor.
    ///
    /// Every successor release runs while this node still holds its own credit,
    /// and each release reserves its own credit before attempting a transition,
    /// so the outstanding count never reaches zero while a successor release is
    /// still in flight. This node's credit is released last.
    ///
    /// # Errors
    ///
    /// Returns [`ReadinessError::IllegalTransition`] unless the node is running,
    /// or any error raised while releasing a successor. On a successor error this
    /// node stays `completed` and its own credit is still released, so a refused
    /// successor cannot wedge the activation permanently awake.
    pub fn complete(
        &mut self,
        node: &NodeOccurrenceId,
    ) -> Result<Vec<ReadinessTransition>, ReadinessError> {
        self.transition(node, NodeLifecycle::Completed, &[NodeLifecycle::Running])?;
        let successors = self.state(node)?.successors.clone();
        let mut transitions = Vec::with_capacity(successors.len());
        let mut failure = None;
        for successor in &successors {
            match self.release_predecessor(successor) {
                Ok(transition) => transitions.push(transition),
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        // Released last, so quiescence is never observable mid-fan-out.
        self.release_credit();
        match failure {
            Some(error) => Err(error),
            None => Ok(transitions),
        }
    }

    /// Fail one node occurrence. Failure may win from `blocked`, `runnable`, or
    /// `running`, and never releases a successor.
    ///
    /// # Errors
    ///
    /// Returns [`ReadinessError::IllegalTransition`] from a terminal state.
    pub fn fail(&mut self, node: &NodeOccurrenceId) -> Result<(), ReadinessError> {
        let had_credit = self.holds_credit(node)?;
        self.transition(
            node,
            NodeLifecycle::Failed,
            &[
                NodeLifecycle::Blocked,
                NodeLifecycle::Runnable,
                NodeLifecycle::Running,
            ],
        )?;
        if had_credit {
            self.release_credit();
        }
        Ok(())
    }

    /// Cancel one node occurrence. Cancellation may win from `blocked`,
    /// `runnable`, or `running`, and never releases a successor.
    ///
    /// # Errors
    ///
    /// Returns [`ReadinessError::IllegalTransition`] from a terminal state.
    pub fn cancel(&mut self, node: &NodeOccurrenceId) -> Result<(), ReadinessError> {
        let had_credit = self.holds_credit(node)?;
        self.transition(
            node,
            NodeLifecycle::Cancelled,
            &[
                NodeLifecycle::Blocked,
                NodeLifecycle::Runnable,
                NodeLifecycle::Running,
            ],
        )?;
        if had_credit {
            self.release_credit();
        }
        Ok(())
    }

    /// Materialize the immutable runnable value for one runnable node.
    ///
    /// # Errors
    ///
    /// Returns [`ReadinessError::IllegalTransition`] unless the node is runnable
    /// and [`ReadinessError::OperandsIncomplete`] if a declared slot is
    /// unpublished.
    pub fn runnable_node(
        &self,
        node: &NodeOccurrenceId,
    ) -> Result<RunnableNode, ReadinessError> {
        let state = self.state(node)?;
        if state.lifecycle != NodeLifecycle::Runnable {
            return Err(ReadinessError::IllegalTransition {
                node: node.clone(),
                from: state.lifecycle,
                to: NodeLifecycle::Runnable,
            });
        }
        let mut operands = BTreeMap::new();
        for (slot, (_, slot_state)) in &state.slots {
            let SlotState::Published { value } = slot_state else {
                return Err(ReadinessError::OperandsIncomplete {
                    node: node.clone(),
                    slot: slot.clone(),
                });
            };
            operands.insert(slot.clone(), value.clone());
        }
        Ok(RunnableNode {
            node: node.clone(),
            operands,
        })
    }

    /// The outstanding local activity credit count.
    ///
    /// A credit is held by every node occurrence that is runnable or running, and
    /// by every successor release still in flight.
    #[must_use]
    pub const fn activity_credits(&self) -> u64 {
        self.activity_credits
    }

    /// True when this activation has no local work left to do.
    ///
    /// This is a *local* observation used to stop a worker from spinning. It is
    /// explicitly non-authoritative for durable no-work truth: only Server-owned
    /// managed durability can decide that, and a locally quiescent activation may
    /// still be awaiting a durable event or a managed effect outcome.
    #[must_use]
    pub const fn is_locally_quiescent(&self) -> bool {
        self.activity_credits == 0
    }

    /// Refuse the transition unless every declared slot is published. The kernel
    /// never manufactures a default operand to unblock work.
    fn require_operands_published(&self, node: &NodeOccurrenceId) -> Result<(), ReadinessError> {
        let state = self.state(node)?;
        for (slot, (_, slot_state)) in &state.slots {
            if !matches!(slot_state, SlotState::Published { .. }) {
                return Err(ReadinessError::OperandsIncomplete {
                    node: node.clone(),
                    slot: slot.clone(),
                });
            }
        }
        Ok(())
    }

    fn release_into_runnable(
        &mut self,
        node: &NodeOccurrenceId,
    ) -> Result<ReadinessTransition, ReadinessError> {
        self.require_operands_published(node)?;
        debug_assert!(self.state(node)?.operands_published());
        // The caller's reservation in `reserved` is transferred to this node
        // rather than a second credit being minted here.
        debug_assert!(self.activity_credits > 0, "runnable without a reservation");
        self.state_mut(node)?.lifecycle = NodeLifecycle::Runnable;
        Ok(ReadinessTransition::BecameRunnable { node: node.clone() })
    }

    /// True when the node currently holds one activity credit, which is exactly
    /// the runnable and running states.
    fn holds_credit(&self, node: &NodeOccurrenceId) -> Result<bool, ReadinessError> {
        Ok(matches!(
            self.state(node)?.lifecycle,
            NodeLifecycle::Runnable | NodeLifecycle::Running
        ))
    }

    fn release_credit(&mut self) {
        debug_assert!(self.activity_credits > 0, "credit released without a hold");
        self.activity_credits = self.activity_credits.saturating_sub(1);
    }

    fn transition(
        &mut self,
        node: &NodeOccurrenceId,
        to: NodeLifecycle,
        from: &[NodeLifecycle],
    ) -> Result<(), ReadinessError> {
        let current = self.state(node)?.lifecycle;
        if !from.contains(&current) {
            return Err(ReadinessError::IllegalTransition {
                node: node.clone(),
                from: current,
                to,
            });
        }
        self.state_mut(node)?.lifecycle = to;
        Ok(())
    }

    fn state(&self, node: &NodeOccurrenceId) -> Result<&NodeState, ReadinessError> {
        self.nodes
            .get(node)
            .ok_or_else(|| ReadinessError::UnknownNodeOccurrence { node: node.clone() })
    }

    fn state_mut(&mut self, node: &NodeOccurrenceId) -> Result<&mut NodeState, ReadinessError> {
        self.nodes
            .get_mut(node)
            .ok_or_else(|| ReadinessError::UnknownNodeOccurrence { node: node.clone() })
    }
}
