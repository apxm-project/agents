"""Typed Agent authoring in ordinary Python.

An everyday author needs five names — ``Agent``, ``Context``, ``Tool``,
``Model``, and ordinary control flow — plus the inferred ``agent`` callback
parameter. Advanced programs add ``Capability``, ``Event``, ``Hook``, and
``TaskGroup``. The frontend reads the authored source statically, builds an
immutable typed tree, and traverses it into the language-neutral FrontendGraph;
it never executes the authored body and exposes no operation constant, node or
region identity, or raw graph builder.
"""

from __future__ import annotations

from ._advanced import Hook, TaskGroup
from ._agent import Agent
from ._workflow import Program, Workflow
from ._markers import Capability, Context, Event, EventRef, Model, Skill, Tool

__all__ = [
    "Agent",
    "Capability",
    "Context",
    "Event",
    "EventRef",
    "Hook",
    "Model",
    "Program",
    "Skill",
    "TaskGroup",
    "Tool",
    "Workflow",
]
