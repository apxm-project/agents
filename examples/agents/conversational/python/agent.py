"""Example-local conversational program over the installed generic frontend."""

from __future__ import annotations

import json
import sys
from pathlib import Path

from apxm_program import AgentProgram, ContextEdge

SOURCE_PATH = Path(__file__)


def bind_source_span(
    program: AgentProgram,
    node_id: str,
    authored_call: str,
    annotation: str,
) -> None:
    """Bind a node to the exact authored call in this source file."""

    for line, text in enumerate(SOURCE_PATH.read_text().splitlines(), start=1):
        if authored_call in text:
            start_column = text.index(authored_call)
            program.source_span(
                node_id,
                SOURCE_PATH.name,
                line,
                annotation,
                start_column,
                start_column + len(authored_call),
            )
            return
    raise ValueError(f"authored call not found: {authored_call}")


class ConversationalAgent(AgentProgram):
    """Repository example that authors one generic structured conversation loop."""

    def define_conversation(self) -> "ConversationalAgent":
        self.loop(
            "region.loop.conversation",
            lambda body: (
                body.model_call("node.model", "model.default")
                .capability_invoke("node.capability", "cap.search")
                .await_event("node.await", "event.conversation.input")
            ),
        )
        self.context_flow(ContextEdge("node.model", "node.capability", "ConversationContext"))
        bind_source_span(
            self,
            "node.model",
            '.model_call("node.model"',
            "model.call",
        )
        bind_source_span(
            self,
            "node.capability",
            '.capability_invoke("node.capability"',
            "capability.invoke",
        )
        bind_source_span(
            self,
            "node.await",
            '.await_event("node.await"',
            "await.event",
        )
        self.return_region("region.return")
        return self


def build_conversational() -> ConversationalAgent:
    """Build the Python conversational example."""

    return ConversationalAgent(
        program_id="ConversationalExample",
        input_type_ref="ConversationInput",
        output_type_ref="ConversationOutput",
        context_type_ref="ConversationContext",
    ).define_conversation()


def main() -> None:
    """Print the example graph, or canonical AIR with ``--air``."""

    program = build_conversational()
    value = program.lower() if "--air" in sys.argv[1:] else program.build_graph()
    print(json.dumps(value, sort_keys=True, separators=(",", ":")))


if __name__ == "__main__":
    main()
