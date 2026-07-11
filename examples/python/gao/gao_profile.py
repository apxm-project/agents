#!/usr/bin/env python3
"""Gao's Python reference profile.

The profile keeps authoring in the Python frontend. Local paper retrieval is a
read-only capability backed by the bundled corpus. Web search is an abstract
provider capability: the profile advertises the ``web`` group only when the
host supplies a non-empty connection reference.
"""

from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path
from typing import Callable, Iterable

from apxm import CompactionPolicy, ConversationalAgent, GraphRecorder, ToolGroup, compile, tool


PROFILE_ROOT = Path(__file__).resolve().parent
PAPER_CORPUS_PATH = PROFILE_ROOT / "knowledge" / "papers.json"
SEARCH_WEB_CAPABILITY = "search_web"
_BASE_GROUPS: tuple[str, ...] = ("discovery", ToolGroup.SKILLS.value, ToolGroup.AUTHORING.value)
_STOPWORDS = frozenset({"about", "and", "are", "does", "for", "how", "the", "what", "with"})


@dataclass(frozen=True, slots=True)
class ProviderConnection:
    """The non-secret connection reference supplied by the host."""

    reference: str | None


@dataclass(frozen=True, slots=True)
class LiveTurnResult:
    """A deterministic representation of a completed Gao turn."""

    answer: str
    paper_result: dict[str, object]
    canvas: dict[str, object]


def _load_papers() -> list[dict[str, object]]:
    raw = json.loads(PAPER_CORPUS_PATH.read_text(encoding="utf-8"))
    if not isinstance(raw, list) or not all(isinstance(item, dict) for item in raw):
        raise ValueError("Gao paper corpus must be a JSON array of objects")
    return [dict(item) for item in raw]


def provider_capabilities(connection: ProviderConnection | None) -> tuple[str, ...]:
    """Return provider-backed capabilities available for this turn.

    The connection reference is opaque. Credential material is resolved by the
    provider/Auth boundary and is never read by this profile.
    """

    if connection is None or not (connection.reference or "").strip():
        return ()
    return (SEARCH_WEB_CAPABILITY,)


def capability_groups(connection: ProviderConnection | None = None) -> list[str]:
    groups = list(_BASE_GROUPS)
    if provider_capabilities(connection):
        groups.append(ToolGroup.WEB.value)
    return groups


@tool(name="search_papers")
def search_papers(query: str) -> str:
    """Search the local APXM paper corpus and return cited matches."""

    terms = {
        term.strip(".,:;!?()[]{}").lower()
        for term in query.split()
        if len(term.strip(".,:;!?()[]{}")) > 2
        and term.strip(".,:;!?()[]{}").lower() not in _STOPWORDS
    }
    matches: list[dict[str, object]] = []
    for paper in _load_papers():
        haystack = " ".join(
            [
                str(paper.get("title", "")),
                str(paper.get("abstract", "")),
                " ".join(str(keyword) for keyword in paper.get("keywords", [])),
            ]
        ).lower()
        score = sum(term in haystack for term in terms)
        if score:
            matches.append(
                {
                    "id": paper["id"],
                    "citation": paper["citation"],
                    "title": paper["title"],
                    "abstract": paper["abstract"],
                    "score": score,
                }
            )
    matches.sort(key=lambda item: (-int(item["score"]), str(item["id"])))
    return json.dumps(
        {
            "query": query,
            "matches": matches,
            "message": "" if matches else "No source found in the local paper corpus.",
        },
        sort_keys=True,
    )


