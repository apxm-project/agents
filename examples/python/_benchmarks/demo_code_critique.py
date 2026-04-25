#!/usr/bin/env python3
"""Composite code-critique demo graph for APXM x vLLM benchmarking.

This is a pragmatic benchmark graph, not a full talk-packaging artifact. It
captures the intended shape from the killer-demo notes:

- one shared retrieval/setup step
- three parallel draft generations
- a merge barrier
- one critical-path reasoning step plus one critique-prep step
- three parallel critique passes
- one final refinement
- one final print

Usage:
  dekk apxm execute examples/python/_benchmarks/demo_code_critique.py -O0
  dekk apxm execute examples/python/_benchmarks/demo_code_critique.py -O2

To emit AIR directly:
  python3 examples/python/_benchmarks/demo_code_critique.py
"""

from __future__ import annotations

import sys
from pathlib import Path


def _bootstrap_repo_python_path() -> None:
    repo_root = Path(__file__).resolve().parents[3]
    frontend_python = repo_root / "crates" / "compiler" / "apxm-frontend" / "python"
    if frontend_python.exists():
        sys.path.insert(0, str(frontend_python))


_bootstrap_repo_python_path()

from apxm import GraphRecorder, compile

from _config import VLLM, VLLM_ROUTE


SHARED_CONTEXT = """You are reviewing a Python web service that creates orders.

Repository notes:
- Runtime: FastAPI + SQLAlchemy + PostgreSQL
- Traffic goal: 500 RPS with p99 under 80 ms
- Current incident history: duplicate order creation during retries,
  blocking database calls in request handlers, and weak payload validation

Current implementation:
```python
from fastapi import FastAPI, HTTPException
from sqlalchemy.orm import Session
from .db import get_db
from .models import User, Order

app = FastAPI()

@app.post("/users/{uid}/orders")
async def create_order(uid: int, payload: dict):
    db: Session = next(get_db())
    user = db.query(User).get(uid)
    if not user:
        raise HTTPException(404, "user not found")
    order = Order(user_id=uid, total=payload["total"])
    db.add(order)
    db.commit()
    return {"order_id": order.id}
```

Known issues:
- no schema validation for payload shape or total bounds
- `next(get_db())` bypasses request-scoped cleanup
- synchronous ORM use inside an async route
- no idempotency key or replay protection
- no transaction retry strategy
"""


@compile(
    default_provider=VLLM,
    default_route=VLLM_ROUTE,
)
def demo_code_critique(g: GraphRecorder):
    """Build the composite code-critique demo graph."""

    rag_query = g.ask(
        name="RAG_Query",
        prompt=SHARED_CONTEXT
        + "\n\nList the most relevant FastAPI, SQLAlchemy, and API reliability"
        " practices that should shape a production-grade rewrite."
        " Keep the answer concise and implementation-focused.",
    )

    gen_v1 = g.ask(
        name="Gen_v1",
        prompt=SHARED_CONTEXT
        + "\n\nReference notes:\n{rag_query}\n\n"
        "Draft version 1 of `create_order` with strong request validation"
        " and clear error handling.",
    )
    gen_v2 = g.ask(
        name="Gen_v2",
        prompt=SHARED_CONTEXT
        + "\n\nReference notes:\n{rag_query}\n\n"
        "Draft version 2 of `create_order` optimized for async database usage"
        " and latency.",
    )
    gen_v3 = g.ask(
        name="Gen_v3",
        prompt=SHARED_CONTEXT
        + "\n\nReference notes:\n{rag_query}\n\n"
        "Draft version 3 of `create_order` with replay protection,"
        " transaction safety, and auditability.",
    )

    merge_drafts = g.merge("Merge_Drafts", gen_v1, gen_v2, gen_v3)

    react_l1 = g.reason(
        name="ReAct_L1",
        prompt="You are selecting the strongest design direction.\n\n"
        "Candidate drafts:\n{merge_drafts}\n\n"
        "Pick the best ideas to carry forward and explain the tradeoffs"
        " across correctness, latency, and operational safety.",
    )
    react_prep = g.ask(
        name="ReAct_Prep",
        prompt="Using these source notes:\n{rag_query}\n\n"
        "Define three short critique prompts for reviewing a code change:"
        " correctness, performance, and security.",
    )

    critique_v1 = g.think(
        name="Critique_v1",
        prompt="Primary proposal:\n{react_l1}\n\n"
        "Critique checklist:\n{react_prep}\n\n"
        "Run the correctness critique and call out concrete risks.",
    )
    critique_v2 = g.think(
        name="Critique_v2",
        prompt="Primary proposal:\n{react_l1}\n\n"
        "Critique checklist:\n{react_prep}\n\n"
        "Run the performance critique and call out concrete risks.",
    )
    critique_v3 = g.think(
        name="Critique_v3",
        prompt="Primary proposal:\n{react_l1}\n\n"
        "Critique checklist:\n{react_prep}\n\n"
        "Run the security critique and call out concrete risks.",
    )

    refine_loop = g.reason(
        name="Refine_Loop",
        prompt="Synthesize a final production-ready recommendation for"
        " `create_order`.\n\n"
        "Selection notes:\n{react_l1}\n\n"
        "Correctness critique:\n{critique_v1}\n\n"
        "Performance critique:\n{critique_v2}\n\n"
        "Security critique:\n{critique_v3}\n\n"
        "Return a final implementation plan plus the key code-level changes.",
    )

    final_output = g.print(
        name="Print_Final",
        message="=== APXM GRAPH: CODE CRITIQUE DEMO ===\n\n{refine_loop}",
    )
    g.done(final_output)


if __name__ == "__main__":
    print(demo_code_critique._graph.to_air())
