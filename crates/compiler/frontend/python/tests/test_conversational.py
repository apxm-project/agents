"""`ConversationalAgent`/`CompactionPolicy` serialization tests.

Fixes the write-only-metadata bug from `docs/plans/tasks/W2.7.md`:
`CompactionPolicy` values used to be stuffed into
`MultiFlowArtifact.metadata`, which `to_air()` never reads — so
`compact_at_tokens`/`strategy` never reached the AIR text. These tests prove
the policy now reaches the turn graph's own `metadata` (AIR-reachable) AND
the marked conversational-turn ASK node's attributes (the runtime's
execution-time channel), and that the four control dials (default /
configure / override / opt-out) are all distinguishable at the artifact
level before any runtime behavior is added.
"""

from apxm import CompactionPolicy, ConversationalAgent, hook
from apxm.conversational import (
    COMPACTION_AT_TOKENS_ATTR,
    COMPACTION_KEEP_RECENT_ATTR,
    COMPACTION_OVERRIDE_PRESENT_ATTR,
    COMPACTION_STRATEGY_ATTR,
    COMPACTION_SUMMARY_KEY_ATTR,
)
from apxm.hooks import LifecycleEvent


def _turn_graph(artifact):
    """The `conversation.turn` flow — the graph the marked ASK lives in."""
    for graph in artifact.graphs:
        if graph.name == "conversation.turn":
            return graph
    raise AssertionError("no conversation.turn flow in compiled artifact")


def _marked_ask(graph):
    for node in graph.nodes:
        if node.op == "ASK" and node.attributes.get("conversational_turn") == "true":
            return node
    raise AssertionError("no marked conversational-turn ASK node in turn graph")


def test_configure_dial_reaches_turn_graph_metadata_and_air_text():
    """Configure: non-default `keep_recent`/`compact_at_tokens` values reach
    the turn graph's own `metadata` (AIR-reachable), not the inert top-level
    `MultiFlowArtifact.metadata`."""
    agent = ConversationalAgent(
        persona="p",
        compaction=CompactionPolicy(
            keep_recent=2, compact_at_tokens=300, strategy="summarize"
        ),
    )
    artifact = agent.compile()
    turn_graph = _turn_graph(artifact)

    compaction_meta = turn_graph.metadata.get("compaction")
    assert compaction_meta == {
        "keep_recent": 2,
        "compact_at_tokens": 300,
        "strategy": "summarize",
        "summary_key": "conversation:summary",
        "override_present": False,
    }

    # The bug this fixes: the OLD code stashed compaction only in the
    # top-level (non-AIR-reaching) artifact metadata.
    assert "compaction" not in artifact.metadata

    air = artifact.to_air()
    assert "300" in air, "compact_at_tokens must reach the AIR text"


def test_default_dial_stamps_dataclass_defaults():
    """Default: an author who supplies a bare `CompactionPolicy()` gets the
    dataclass defaults serialized (not silently dropped)."""
    agent = ConversationalAgent(persona="p", compaction=CompactionPolicy())
    artifact = agent.compile()
    turn_graph = _turn_graph(artifact)

    assert turn_graph.metadata["compaction"] == {
        "keep_recent": 4,
        "compact_at_tokens": 20_000,
        "strategy": "summarize",
        "summary_key": "conversation:summary",
        "override_present": False,
    }


def test_opt_out_dial_omits_compaction_entirely():
    """Opt-out: `compaction=None` (the default) must not stamp ANY compaction
    key into the turn graph metadata or the marked ask's attributes — a true
    no-op, not a zero-valued policy."""
    agent = ConversationalAgent(persona="p")
    artifact = agent.compile()
    turn_graph = _turn_graph(artifact)
    ask_node = _marked_ask(turn_graph)

    assert "compaction" not in turn_graph.metadata
    for attr in (
        COMPACTION_AT_TOKENS_ATTR,
        COMPACTION_KEEP_RECENT_ATTR,
        COMPACTION_STRATEGY_ATTR,
        COMPACTION_SUMMARY_KEY_ATTR,
        COMPACTION_OVERRIDE_PRESENT_ATTR,
    ):
        assert attr not in ask_node.attributes


def test_configure_dial_reaches_marked_ask_node_attributes():
    """The runtime's execution-time channel: the marked conversational-turn
    ASK carries the same fields as the graph metadata, flat (not nested), so
    `ConversationMemoryMiddleware` can read them directly off the node."""
    agent = ConversationalAgent(
        persona="p",
        compaction=CompactionPolicy(
            keep_recent=2, compact_at_tokens=300, summary_key="conv:rolling"
        ),
    )
    artifact = agent.compile()
    ask_node = _marked_ask(_turn_graph(artifact))

    assert ask_node.attributes[COMPACTION_AT_TOKENS_ATTR] == 300
    assert ask_node.attributes[COMPACTION_KEEP_RECENT_ATTR] == 2
    assert ask_node.attributes[COMPACTION_STRATEGY_ATTR] == "summarize"
    assert ask_node.attributes[COMPACTION_SUMMARY_KEY_ATTR] == "conv:rolling"
    assert ask_node.attributes[COMPACTION_OVERRIDE_PRESENT_ATTR] is False


def test_override_dial_sets_override_present_when_post_turn_hook_registered():
    """Override: a program that ALSO registers a `post_turn` hook (the
    `controllable_agent.py` pattern) must flip `override_present` so the
    runtime default fails closed (no double-compaction)."""

    @hook(on=LifecycleEvent.POST_TURN)
    def compact(ctx, reply):
        pass

    policy = CompactionPolicy(keep_recent=4, compact_at_tokens=300)
    agent = ConversationalAgent(persona="p", compaction=policy, hooks=[compact])
    artifact = agent.compile()
    turn_graph = _turn_graph(artifact)
    ask_node = _marked_ask(turn_graph)

    assert turn_graph.metadata["compaction"]["override_present"] is True
    assert ask_node.attributes[COMPACTION_OVERRIDE_PRESENT_ATTR] is True


def test_override_dial_unaffected_by_unrelated_hooks():
    """A non-`post_turn` hook must NOT be mistaken for a compaction
    override — only a `post_turn` hook trips `override_present`."""

    @hook(on=LifecycleEvent.PRE_TURN)
    def before(ctx):
        pass

    agent = ConversationalAgent(
        persona="p",
        compaction=CompactionPolicy(compact_at_tokens=300),
        hooks=[before],
    )
    artifact = agent.compile()
    assert _turn_graph(artifact).metadata["compaction"]["override_present"] is False