@tool(name="plan_workflow")
def plan_workflow(request: str) -> str:
    """Create a Studio workflow draft that can be applied to the canvas."""

    canvas = {
        "format": "apxm_studio_workflow",
        "name": "gao_architecture_response",
        "description": request.strip() or "APXM architecture response",
        "nodes": [
            {
                "id": "question",
                "kind": "text",
                "label": "Question",
                "position": {"x": 40, "y": 100},
                "config": {"text": request},
            },
            {
                "id": "answer",
                "kind": "llm",
                "label": "Architecture answer",
                "position": {"x": 380, "y": 100},
                "config": {"prompt": "Answer the connected question: {question}"},
            },
            {
                "id": "result",
                "kind": "output",
                "label": "Result",
                "position": {"x": 760, "y": 100},
                "config": {},
            },
        ],
        "edges": [
            {"source": "question", "target": "answer", "kind": "data"},
            {"source": "answer", "target": "result", "kind": "data"},
        ],
    }
    return json.dumps(canvas, sort_keys=True)


GAO_CAPABILITY_TOOL_BINDINGS = [search_papers, plan_workflow]


def _persona() -> str:
    return (
        "You are Gao, an APXM architecture expert. Use the supplied paper search "
        "result as the only source for citations. If it has no matches, say that "
        "no source was found. When a workflow is requested, return the supplied "
        "Studio draft without inventing capabilities or credentials."
    )


def build_profile(connection: ProviderConnection | None = None):
    """Build the multi-flow Python profile artifact."""

    return ConversationalAgent(
        persona=_persona(),
        memory_space="stm",
        tools=GAO_CAPABILITY_TOOL_BINDINGS,
        capability_groups=capability_groups(connection),
        skills=True,
        compaction=CompactionPolicy(
            keep_recent=4,
            compact_at_tokens=20_000,
            summary_key="gao:conversation:summary",
        ),
        loop="host",
    ).compile()


def load_profile(connection: ProviderConnection | None = None):
    """Load the profile and fail if its Python artifact is invalid."""

    profile = build_profile(connection)
    result = profile.validate()
    if not result.valid:
        raise ValueError("invalid Gao profile: " + "; ".join(result.errors))
    return profile


@compile()
def gao_turn(g: GraphRecorder, user_message: str):
    """One live turn: retrieve papers, prepare a canvas, then answer."""

    papers = g.invoke_capability(search_papers, query="{user_message}")
    canvas = g.invoke_capability(plan_workflow, request="{user_message}")
    answer = g.ask(
        name="answer",
        prompt=(
            "User question:\n{user_message}\n\n"
            "Local paper search:\n{papers}\n\n"
            "Studio draft:\n{canvas}"
        ),
        system_prompt=_persona(),
        capability_groups=capability_groups(),
    )
    g.done(source=answer)


def _first_match(paper_result: dict[str, object]) -> dict[str, object] | None:
    matches = paper_result.get("matches")
    if not isinstance(matches, list) or not matches:
        return None
    first = matches[0]
    return first if isinstance(first, dict) else None


def run_live_turn(
    question: str,
    *,
    responder: Callable[[str], str] | None = None,
) -> LiveTurnResult:
    """Run the deterministic profile turn used by local acceptance checks.

    A production host supplies the model responder through APXM runtime. The
    default responder keeps this example deterministic and makes source use
    visible without network access.
    """

    paper_result = json.loads(search_papers(question))
    canvas = json.loads(plan_workflow(question))
    match = _first_match(paper_result)
    prompt = json.dumps(
        {"question": question, "papers": paper_result, "canvas": canvas},
        sort_keys=True,
    )
    if responder is None:
        if match is None:
            answer = "No source found in the local paper corpus."
        else:
            answer = f"APXM uses dependency-driven capability graphs. Source: {match['citation']}"
    else:
        answer = responder(prompt)
    return LiveTurnResult(answer=answer, paper_result=paper_result, canvas=canvas)


__all__ = [
    "GAO_CAPABILITY_TOOL_BINDINGS",
    "LiveTurnResult",
    "ProviderConnection",
    "SEARCH_WEB_CAPABILITY",
    "build_profile",
    "capability_groups",
    "gao_turn",
    "load_profile",
    "plan_workflow",
    "provider_capabilities",
    "run_live_turn",
    "search_papers",
]


if __name__ == "__main__":
    print(load_profile().to_air())
