"""The Python and TypeScript conversational examples agree after canonicalization."""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path

EXAMPLE_DIR = Path(__file__).resolve().parents[1]
FRONTEND_PACKAGE_DIR = EXAMPLE_DIR.parents[2] / ".apxm" / "frontend-example-python"
PYTHON_SOURCE = EXAMPLE_DIR / "python" / "agent.py"
TYPESCRIPT_SOURCE = EXAMPLE_DIR / "src" / "conversational-agent.ts"


def _python_environment() -> dict[str, str]:
    """Keep the packed frontend importable after the subprocess changes cwd."""
    environment = os.environ.copy()
    inherited = environment.get("PYTHONPATH", "")
    environment["PYTHONPATH"] = os.pathsep.join(
        value for value in (str(FRONTEND_PACKAGE_DIR), inherited) if value
    )
    return environment


def python_air() -> dict:
    result = subprocess.run(
        ["python", "python/agent.py", "--air"],
        cwd=EXAMPLE_DIR,
        env=_python_environment(),
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def python_graph() -> dict:
    result = subprocess.run(
        ["python", "python/agent.py"],
        cwd=EXAMPLE_DIR,
        env=_python_environment(),
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def python_diagnostics() -> object:
    result = subprocess.run(
        ["python", "python/agent.py", "--diagnostics"],
        cwd=EXAMPLE_DIR,
        env=_python_environment(),
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def typescript_air() -> dict:
    result = subprocess.run(
        [
            "node",
            "--input-type=module",
            "-e",
            "import { buildConversational } from './dist/index.js';"
            "console.log(buildConversational().canonicalAir());",
        ],
        cwd=EXAMPLE_DIR,
        env=_python_environment(),
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def typescript_graph() -> dict:
    result = subprocess.run(
        [
            "node",
            "--input-type=module",
            "-e",
            "import { buildConversational } from './dist/index.js';"
            "console.log(JSON.stringify(buildConversational().frontendGraph()));",
        ],
        cwd=EXAMPLE_DIR,
        env=_python_environment(),
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def typescript_diagnostics() -> object:
    result = subprocess.run(
        [
            "node",
            "--input-type=module",
            "-e",
            "import { buildConversational } from './dist/index.js';"
            "console.log(JSON.stringify(buildConversational().diagnostics()));",
        ],
        cwd=EXAMPLE_DIR,
        env=_python_environment(),
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def python_artifact() -> dict:
    result = subprocess.run(
        [
            "python",
            "-c",
            "import json; from agent import ConversationalExample; "
            "print(json.dumps(ConversationalExample.artifact()))",
        ],
        cwd=EXAMPLE_DIR / "python",
        env=_python_environment(),
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def typescript_artifact() -> dict:
    result = subprocess.run(
        [
            "node",
            "--input-type=module",
            "-e",
            "import { buildConversational } from './dist/index.js';"
            "console.log(JSON.stringify(buildConversational().artifact()));",
        ],
        cwd=EXAMPLE_DIR,
        env=_python_environment(),
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def _python_parity(mode: str = "") -> object:
    command = ["python", "python/parity.py"]
    if mode:
        command.append(mode)
    result = subprocess.run(
        command,
        cwd=EXAMPLE_DIR,
        env=_python_environment(),
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def _typescript_parity(method: str) -> object:
    result = subprocess.run(
        [
            "node",
            "--input-type=module",
            "-e",
            "import { buildParityCorpus } from './dist/parity-agent.js';"
            f"const value = buildParityCorpus().{method}();"
            "console.log(typeof value === 'string' ? value : JSON.stringify(value));",
        ],
        cwd=EXAMPLE_DIR,
        env=_python_environment(),
        capture_output=True,
        text=True,
    )
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout)


def _canonical_shape(air: dict) -> dict:
    """Project the semantics both languages must share, ignoring source spans."""
    return {
        "semantic_operations": [
            {
                "op": op["op"],
                "operand_slots": sorted(o["slot"] for o in op.get("operands", [])),
            }
            for op in air["semantic_operations"]
        ],
        "structural_ir": [node["kind"] for node in air["structural_ir"]],
    }


def _closed_semantics(air: dict) -> dict:
    """Return executable AIR semantics with language-local source maps removed.

    Source maps intentionally differ by language (paths, spans, source_language).
    Closed program meaning is the remaining AIR: operations, structural IR,
    context flow, and stable value/type identities.
    """
    return {
        key: value
        for key, value in air.items()
        if key != "source_map"
    }


def _conversational_semantics(air: dict) -> dict:
    semantics = _closed_semantics(air)
    semantics["context_flow"] = [
        {
            "context_type_ref": edge["context_type_ref"],
            "to_node": edge["to_node"],
        }
        for edge in semantics["context_flow"]
    ]
    return semantics


def _frontend_intent(graph: dict) -> dict:
    """Project typed FrontendGraph intent without language-specific spans or ids."""
    declarations = [
        {
            key: declaration.get(key)
            for key in (
                "decl_kind",
                "input_type_ref",
                "output_type_ref",
                "target_ref",
                "context_default_present",
            )
            if key in declaration
        }
        for declaration in graph["declarations"]
    ]
    return {
        "declarations": sorted(declarations, key=lambda declaration: json.dumps(declaration, sort_keys=True)),
        "call_intents": [call["intent_kind"] for call in graph["call_intents"]],
        "control_intents": [control["control_kind"] for control in graph["control_intents"]],
        "capability_requirements": graph["capability_requirements"],
        "model_requirements": graph["model_requirements"],
        "context_flow": [edge["context_type_ref"] for edge in graph["context_flow"]],
        "imported_program_refs": [
            {
                key: reference[key]
                for key in ("program_ref", "entrypoint", "target_agent_identity_requirement")
            }
            for reference in graph["imported_program_refs"]
        ],
    }


def _complete_frontend_shape(graph: dict) -> dict:
    """Project every typed semantic relation while ignoring ids and spans."""
    regions = {region["region_id"]: region for region in graph["regions"]}
    values = {value["value_id"]: value for value in graph["values"]}
    declarations = {
        declaration["decl_id"]: declaration for declaration in graph["declarations"]
    }
    slots_by_consumer: dict[str, list[str]] = {}
    for edge in graph["data_edges"]:
        slots_by_consumer.setdefault(edge["to_consumer"], []).append(
            edge["consumer_slot"]
        )

    def declaration_shape(declaration: dict) -> dict:
        return {
            key: declaration[key]
            for key in (
                "decl_kind",
                "input_type_ref",
                "output_type_ref",
                "target_ref",
                "context_default_present",
            )
            if key in declaration
        }

    def result_shape(node: dict) -> dict | None:
        result_id = node.get("result_value")
        if result_id is None:
            return None
        value = values[result_id]
        return {"type_ref": value["type_ref"], "origin": value["origin"]}

    calls = []
    for call in graph["call_intents"]:
        binding = declarations.get(call.get("binding_ref"))
        calls.append(
            {
                "intent_kind": call["intent_kind"],
                "binding": declaration_shape(binding) if binding else None,
                "receiver_kind": call.get("receiver_kind"),
                "parent_region_role": regions[call["parent_region_id"]]["region_role"],
                "operand_slots": sorted(slots_by_consumer.get(call["node_id"], [])),
                "result": result_shape(call),
            }
        )

    controls = []
    for control in graph["control_intents"]:
        controls.append(
            {
                "control_kind": control["control_kind"],
                "parent_region_role": regions[control["parent_region_id"]]["region_role"],
                "body_region_roles": [
                    regions[region_id]["region_role"]
                    for region_id in control.get("body_region_ids", [])
                ],
                "operand_slots": sorted(
                    slots_by_consumer.get(control["node_id"], [])
                ),
                "result": result_shape(control),
            }
        )

    return {
        "program": {
            key: graph["program_definitions"][0][key]
            for key in (
                "program_id",
                "input_type_ref",
                "output_type_ref",
                "context_type_ref",
                "has_default_context",
            )
        },
        "declarations": sorted(
            (declaration_shape(value) for value in declarations.values()),
            key=lambda value: json.dumps(value, sort_keys=True),
        ),
        "calls": calls,
        "controls": controls,
        "region_roles": sorted(region["region_role"] for region in regions.values()),
        "context_flow": sorted(
            edge["context_type_ref"] for edge in graph["context_flow"]
        ),
        "hooks": [
            {
                key: hook[key]
                for key in (
                    "scope",
                    "phase",
                    "input_type_ref",
                    "output_type_ref",
                    "return_mode",
                )
            }
            for hook in graph["hook_bindings"]
        ],
        "capability_requirements": graph["capability_requirements"],
        "model_requirements": graph["model_requirements"],
        "imported_program_refs": [
            {
                key: reference[key]
                for key in (
                    "program_ref",
                    "entrypoint",
                    "target_agent_identity_requirement",
                )
            }
            for reference in graph["imported_program_refs"]
        ],
    }


def test_conversational_examples_lower_equivalently() -> None:
    python = python_air()
    typescript = typescript_air()
    assert python["schema_version"] == "apxm.air"
    assert typescript["schema_version"] == "apxm.air"
    assert _canonical_shape(python) == _canonical_shape(typescript)
    assert _conversational_semantics(python) == _conversational_semantics(typescript), json.dumps(
        {
            "python": _conversational_semantics(python),
            "typescript": _conversational_semantics(typescript),
        },
        indent=2,
        sort_keys=True,
    )


def test_conversational_examples_record_equivalent_frontend_graph_intent_and_diagnostics() -> None:
    assert _frontend_intent(python_graph()) == _frontend_intent(typescript_graph())
    assert python_diagnostics() is None
    assert typescript_diagnostics() is None


def test_conversational_example_uses_only_generic_operations() -> None:
    air = python_air()
    ops = {op["op"] for op in air["semantic_operations"]}
    assert ops == {"model.call", "capability.invoke"}
    structural = [node["kind"] for node in air["structural_ir"]]
    assert structural.count("ais.loop") == 2
    assert "yield" in structural


def test_conversational_tool_dispatch_is_model_directed_and_closed() -> None:
    for graph in (python_graph(), typescript_graph()):
        loops = [
            control
            for control in graph["control_intents"]
            if control["control_kind"] == "loop"
        ]
        assert len(loops) == 2
        outer_loop, tool_loop = loops
        outer_region = outer_loop["body_region_ids"][0]
        tool_region = tool_loop["body_region_ids"][0]
        assert tool_loop["parent_region_id"] == outer_region

        model_calls = [
            call
            for call in graph["call_intents"]
            if call["intent_kind"] == "model_invocation"
        ]
        tool_calls = [
            call
            for call in graph["call_intents"]
            if call["intent_kind"] == "tool_invocation"
        ]
        assert len(model_calls) == 2
        assert len(tool_calls) == 1
        initial_model, reentry_model = model_calls
        tool_call = tool_calls[0]
        assert initial_model["parent_region_id"] == outer_region
        assert tool_loop["predicate"]["comparator"] == "equals"
        assert tool_loop["predicate"]["property_path"] == ["kind"]
        assert tool_loop["predicate"]["literal"] == {
            "scalar_type": "string",
            "value": "tool_request",
        }
        assert len(tool_loop["operand_values"]) == 2

        dispatch = next(
            control
            for control in graph["control_intents"]
            if control["control_kind"] == "conditional"
            and control["parent_region_id"] == tool_region
        )
        assert dispatch["predicate"]["comparator"] == "equals"
        assert dispatch["predicate"]["property_path"] == ["tool_request", "kind"]
        assert dispatch["predicate"]["literal"] == {
            "scalar_type": "string",
            "value": "search_web",
        }
        declared_arm, rejected_arm = dispatch["body_region_ids"]
        assert tool_call["parent_region_id"] == declared_arm
        assert reentry_model["parent_region_id"] == declared_arm
        assert tool_call["execution_order"] < reentry_model["execution_order"]
        assert any(
            control["control_kind"] == "throw"
            and control["parent_region_id"] == rejected_arm
            for control in graph["control_intents"]
        )

        hooks = graph["hook_bindings"]
        assert [hook["phase"] for hook in hooks] == ["before", "after"]
        assert {hook["scope"] for hook in hooks} == {"capability"}
        assert {hook["target_selector"] for hook in hooks} == {
            tool_call["node_id"]
        }
        assert graph["capability_requirements"] == [
            {"capability_ref": "cap.search", "tool_schema_present": True}
        ]


def test_conversational_source_passes_history_and_tool_results_to_the_model() -> None:
    python = PYTHON_SOURCE.read_text()
    typescript = TYPESCRIPT_SOURCE.read_text()

    assert 'while response["kind"] == "tool_request":' in python
    assert 'while (response.kind === "tool_request")' in typescript
    assert '"messages": agent.context.messages' in python
    assert 'messages: agent.context.messages' in typescript
    assert '"incoming": incoming' in python
    assert 'incoming,' in typescript
    assert '"tool_result": tool_result' in python
    assert 'tool_result: toolResult' in typescript
    assert 'last_reply=response["reply"]["message"]' in python
    assert 'last_reply: response.reply.message' in typescript

    for source in (python, typescript):
        assert "ResearchSpecialist" not in source
        assert ".new(" not in source
        assert ".invoke(" not in source


def test_conversational_artifacts_bind_and_schedule_hooks_deterministically() -> None:
    for artifact in (python_artifact(), typescript_artifact()):
        hooks = artifact["hook_bindings"]
        assert [hook["phase"] for hook in hooks] == ["before", "after"]
        assert [hook["handler_ref"] for hook in hooks] == [
            "PrepareSearchContext",
            "RecordSearchContext",
        ]
        assert {hook["scope"] for hook in hooks} == {"capability"}
        assert all(hook["handler_digest"].startswith("sha256:") for hook in hooks)

        air = artifact["air"]
        tool_call = next(
            operation
            for operation in air["semantic_operations"]
            if operation["op"] == "capability.invoke"
        )
        wrappers = {
            node["region_id"]: node
            for node in air["structural_ir"]
            if node["region_id"]
            in {"hook.PrepareSearchContext", "hook.RecordSearchContext"}
        }
        assert wrappers.keys() == {
            "hook.PrepareSearchContext",
            "hook.RecordSearchContext",
        }
        assert (
            wrappers["hook.PrepareSearchContext"]["execution_order"]
            < tool_call["execution_order"]
            < wrappers["hook.RecordSearchContext"]["execution_order"]
        )
        assert {
            wrappers["hook.PrepareSearchContext"]["parent_region_id"],
            wrappers["hook.RecordSearchContext"]["parent_region_id"],
        } == {tool_call["parent_region_id"]}


def test_conversational_context_update_precedes_yield_and_resume() -> None:
    for graph in (python_graph(), typescript_graph()):
        yield_control = next(
            control
            for control in graph["control_intents"]
            if control["control_kind"] == "yield"
        )
        assert len(graph["context_flow"]) == 1
        context_edge = graph["context_flow"][0]
        assert context_edge["to_node"] == yield_control["node_id"]
        assert context_edge["context_type_ref"] == "ConversationContext"
        context_value = next(
            value
            for value in graph["values"]
            if value["value_id"] == context_edge["value_id"]
        )
        assert context_value["origin"] == "context_value"
        assert next(
            field
            for field in context_value["expression"]["fields"]
            if field["name"] == "last_reply"
        )["value"]["property_path"] == ["reply", "message"]
        resume_value = yield_control["result_value"]
        assert next(
            value for value in graph["values"] if value["value_id"] == resume_value
        )["origin"] == "resume_input"
        assert any(
            resume_value in block["block_arguments"] for block in graph["blocks"]
        )

        tool_call = next(
            call for call in graph["call_intents"] if call["intent_kind"] == "tool_invocation"
        )
        tool_arguments = next(
            value
            for value in graph["values"]
            if value["value_id"] == tool_call["operand_values"][0]
        )["expression"]
        assert tool_arguments["kind"] == "projection"
        assert tool_arguments["property_path"] == ["tool_request", "arguments"]

        model_calls = [
            call for call in graph["call_intents"] if call["intent_kind"] == "model_invocation"
        ]
        reentry_request = next(
            value
            for value in graph["values"]
            if value["value_id"] == model_calls[1]["operand_values"][0]
        )["expression"]
        assert next(
            field for field in reentry_request["fields"] if field["name"] == "tool_result"
        )["value"] == {"kind": "ssa", "value_id": tool_call["result_value"]}


def test_paired_corpus_records_equivalent_complete_frontend_intent() -> None:
    python = _python_parity()
    typescript = _typescript_parity("frontendGraph")
    python_shape = _complete_frontend_shape(python)
    typescript_shape = _complete_frontend_shape(typescript)
    assert python_shape == typescript_shape, json.dumps(
        {"python": python_shape, "typescript": typescript_shape},
        indent=2,
        sort_keys=True,
    )
    assert _python_parity("--diagnostics") is None
    assert _typescript_parity("diagnostics") is None


def test_paired_corpus_covers_every_public_operation_and_structural_family() -> None:
    python = _python_parity("--air")
    typescript = _typescript_parity("canonicalAir")
    assert _canonical_shape(python) == _canonical_shape(typescript)
    assert _closed_semantics(python) == _closed_semantics(typescript), json.dumps(
        {"python": _closed_semantics(python), "typescript": _closed_semantics(typescript)},
        indent=2,
        sort_keys=True,
    )
    assert {operation["op"] for operation in python["semantic_operations"]} == {
        "model.call",
        "capability.invoke",
        "program.new",
        "program.invoke",
        "await.event",
    }
    assert {
        "ais.loop",
        "branch",
        "parallel_join",
        "try",
        "yield",
    } <= {node["kind"] for node in python["structural_ir"]}
