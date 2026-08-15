"""Static capture of an authored Agent callback into the bound tree.

Reads the decorated ``async def`` with the standard :mod:`ast` module, binds the
imported declaration markers by symbol identity, and folds the supported subset
of control flow into an immutable :class:`BoundProgram`. It never executes the
callback body: behavior is read from syntax and resolved declarations, not from
running one path. Stable node and region identities come from the program's
canonical identity plus lexical preorder, never from author-supplied strings.
"""

from __future__ import annotations

import ast
import inspect
import textwrap
from dataclasses import replace
from pathlib import Path
from typing import Any, Optional, Union

from ._bound_tree import (
    BoundCall,
    BoundCapabilityRequirement,
    BoundContextEdge,
    BoundControl,
    BoundDeclaration,
    BoundHook,
    BoundOperand,
    BoundParameter,
    BoundPredicate,
    BoundProgram,
    BoundRegion,
    BoundValue,
    Span,
)
from ._advanced import HookDecl, TaskGroup
from ._generated.frontend_graph import (
    HOOK_RETURN_MODE_OBSERVE,
    HOOK_RETURN_MODE_REPLACE_RESULT,
    HOOK_SCOPE_AGENT,
    HOOK_SCOPE_LOOP,
    REGION_ROLE_HOOK_BODY,
)
from ._generated.capabilities import READ_SKILL
from ._generated.diagnostics import (
    AGENT_DYNAMIC_ARGUMENT,
    AGENT_BODY_NOT_ASYNC,
    AGENT_MISSING_INPUT_OUTPUT,
    CAPABILITY_REF_NOT_EXACT,
    CONTEXT_NOT_TYPED,
    DiagnosticCode,
    EVENT_NOT_TYPED,
    HOOK_ORDER_AMBIGUOUS,
    HOOK_TARGET_UNRESOLVED,
    SKILL_LOAD_OUTSIDE_BODY,
)
from ._generated.frontend_records import CallIntent, ControlIntent, SkillRequirement
from ._markers import (
    CapabilityBinding,
    ContextSchema,
    EventType,
    ModelBinding,
    SkillDecl,
    ToolBinding,
)

#: The declaration every `Skill(...).load()` invokes. One synthetic Capability
#: binding serves every skill a program declares: loading instructions is one
#: authority, not one per skill, and it is the authority the artifact states.
SKILL_READER_DECL_ID = "decl.capability.read_skill"

#: The typed interface of that binding. A load takes the skill's identity and
#: returns its instruction document.
SKILL_READER_INPUT = "SkillRequest"
SKILL_READER_OUTPUT = "SkillInstructions"


class CaptureError(ValueError):
    """A source construct outside the supported authoring subset."""

    def __init__(
        self, code: DiagnosticCode, message: str, node: Optional[ast.AST] = None
    ) -> None:
        self.code = code
        location = ""
        if node is not None and hasattr(node, "lineno"):
            location = f" (line {node.lineno})"
        super().__init__(f"{code}: {message}{location}")


class _Capture:
    """Folds one Agent callback function into a bound program."""

    def __init__(
        self,
        program_id: str,
        entrypoint: str,
        input_type_ref: str,
        output_type_ref: str,
        context_type_ref: Optional[str],
        has_default_context: bool,
        bindings: dict[str, Any],
        source_file: str,
    ) -> None:
        self.program_id = program_id
        self.entrypoint = entrypoint
        self.input_type_ref = input_type_ref
        self.output_type_ref = output_type_ref
        self.context_type_ref = context_type_ref
        self.has_default_context = has_default_context
        self.bindings = bindings
        self.source_file = source_file

        self.body_region_id = f"{program_id}.body"
        self.declarations: list[BoundDeclaration] = []
        self.values: list[BoundValue] = []
        self.regions: list[BoundRegion] = []
        self.calls: list[BoundCall] = []
        self.controls: list[BoundControl] = []
        self.context_edges: list[BoundContextEdge] = []
        self.hooks: list[BoundHook] = []
        self.imported_programs: list[tuple[str, str, str, str]] = []
        self.capability_requirements: list[BoundCapabilityRequirement] = []
        self.model_requirements: list[str] = []
        self.skill_requirements: list[SkillRequirement] = []
        self._skills_by_name: dict[str, str] = {}
        self.spans: list[tuple[str, Span, str]] = []

        self._facade_name: Optional[str] = None
        self._input_name: Optional[str] = None
        self._counter = 0
        self._order: dict[str, int] = {}
        self._declared: dict[str, str] = {}
        self._values_by_name: dict[str, str] = {}
        self._instance_programs: dict[str, str] = {}
        self._last_node_by_region: dict[str, str] = {}
        self._pending_context_by_region: dict[str, tuple[str, str]] = {}
        self._referenced_names: set[str] = set()
        self._hook_body_regions: set[str] = set()
        self._hook_context_assignment: dict[str, str] = {}

    def _next(self, prefix: str) -> str:
        self._counter += 1
        return f"{self.program_id}.{prefix}.{self._counter}"

    def _order_in(self, region_id: str) -> int:
        order = self._order.get(region_id, 0)
        self._order[region_id] = order + 1
        return order

    def _span(self, node: ast.AST) -> Optional[Span]:
        if not hasattr(node, "lineno"):
            return None
        end_line = getattr(node, "end_lineno", node.lineno) or node.lineno
        end_col = getattr(node, "end_col_offset", node.col_offset + 1)
        return Span(self.source_file, node.lineno, node.col_offset, end_line, end_col)

    def _require_capability(
        self,
        binding: Union[ToolBinding, CapabilityBinding],
        *,
        tool_schema_present: bool,
    ) -> None:
        """Record one authored Capability declaration in declaration order.

        Requirements are held as a list rather than keyed by ``capability_ref``
        so a reference declared both as a Tool and as a plain Capability keeps
        both declarations. Only a declaration identical in every field collapses.
        """
        requirement = BoundCapabilityRequirement(
            capability_ref=binding.target_ref,
            tool_schema_present=tool_schema_present,
            requested_permission=binding.permission,
        )
        if requirement not in self.capability_requirements:
            self.capability_requirements.append(requirement)

    def _declare_bindings(self) -> None:
        for name in sorted(self.bindings):
            binding = self.bindings[name]
            if (
                name not in self._referenced_names
                and not (
                    isinstance(binding, ContextSchema)
                    and binding.type_ref == self.context_type_ref
                )
                and not isinstance(binding, HookDecl)
            ):
                continue
            if isinstance(binding, ModelBinding):
                decl_id = f"decl.model.{name}"
                self._declared[name] = decl_id
                self.declarations.append(
                    BoundDeclaration(
                        decl_id=decl_id,
                        decl_kind="model_binding",
                        input_type_ref=binding.input_type_ref,
                        output_type_ref=binding.output_type_ref,
                        target_ref=binding.target_ref,
                    )
                )
                self.model_requirements.append(binding.target_ref)
            elif isinstance(binding, ToolBinding):
                decl_id = f"decl.tool.{name}"
                self._declared[name] = decl_id
                self.declarations.append(
                    BoundDeclaration(
                        decl_id=decl_id,
                        decl_kind="tool_binding",
                        input_type_ref=binding.input_type_ref,
                        output_type_ref=binding.output_type_ref,
                        target_ref=binding.target_ref,
                    )
                )
                self._require_capability(binding, tool_schema_present=True)
            elif isinstance(binding, CapabilityBinding):
                decl_id = f"decl.capability.{name}"
                self._declared[name] = decl_id
                self.declarations.append(
                    BoundDeclaration(
                        decl_id=decl_id,
                        decl_kind="capability_binding",
                        input_type_ref=binding.input_type_ref,
                        output_type_ref=binding.output_type_ref,
                        target_ref=binding.target_ref,
                    )
                )
                self._require_capability(binding, tool_schema_present=False)
            elif isinstance(binding, EventType):
                decl_id = f"decl.event.{name}"
                self._declared[name] = decl_id
                self.declarations.append(
                    BoundDeclaration(
                        decl_id=decl_id,
                        decl_kind="event_type",
                        input_type_ref=binding.type_ref,
                        output_type_ref=binding.type_ref,
                        target_ref=binding.target_ref,
                    )
                )
            elif isinstance(binding, ContextSchema):
                decl_id = f"decl.context.{name}"
                self._declared[name] = decl_id
                self.declarations.append(
                    BoundDeclaration(
                        decl_id=decl_id,
                        decl_kind="context",
                        input_type_ref=binding.type_ref,
                        output_type_ref=binding.type_ref,
                        context_default_present=binding.default_present,
                    )
                )
            elif isinstance(binding, SkillDecl):
                self._skills_by_name[name] = binding.skill_id
                self.skill_requirements.append(
                    SkillRequirement(
                        skill_id=binding.skill_id,
                        instruction_source=binding.instruction_source,
                    )
                )
            elif isinstance(binding, HookDecl):
                if binding.handler_ref is None or binding.handler_digest is None:
                    raise CaptureError(
                        HOOK_DYNAMIC_REGISTRATION,
                        f"Hook '{name}' is missing its decorated async handler",
                    )
            else:
                program_reference = getattr(binding, "_program_reference", None)
                if program_reference is not None and name in self._referenced_names:
                    program_ref, digest, entrypoint, identity_requirement = program_reference
                    self._declared[name] = program_ref
                    if not any(ref == program_ref for ref, *_ in self.imported_programs):
                        self.imported_programs.append(
                            (program_ref, digest, entrypoint, identity_requirement)
                        )
        if self.skill_requirements:
            self._declare_skill_reader()

    def _declare_skill_reader(self) -> None:
        """Declare the one Capability every ``Skill(...).load()`` invokes.

        It is declared last, after the author's own bindings, so both frontends
        place it identically. One binding serves every declared skill: reading
        instructions is a single authority the artifact states once, not one
        authority per skill.
        """
        self.declarations.append(
            BoundDeclaration(
                decl_id=SKILL_READER_DECL_ID,
                decl_kind="capability_binding",
                input_type_ref=SKILL_READER_INPUT,
                output_type_ref=SKILL_READER_OUTPUT,
                target_ref=READ_SKILL,
            )
        )
        requirement = BoundCapabilityRequirement(
            capability_ref=READ_SKILL, tool_schema_present=False
        )
        if requirement not in self.capability_requirements:
            self.capability_requirements.append(requirement)

    def capture(self, func_ast: ast.AsyncFunctionDef) -> BoundProgram:
        # A Hook body is captured source too, so the Models and Capabilities it
        # calls have to be declared alongside the ones the Agent body calls.
        sources: list[ast.AST] = [func_ast]
        sources.extend(
            handler
            for handler in (
                _handler_ast(binding)
                for binding in self.bindings.values()
                if isinstance(binding, HookDecl)
            )
            if handler is not None
        )
        self._referenced_names = {
            node.id
            for source in sources
            for node in ast.walk(source)
            if isinstance(node, ast.Name) and isinstance(node.ctx, ast.Load)
        }
        self._declare_bindings()

        args = func_ast.args.args
        if len(args) < 2:
            raise CaptureError(
                AGENT_MISSING_INPUT_OUTPUT,
                "an Agent callback takes an inferred agent facade and a typed input",
                func_ast,
            )
        self._facade_name = args[0].arg
        self._input_name = args[1].arg
        parameters = (
            BoundParameter(
                value_id=f"{self.program_id}.param.agent",
                type_ref="AgentFacade",
                role="agent_facade",
            ),
            BoundParameter(
                value_id=f"{self.program_id}.param.input",
                type_ref=self.input_type_ref,
                role="input",
            ),
        )
        self.values.append(
            BoundValue(
                value_id=f"{self.program_id}.param.agent",
                type_ref="AgentFacade",
                origin="parameter",
                origin_id=self.entrypoint,
            )
        )
        self.values.append(
            BoundValue(
                value_id=f"{self.program_id}.param.input",
                type_ref=self.input_type_ref,
                origin="parameter",
                origin_id=self.entrypoint,
            )
        )
        self._values_by_name[self._input_name] = f"{self.program_id}.param.input"

        self.regions.append(
            BoundRegion(
                region_id=self.body_region_id,
                region_role="function_body",
                parent_region_id=None,
                execution_order=0,
            )
        )
        self._visit_block(func_ast.body, self.body_region_id)
        self._capture_hooks()

        return BoundProgram(
            program_id=self.program_id,
            entrypoint=self.entrypoint,
            input_type_ref=self.input_type_ref,
            output_type_ref=self.output_type_ref,
            has_default_context=self.has_default_context,
            context_type_ref=self.context_type_ref,
            parameters=parameters,
            body_region_id=self.body_region_id,
            declarations=tuple(self.declarations),
            values=tuple(self.values),
            regions=tuple(self.regions),
            calls=tuple(self.calls),
            controls=tuple(self.controls),
            context_edges=tuple(self.context_edges),
            hooks=tuple(self.hooks),
            imported_programs=tuple(self.imported_programs),
            capability_requirements=tuple(self.capability_requirements),
            model_requirements=tuple(self.model_requirements),
            skill_requirements=tuple(self.skill_requirements),
            spans=tuple(self.spans),
        )

    def _visit_block(self, body: list[ast.stmt], region_id: str) -> None:
        for stmt in body:
            self._visit_stmt(stmt, region_id)

    def _visit_stmt(self, stmt: ast.stmt, region_id: str) -> None:
        if isinstance(stmt, ast.Expr) and isinstance(stmt.value, ast.Await):
            self._visit_await(stmt.value, region_id, assign_to=None)
        elif isinstance(stmt, ast.Expr) and isinstance(stmt.value, ast.Call):
            self._visit_new(stmt.value, region_id, assign_to="_")
        elif isinstance(stmt, ast.Assign):
            self._visit_assign(stmt, region_id)
        elif isinstance(stmt, ast.AsyncWith):
            self._visit_task_group(stmt, region_id)
        elif isinstance(stmt, (ast.While, ast.For)):
            self._visit_loop(stmt, region_id)
        elif isinstance(stmt, ast.If):
            self._visit_conditional(stmt, region_id)
        elif isinstance(stmt, ast.Return):
            self._visit_return(stmt, region_id)
        elif isinstance(stmt, ast.Raise):
            self._visit_throw(stmt, region_id)
        elif isinstance(stmt, ast.Try):
            self._visit_try(stmt, region_id)
        elif isinstance(stmt, (ast.Import, ast.ImportFrom, ast.Pass)):
            return
        # A docstring is prose about the source, not source intent. TypeScript
        # carries the same prose as trivia the parser already drops, so refusing
        # it here would make a documented Python body uncapturable and an
        # identically documented TypeScript body fine.
        elif isinstance(stmt, ast.Expr) and isinstance(stmt.value, ast.Constant):
            if not isinstance(stmt.value.value, str):
                raise CaptureError(
                    AGENT_DYNAMIC_ARGUMENT,
                    "a bare constant is not an Agent source statement", stmt
                )
            return
        elif isinstance(stmt, ast.AnnAssign) and stmt.value is None:
            return
        else:
            raise CaptureError(
                AGENT_DYNAMIC_ARGUMENT,
                f"unsupported statement '{type(stmt).__name__}' in Agent source",
                stmt,
            )

    def _visit_assign(self, stmt: ast.Assign, region_id: str) -> None:
        target = stmt.targets[0]
        # agent.context = ... produces an explicit typed Context transition.
        if (
            isinstance(target, ast.Attribute)
            and isinstance(target.value, ast.Name)
            and target.value.id == self._facade_name
            and target.attr == "context"
        ):
            value_id = self._next("context")
            expression = self._expression_for_value(stmt.value)
            self.values.append(
                BoundValue(
                    value_id=value_id,
                    type_ref=self.context_type_ref or "Context",
                    origin="context_value",
                    expression=expression,
                )
            )
            # Inside a captured Hook body the assignment *is* the Hook's return
            # value, so it is carried by the binding rather than wired to the
            # next node in the region. That is what makes an observing Hook
            # distinguishable from a replacing one by looking at the body.
            if region_id in self._hook_body_regions:
                self._hook_context_assignment[region_id] = value_id
                return
            # A context replacement before the first effect is anchored to the
            # lexical region entry so it remains executable rather than being
            # silently discarded.
            source = self._last_node_by_region.get(region_id, region_id)
            self._pending_context_by_region[region_id] = (source, value_id)
            return
        # name = await <call>  binds the call result to a named value.
        if isinstance(stmt.value, ast.Await) and isinstance(target, ast.Name):
            self._visit_await(stmt.value, region_id, assign_to=target.id)
            return
        if isinstance(stmt.value, ast.Await):
            self._visit_await(stmt.value, region_id, assign_to=None)
            return
        if isinstance(stmt.value, ast.Call) and isinstance(target, ast.Name):
            self._visit_new(stmt.value, region_id, assign_to=target.id)
            return
        # Ordinary local binding of a literal or expression is not an effect.
        return

    def _visit_await(
        self, await_node: ast.Await, region_id: str, assign_to: Optional[str]
    ) -> None:
        call = await_node.value
        if not isinstance(call, ast.Call):
            raise CaptureError(
                AGENT_DYNAMIC_ARGUMENT, "await targets a typed effect call", await_node
            )

        intent, binding_ref, receiver_kind, slot = self._resolve_call_target(call)
        if intent == "yield":
            self._visit_yield(call, region_id, assign_to)
            return
        node_id = self._next(intent)
        result_value: Optional[str] = None
        if intent == "event_wait":
            result_value = self._next("resume")
            self.values.append(
                BoundValue(
                    value_id=result_value,
                    type_ref=self._result_type(binding_ref),
                    origin="call_result",
                    origin_id=node_id,
                )
            )
        elif assign_to is not None or intent != "event_wait":
            result_value = self._next("value")
            self.values.append(
                BoundValue(
                    value_id=result_value,
                    type_ref=self._result_type(binding_ref),
                    origin="call_result",
                    origin_id=node_id,
                )
            )

        operands = self._call_operands(call, node_id, slot, receiver_kind)
        self.calls.append(
            BoundCall(
                contract=CallIntent(
                    node_id=node_id,
                    intent_kind=intent,
                    parent_region_id=region_id,
                    execution_order=self._order_in(region_id),
                    binding_ref=binding_ref,
                    receiver_kind=receiver_kind,
                    result_value=result_value,
                ),
                span=self._span(call),
                operands=tuple(operands),
            )
        )
        self._record_node(region_id, node_id)
        if assign_to is not None and result_value is not None:
            self._values_by_name[assign_to] = result_value
        span = self._span(call)
        if span is not None:
            self.spans.append((node_id, span, intent))

    def _resolve_call_target(
        self, call: ast.Call
    ) -> tuple[str, Optional[str], Optional[str], str]:
        func = call.func
        # instance.invoke(...) or Definition.invoke(...) -> agent_invocation
        if isinstance(func, ast.Attribute):
            if (
                func.attr == "yield_"
                and isinstance(func.value, ast.Name)
                and func.value.id == self._facade_name
            ):
                return "yield", None, None, "output"
            if func.attr == "invoke":
                receiver_kind = "program_instance_ref"
                if isinstance(func.value, ast.Name) and func.value.id in self._declared:
                    receiver_kind = "program_ref"
                binding_ref = self._binding_of(func.value)
                if binding_ref not in {reference[0] for reference in self.imported_programs}:
                    raise CaptureError(
                        AGENT_DYNAMIC_ARGUMENT,
                        "Agent.invoke resolves one static Agent reference",
                        call,
                    )
                return "agent_invocation", binding_ref, receiver_kind, "input"
            if func.attr == "new":
                binding_ref = self._binding_of(func.value)
                if binding_ref not in {reference[0] for reference in self.imported_programs}:
                    raise CaptureError(
                        AGENT_DYNAMIC_ARGUMENT,
                        "Agent.new resolves one static Agent reference",
                        call,
                    )
                return "agent_creation", binding_ref, None, "initial_context"
            if func.attr == "load":
                if self._skill_id_of_load(call) is None:
                    raise CaptureError(
                        SKILL_LOAD_OUTSIDE_BODY,
                        "load receiver is a statically declared Skill",
                        call,
                    )
                return (
                    "capability_invocation",
                    SKILL_READER_DECL_ID,
                    None,
                    "arguments",
                )
            if func.attr == "wait":
                binding_ref = self._binding_of(func.value)
                if binding_ref not in {decl.decl_id for decl in self.declarations if decl.decl_kind == "event_type"}:
                    raise CaptureError(
                        EVENT_NOT_TYPED,
                        "Event.wait resolves one static Event value",
                        call,
                    )
                return "event_wait", binding_ref, None, "event_ref"
            raise CaptureError(
                AGENT_DYNAMIC_ARGUMENT, f"unsupported call '.{func.attr}(...)'", call
            )
        if isinstance(func, ast.Name):
            binding = self.bindings.get(func.id)
            decl_id = self._declared.get(func.id)
            if isinstance(binding, ModelBinding):
                return "model_invocation", decl_id, None, "request"
            if isinstance(binding, ToolBinding):
                return "tool_invocation", decl_id, None, "arguments"
            if isinstance(binding, CapabilityBinding):
                return "capability_invocation", decl_id, None, "arguments"
            raise CaptureError(
                CAPABILITY_REF_NOT_EXACT,
                f"call target '{func.id}' is not a bound Model, Tool, or Capability",
                call,
            )
        raise CaptureError(
            AGENT_DYNAMIC_ARGUMENT,
            "computed or dynamic call targets are not supported",
            call,
        )

    def _binding_of(self, node: ast.AST) -> Optional[str]:
        if isinstance(node, ast.Name):
            return self._instance_programs.get(node.id, self._declared.get(node.id, node.id))
        return None

    def _result_type(self, binding_ref: Optional[str]) -> str:
        for decl in self.declarations:
            if decl.decl_id == binding_ref:
                return decl.output_type_ref
        return self.output_type_ref

    def _record_node(self, region_id: str, node_id: str) -> None:
        """Connect a pending Context replacement to its next static boundary."""
        pending = self._pending_context_by_region.pop(region_id, None)
        if pending is not None and pending[0] != node_id:
            source, value_id = pending
            self.context_edges.append(
                BoundContextEdge(
                    from_node=source,
                    to_node=node_id,
                    context_type_ref=self.context_type_ref or "Context",
                    value_id=value_id,
                )
            )
        self._last_node_by_region[region_id] = node_id

    def _reject_unbound_calls(self, expression: ast.AST) -> None:
        """Reject every call in a value expression that is not a Context literal.

        A value expression stands for data, not for an effect. The only call a
        value position admits is constructing a bound ``Context`` schema. A
        Model, Tool, or Capability call is an effect and reaches the graph only
        through ``await``; any other callee is unresolved. Both are rejected
        here so an unresolved or effectful call can never be silently folded
        into an untyped literal operand.
        """
        for node in ast.walk(expression):
            if not isinstance(node, ast.Call):
                continue
            callee = node.func
            if isinstance(callee, ast.Name):
                binding = self.bindings.get(callee.id)
                if isinstance(binding, ContextSchema):
                    continue
                if isinstance(
                    binding, (ModelBinding, ToolBinding, CapabilityBinding)
                ):
                    raise CaptureError(
                        CAPABILITY_REF_NOT_EXACT,
                        f"'{callee.id}' is a typed effect and is called with await, "
                        "not used as a value",
                        node,
                    )
                raise CaptureError(
                    CONTEXT_NOT_TYPED,
                    f"call target '{callee.id}' is not a bound Context schema",
                    node,
                )
            raise CaptureError(
                CONTEXT_NOT_TYPED,
                "value expressions call only a bound Context schema by name",
                node,
            )

    def _value_for_expression(self, expression: ast.AST, node_id: str) -> str:
        """Resolve one source expression to a prior value or a typed literal."""
        if isinstance(expression, ast.Name) and expression.id in self._values_by_name:
            return self._values_by_name[expression.id]
        assembled = self._expression_for_value(expression)
        value_id = self._next("value")
        self.values.append(
            BoundValue(
                value_id=value_id,
                type_ref="ArgumentValue",
                origin="literal",
                expression=assembled,
            )
        )
        return value_id

    def _expression_for_value(self, expression: ast.AST) -> dict[str, object]:
        """Capture one pure authored value without executing or flattening it."""
        self._reject_unbound_calls(expression)
        if isinstance(expression, ast.Name) and expression.id in self._values_by_name:
            return {"kind": "ssa", "value_id": self._values_by_name[expression.id]}
        if isinstance(expression, ast.Constant):
            if expression.value is None:
                return {"kind": "null"}
            if isinstance(expression.value, bool):
                return {"kind": "boolean", "value": expression.value}
            if isinstance(expression.value, int):
                if abs(expression.value) > 9_007_199_254_740_991:
                    raise CaptureError(
                        AGENT_DYNAMIC_ARGUMENT,
                        "integer exceeds the shared safe integer domain",
                        expression,
                    )
                return {"kind": "integer", "value": expression.value}
            if isinstance(expression.value, str):
                return {"kind": "string", "value": expression.value}
            raise CaptureError(
                AGENT_DYNAMIC_ARGUMENT,
                "value expression admits only JSON scalar literals",
                expression,
            )
        if (
            isinstance(expression, ast.UnaryOp)
            and isinstance(expression.op, ast.USub)
            and isinstance(expression.operand, ast.Constant)
            and isinstance(expression.operand.value, int)
            and not isinstance(expression.operand.value, bool)
        ):
            value = -expression.operand.value
            if abs(value) > 9_007_199_254_740_991:
                raise CaptureError(
                    AGENT_DYNAMIC_ARGUMENT,
                    "integer exceeds the shared safe integer domain",
                    expression,
                )
            return {"kind": "integer", "value": value}
        if isinstance(expression, (ast.Dict,)):
            fields: list[dict[str, object]] = []
            for key, value in zip(expression.keys, expression.values, strict=True):
                if not isinstance(key, ast.Constant) or not isinstance(key.value, str):
                    raise CaptureError(
                        AGENT_DYNAMIC_ARGUMENT,
                        "object expression keys are static strings",
                        expression,
                    )
                fields.append({"name": key.value, "value": self._expression_for_value(value)})
            return {"kind": "object", "fields": fields}
        if isinstance(expression, (ast.List, ast.Tuple)):
            if any(isinstance(item, ast.Starred) for item in expression.elts):
                raise CaptureError(
                    AGENT_DYNAMIC_ARGUMENT,
                    "array expression does not admit spreads",
                    expression,
                )
            return {
                "kind": "array",
                "items": [self._expression_for_value(item) for item in expression.elts],
            }
        if isinstance(expression, ast.Call) and isinstance(expression.func, ast.Name):
            binding = self.bindings.get(expression.func.id)
            if isinstance(binding, ContextSchema):
                if expression.args:
                    raise CaptureError(
                        CONTEXT_NOT_TYPED, "Context values use named fields", expression
                    )
                fields = []
                for keyword in expression.keywords:
                    if keyword.arg is None:
                        raise CaptureError(
                            CONTEXT_NOT_TYPED,
                            "Context values do not admit spreads",
                            expression,
                        )
                    fields.append(
                        {"name": keyword.arg, "value": self._expression_for_value(keyword.value)}
                    )
                return {"kind": "object", "fields": fields}
        projection = self._projection_parts(expression)
        if projection is not None:
            root, path = projection
            if root == "context":
                return {"kind": "context", "property_path": path}
            return {
                "kind": "projection",
                "root": {"kind": "ssa", "value_id": root},
                "property_path": path,
            }
        raise CaptureError(
            AGENT_DYNAMIC_ARGUMENT, "unsupported pure value expression", expression
        )

    def _projection_parts(self, expression: ast.AST) -> Optional[tuple[str, list[str]]]:
        path: list[str] = []
        node = expression
        while True:
            if (
                isinstance(node, ast.Attribute)
                and isinstance(node.value, ast.Name)
                and node.value.id == self._facade_name
                and node.attr == "context"
            ):
                return ("context", path)
            if isinstance(node, ast.Attribute):
                path.insert(0, node.attr)
                node = node.value
                continue
            if (
                isinstance(node, ast.Subscript)
                and isinstance(node.slice, ast.Constant)
                and isinstance(node.slice.value, str)
            ):
                path.insert(0, node.slice.value)
                node = node.value
                continue
            break
        if isinstance(node, ast.Name) and node.id in self._values_by_name and path:
            return (self._values_by_name[node.id], path)
        return None

    def _skill_id_of_load(self, call: ast.Call) -> Optional[str]:
        """The skill a ``<name>.load()`` call names, when it names one."""
        func = call.func
        if (
            isinstance(func, ast.Attribute)
            and func.attr == "load"
            and isinstance(func.value, ast.Name)
        ):
            return self._skills_by_name.get(func.value.id)
        return None

    def _skill_argument_value(self, skill_id: str) -> str:
        """The typed argument a skill load carries: the skill's own identity.

        The author writes no operand, so one is assembled here rather than left
        out. It is an ordinary authored literal, which is what keeps the loaded
        skill visible in the graph, in AIR, and to anything reading the effect's
        operands.
        """
        value_id = self._next("value")
        self.values.append(
            BoundValue(
                value_id=value_id,
                type_ref="ArgumentValue",
                origin="literal",
                expression={
                    "kind": "object",
                    "fields": [
                        {"name": "skill_id", "value": {"kind": "string", "value": skill_id}}
                    ],
                },
            )
        )
        return value_id

    def _call_operands(
        self,
        call: ast.Call,
        node_id: str,
        slot: str,
        receiver_kind: Optional[str],
    ) -> list[BoundOperand]:
        operands: list[BoundOperand] = []
        skill_id = self._skill_id_of_load(call)
        if skill_id is not None:
            if call.args or call.keywords:
                raise CaptureError(
                    SKILL_LOAD_OUTSIDE_BODY,
                    "a Skill load takes no authored operand",
                    call,
                )
            return [
                BoundOperand(value_id=self._skill_argument_value(skill_id), slot=slot)
            ]
        argument_count = len(call.args) + len(call.keywords)
        if argument_count == 0:
            return operands
        if argument_count != 1:
            raise CaptureError(
                AGENT_DYNAMIC_ARGUMENT,
                "typed effect calls accept exactly one authored operand",
                call,
            )
        if call.keywords and call.keywords[0].arg is None:
            raise CaptureError(
                AGENT_DYNAMIC_ARGUMENT,
                "typed effect operands do not admit spreads",
                call,
            )
        expression = call.args[0] if call.args else call.keywords[0].value
        value_id = self._value_for_expression(expression, node_id)
        operands.append(BoundOperand(value_id=value_id, slot=slot))
        return operands

    def _visit_yield(
        self, call: ast.Call, region_id: str, assign_to: Optional[str]
    ) -> None:
        """Capture the explicit stateful yield boundary and its resume input."""
        if region_id in self._hook_body_regions:
            raise CaptureError(
                HOOK_DYNAMIC_REGISTRATION,
                "a Hook body does not park the Agent invocation",
                call,
            )
        if assign_to is None:
            raise CaptureError(
                AGENT_MISSING_INPUT_OUTPUT,
                "agent.yield_ assigns its typed resume input",
                call,
            )
        node_id = self._next("yield")
        resume_value = self._next("resume")
        self.values.append(
            BoundValue(
                value_id=resume_value,
                type_ref=self.input_type_ref,
                origin="resume_input",
                origin_id=node_id,
            )
        )
        self.controls.append(
            BoundControl(
                contract=ControlIntent(
                    node_id=node_id,
                    control_kind="yield",
                    parent_region_id=region_id,
                    execution_order=self._order_in(region_id),
                    result_value=resume_value,
                ),
                span=self._span(call),
                operands=tuple(self._call_operands(call, node_id, "output", None)),
                predicate=None,
            )
        )
        self._values_by_name[assign_to] = resume_value
        self._record_node(region_id, node_id)
        span = self._span(call)
        if span is not None:
            self.spans.append((node_id, span, "yield"))

    def _visit_new(self, call: ast.Call, region_id: str, assign_to: str) -> None:
        """Capture Agent.new as one typed Program Instance creation intent."""
        intent, binding_ref, receiver_kind, slot = self._resolve_call_target(call)
        if intent != "agent_creation":
            raise CaptureError(
                AGENT_DYNAMIC_ARGUMENT,
                "only Agent.new may initialize a program instance",
                call,
            )
        if binding_ref is None or binding_ref not in {
            reference[0] for reference in self.imported_programs
        }:
            raise CaptureError(
                AGENT_DYNAMIC_ARGUMENT,
                "Agent.new resolves one statically imported Agent",
                call,
            )
        node_id = self._next(intent)
        result_value = self._next("instance")
        self.values.append(
            BoundValue(
                value_id=result_value,
                type_ref="ProgramInstanceRef",
                origin="call_result",
                origin_id=node_id,
            )
        )
        self.calls.append(
            BoundCall(
                contract=CallIntent(
                    node_id=node_id,
                    intent_kind=intent,
                    parent_region_id=region_id,
                    execution_order=self._order_in(region_id),
                    binding_ref=binding_ref,
                    result_value=result_value,
                ),
                span=self._span(call),
                operands=tuple(self._call_operands(call, node_id, slot, receiver_kind)),
            )
        )
        self._values_by_name[assign_to] = result_value
        self._instance_programs[assign_to] = binding_ref
        self._record_node(region_id, node_id)
        span = self._span(call)
        if span is not None:
            self.spans.append((node_id, span, intent))

    def _visit_loop(self, stmt: ast.stmt, region_id: str) -> None:
        node_id = self._next("loop")
        body_region = f"{node_id}.body"
        predicate = (
            self._predicate_for_expression(stmt.test, node_id)
            if isinstance(stmt, ast.While)
            else None
        )
        source_name = (
            next(
                (
                    name
                    for name, value_id in self._values_by_name.items()
                    if predicate is not None and value_id == predicate.root_value_id
                ),
                None,
            )
            if predicate is not None
            else None
        )
        control_index = len(self.controls)
        self.controls.append(
            BoundControl(
                contract=ControlIntent(
                    node_id=node_id,
                    control_kind="loop",
                    parent_region_id=region_id,
                    execution_order=self._order_in(region_id),
                    body_region_ids=(body_region,),
                ),
                span=self._span(stmt),
                operands=(),
                predicate=predicate,
            )
        )
        self._record_node(region_id, node_id)
        self.regions.append(
            BoundRegion(
                region_id=body_region,
                region_role="loop_body",
                parent_region_id=region_id,
                execution_order=self._order_in(region_id),
            )
        )
        operands: tuple[BoundOperand, ...] = ()
        result_value = None
        if predicate is not None and source_name is not None:
            initial_value = predicate.root_value_id
            result_value = self._next("value")
            carried_type = next(
                value.type_ref for value in self.values if value.value_id == initial_value
            )
            self.values.append(
                BoundValue(
                    value_id=result_value,
                    type_ref=carried_type,
                    origin="block_argument",
                    origin_id=f"{body_region}.block.0",
                )
            )
            self._values_by_name[source_name] = result_value
            predicate = BoundPredicate(
                root_value_id=result_value,
                property_path=predicate.property_path,
                comparator=predicate.comparator,
                literal=predicate.literal,
            )
        self._visit_block(stmt.body, body_region)
        if result_value is not None and source_name is not None:
            carried_value = self._values_by_name.get(source_name)
            if carried_value is not None:
                operands = (
                    BoundOperand(value_id=initial_value, slot="initial"),
                    BoundOperand(value_id=carried_value, slot="carried"),
                )
            self._values_by_name[source_name] = result_value
        self.controls[control_index] = BoundControl(
            contract=ControlIntent(
                node_id=node_id,
                control_kind="loop",
                parent_region_id=region_id,
                execution_order=self._order_in(region_id),
                body_region_ids=(body_region,),
                result_value=result_value,
            ),
            span=self._span(stmt),
            operands=operands,
            predicate=predicate,
        )

    def _predicate_projection(self, expression: ast.AST) -> tuple[str, tuple[str, ...]]:
        if isinstance(expression, ast.Name) and expression.id in self._values_by_name:
            return self._values_by_name[expression.id], ()
        if isinstance(expression, ast.Attribute):
            root, path = self._predicate_projection(expression.value)
            return root, (*path, expression.attr)
        if (
            isinstance(expression, ast.Subscript)
            and isinstance(expression.slice, ast.Constant)
            and isinstance(expression.slice.value, str)
        ):
            root, path = self._predicate_projection(expression.value)
            return root, (*path, expression.slice.value)
        raise CaptureError(
            AGENT_DYNAMIC_ARGUMENT,
            "control predicate reads a prior typed value through static properties",
            expression,
        )

    def _predicate_literal(self, expression: ast.AST) -> dict[str, object]:
        try:
            value = ast.literal_eval(expression)
        except (ValueError, TypeError, SyntaxError):
            raise CaptureError(
                AGENT_DYNAMIC_ARGUMENT,
                "predicate equality compares with a scalar literal",
                expression,
            )
        if isinstance(value, bool):
            return {"scalar_type": "boolean", "value": value}
        if isinstance(value, str):
            return {"scalar_type": "string", "value": value}
        if isinstance(value, int):
            if abs(value) > 9_007_199_254_740_991:
                raise CaptureError(
                    AGENT_DYNAMIC_ARGUMENT,
                    "predicate integer literal is within the shared safe-integer domain",
                    expression,
                )
            return {"scalar_type": "integer", "value": value}
        if value is None:
            return {"scalar_type": "null"}
        raise CaptureError(
            AGENT_DYNAMIC_ARGUMENT,
            "predicate literal is boolean, string, integer, or null",
            expression,
        )

    def _predicate_for_expression(
        self, expression: ast.AST, node_id: str
    ) -> Optional[BoundPredicate]:
        if isinstance(expression, ast.Constant) and expression.value is True:
            return None
        if (
            isinstance(expression, ast.Compare)
            and len(expression.ops) == 1
            and isinstance(expression.ops[0], (ast.Eq, ast.NotEq, ast.Is, ast.IsNot))
            and len(expression.comparators) == 1
        ):
            root_value_id, property_path = self._predicate_projection(expression.left)
            return BoundPredicate(
                root_value_id=root_value_id,
                property_path=property_path,
                comparator="not_equals"
                if isinstance(expression.ops[0], (ast.NotEq, ast.IsNot))
                else "equals",
                literal=self._predicate_literal(expression.comparators[0]),
            )
        root_value_id, property_path = self._predicate_projection(expression)
        return BoundPredicate(
            root_value_id=root_value_id,
            property_path=property_path,
            comparator="truthy",
        )

    def _visit_task_group(self, stmt: ast.AsyncWith, region_id: str) -> None:
        """Capture one lexical TaskGroup scope whose exit joins all child work."""
        if len(stmt.items) != 1:
            raise CaptureError(AGENT_DYNAMIC_ARGUMENT, "TaskGroup has one static scope expression", stmt)
        expression = stmt.items[0].context_expr
        if not (
            isinstance(expression, ast.Call)
            and isinstance(expression.func, ast.Name)
            and self.bindings.get(expression.func.id) is TaskGroup
        ):
            raise CaptureError(AGENT_DYNAMIC_ARGUMENT, "async with uses the imported TaskGroup marker", stmt)
        if expression.args or expression.keywords:
            raise CaptureError(
                AGENT_DYNAMIC_ARGUMENT,
                "TaskGroup accepts no dynamic constructor arguments",
                expression,
            )

        node_id = self._next("task_group")
        scope_region = f"{node_id}.scope"
        self.controls.append(
            BoundControl(
                contract=ControlIntent(
                    node_id=node_id,
                    control_kind="task_group",
                    parent_region_id=region_id,
                    execution_order=self._order_in(region_id),
                    body_region_ids=(scope_region,),
                ),
                span=self._span(stmt),
                operands=(),
                predicate=None,
            )
        )
        self._record_node(region_id, node_id)
        self.regions.append(
            BoundRegion(
                region_id=scope_region,
                region_role="task_scope",
                parent_region_id=region_id,
                execution_order=self._order_in(region_id),
            )
        )
        self._visit_block(stmt.body, scope_region)

    def _visit_conditional(self, stmt: ast.If, region_id: str) -> None:
        node_id = self._next("cond")
        self.controls.append(
            BoundControl(
                contract=ControlIntent(
                    node_id=node_id,
                    control_kind="conditional",
                    parent_region_id=region_id,
                    execution_order=self._order_in(region_id),
                    body_region_ids=(f"{node_id}.then", f"{node_id}.else")
                    if stmt.orelse
                    else (f"{node_id}.then",),
                ),
                span=self._span(stmt),
                operands=(),
                predicate=self._predicate_for_expression(stmt.test, node_id),
            )
        )
        self._record_node(region_id, node_id)
        then_region = f"{node_id}.then"
        self.regions.append(
            BoundRegion(
                region_id=then_region,
                region_role="conditional_arm",
                parent_region_id=region_id,
                execution_order=self._order_in(region_id),
            )
        )
        self._visit_block(stmt.body, then_region)
        if stmt.orelse:
            else_region = f"{node_id}.else"
            self.regions.append(
                BoundRegion(
                    region_id=else_region,
                    region_role="conditional_arm",
                    parent_region_id=region_id,
                    execution_order=self._order_in(region_id),
                )
            )
            self._visit_block(stmt.orelse, else_region)

    def _visit_try(self, stmt: ast.Try, region_id: str) -> None:
        node_id = self._next("try")
        body_regions = [f"{node_id}.try"] + [
            f"{node_id}.catch.{index}"
            for index, _ in enumerate(stmt.handlers, start=1)
        ]
        self.controls.append(
            BoundControl(
                contract=ControlIntent(
                    node_id=node_id,
                    control_kind="try_catch",
                    parent_region_id=region_id,
                    execution_order=self._order_in(region_id),
                    body_region_ids=tuple(body_regions),
                ),
                span=self._span(stmt),
                operands=(),
                predicate=None,
            )
        )
        self._record_node(region_id, node_id)
        try_region = f"{node_id}.try"
        self.regions.append(
            BoundRegion(
                region_id=try_region,
                region_role="try_body",
                parent_region_id=region_id,
                execution_order=self._order_in(region_id),
            )
        )
        self._visit_block(stmt.body, try_region)
        for index, handler in enumerate(stmt.handlers, start=1):
            catch_region = f"{node_id}.catch.{index}"
            self.regions.append(
                BoundRegion(
                    region_id=catch_region,
                    region_role="catch_body",
                    parent_region_id=region_id,
                    execution_order=self._order_in(region_id),
                )
            )
            self._visit_block(handler.body, catch_region)

    def _visit_return(self, stmt: ast.Return, region_id: str) -> None:
        # A Hook body is not the Agent body: returning a value from it would
        # otherwise lower to a program return and end the whole invocation. A
        # Hook states its replacement by assigning Context, so a bare `return`
        # is a terminator with nothing to record and a returned value is refused.
        if region_id in self._hook_body_regions:
            if stmt.value is not None and not (
                isinstance(stmt.value, ast.Constant) and stmt.value.value is None
            ):
                raise CaptureError(
                    HOOK_DYNAMIC_REGISTRATION,
                    "a Hook returns its replacement by assigning agent.context",
                    stmt,
                )
            return
        node_id = self._next("return")
        operands: list[BoundOperand] = []
        if isinstance(stmt.value, ast.Await):
            self._visit_await(stmt.value, region_id, assign_to=None)
        elif stmt.value is not None:
            operands.append(
                BoundOperand(
                    value_id=self._value_for_expression(stmt.value, node_id),
                    slot="output",
                )
            )
        self.controls.append(
            BoundControl(
                contract=ControlIntent(
                    node_id=node_id,
                    control_kind="return",
                    parent_region_id=region_id,
                    execution_order=self._order_in(region_id),
                ),
                span=self._span(stmt),
                operands=tuple(operands),
                predicate=None,
            )
        )
        self._record_node(region_id, node_id)

    def _visit_throw(self, stmt: ast.Raise, region_id: str) -> None:
        node_id = self._next("throw")
        self.controls.append(
            BoundControl(
                contract=ControlIntent(
                    node_id=node_id,
                    control_kind="throw",
                    parent_region_id=region_id,
                    execution_order=self._order_in(region_id),
                ),
                span=self._span(stmt),
                operands=(),
                predicate=None,
            )
        )
        self._record_node(region_id, node_id)

    def _resolve_hook_target_name(self, name: str, declaration: HookDecl) -> HookDecl:
        """Give a Hook that named its target declaration the selector for it.

        The rest of Hook resolution works from the module name a declaration is
        bound to, which is what TypeScript reads off the authored identifier.
        Naming the declaration is the same statement, so it is turned into that
        name here rather than doubling every lookup below.
        """
        target = declaration.target_declaration
        if target is None:
            return declaration
        for bound, binding in self.bindings.items():
            if binding is target:
                return replace(declaration, target_selector=bound)
        raise CaptureError(
            HOOK_TARGET_UNRESOLVED,
            f"Hook '{name}' targets a declaration this module binds to no name",
        )

    def _names_a_hook_target(self, target: str) -> bool:
        """Whether a Hook target names something a module can actually declare.

        A target naming a declaration this Agent never calls belongs to another
        Agent in the same module and is skipped. A target naming nothing at all
        belongs to no Agent and would otherwise disappear, so the two are told
        apart here rather than folded into one silent drop.
        """
        if target in {self.program_id, self.entrypoint, "loop"}:
            return True
        binding = self.bindings.get(target)
        if getattr(binding, "_program_reference", None) is not None:
            return True
        return isinstance(
            binding,
            (ModelBinding, ToolBinding, CapabilityBinding, EventType, SkillDecl),
        )

    def _capture_hooks(self) -> None:
        """Bind Hook decorators and capture their bodies as ordinary regions.

        Hooks are visited in the order their module declared them, which is the
        order ``declaration_order`` states and therefore the order the schedule
        runs two same-phase Hooks on one target in. Numbering is dense over the
        Hooks this Agent keeps, so the order does not depend on how many Hooks
        the module wrote for some other Agent.
        """
        for name, declaration in self.bindings.items():
            if not isinstance(declaration, HookDecl):
                continue
            declaration = self._resolve_hook_target_name(name, declaration)
            if not self._names_a_hook_target(declaration.target_selector):
                raise CaptureError(
                    HOOK_TARGET_UNRESOLVED,
                    f"Hook '{name}' targets '{declaration.target_selector}', which "
                    "this module declares nowhere",
                )
            if (
                declaration.target_selector not in self._referenced_names
                and declaration.target_selector not in {self.program_id, self.entrypoint}
                and declaration.target_selector != "loop"
            ):
                continue
            if declaration.handler_ref is None or declaration.handler_digest is None:
                raise CaptureError(
                    HOOK_DYNAMIC_REGISTRATION,
                    f"Hook '{name}' is missing its decorated async handler",
                )
            hook_id = f"hook.{name}"
            target_selector = self._hook_target(declaration)
            body_region_id = self._capture_hook_body(hook_id, declaration, target_selector)
            assigned = self._hook_context_assignment.get(body_region_id)
            self.hooks.append(
                BoundHook(
                    hook_id=hook_id,
                    scope=declaration.scope,
                    phase=declaration.phase,
                    target_selector=target_selector,
                    declaration_order=len(self.hooks),
                    handler_ref=declaration.handler_ref,
                    handler_digest=declaration.handler_digest,
                    input_type_ref=declaration.input_type_ref,
                    output_type_ref=(
                        "Unit" if assigned is None else (self.context_type_ref or "Context")
                    ),
                    return_mode=(
                        HOOK_RETURN_MODE_OBSERVE
                        if assigned is None
                        else HOOK_RETURN_MODE_REPLACE_RESULT
                    ),
                    body_region_id=body_region_id,
                    assigned_context_value_id=assigned,
                )
            )

    def _capture_hook_body(
        self, hook_id: str, declaration: HookDecl, target_selector: str
    ) -> str:
        """Fold one Hook handler into its own region of the same bound tree.

        The body is read from syntax exactly like an Agent body, so a Hook's
        Capability and Model calls become the same typed intents and stay
        visible to the compiler. Where the region *runs* is decided by scope and
        phase during lowering, not by where the handler happened to be written.
        """
        handler_ast = _handler_ast(declaration)
        if handler_ast is None:
            raise CaptureError(
                HOOK_DYNAMIC_REGISTRATION,
                f"Hook '{hook_id}' handler source is not readable",
            )
        parameters = handler_ast.args.args
        if not parameters:
            raise CaptureError(
                HOOK_DYNAMIC_REGISTRATION,
                f"Hook '{hook_id}' takes the inferred agent facade",
            )

        body_region_id = f"{hook_id}.body"
        parent_region_id = self._hook_parent_region(target_selector)
        self.regions.append(
            BoundRegion(
                region_id=body_region_id,
                region_role=REGION_ROLE_HOOK_BODY,
                parent_region_id=parent_region_id,
                execution_order=self._order_in(parent_region_id),
            )
        )
        self._hook_body_regions.add(body_region_id)

        outer_facade = self._facade_name
        self._facade_name = parameters[0].arg
        try:
            self._visit_block(handler_ast.body, body_region_id)
        except CaptureError as error:
            raise CaptureError(
                error.code, f"in Hook '{hook_id}' body: {error}"
            ) from error
        finally:
            self._facade_name = outer_facade
        return body_region_id

    def _hook_parent_region(self, target_selector: str) -> str:
        """The region a Hook body is lexically held in.

        A node target puts the body beside the node; a region target — the Agent
        body or a loop body — puts it inside that region.
        """
        for call in self.calls:
            if call.node_id == target_selector:
                return call.parent_region_id
        for control in self.controls:
            if control.node_id == target_selector:
                return control.parent_region_id
        return target_selector

    def _hook_target(self, declaration: HookDecl) -> str:
        """Resolve a friendly Hook target to one captured intent or region."""
        target = declaration.target_selector
        if target in {region.region_id for region in self.regions}:
            return target
        binding_ref = self._declared.get(target)
        matches = (
            [call.node_id for call in self.calls if call.binding_ref == binding_ref]
            if binding_ref is not None
            else []
        )
        if declaration.scope == HOOK_SCOPE_AGENT:
            # The scope decides which boundary is wrapped, and the only boundary
            # an Agent scope has is the Agent's own body region. Resolving to the
            # invocation the target happened to name would emit a Hook lowering
            # refuses: it admits a region for this scope and nothing else.
            if target in {self.program_id, self.entrypoint} or matches:
                return self.body_region_id
            raise CaptureError(
                HOOK_TARGET_UNRESOLVED,
                f"Hook target '{target}' is not the Agent or a captured region",
            )
        if target in {call.node_id for call in self.calls}:
            return target
        if target in {control.node_id for control in self.controls}:
            return target
        if declaration.scope == HOOK_SCOPE_LOOP:
            # A loop is not a name an author can write, so a loop Hook selects
            # its loop through something inside it. Falling back to the first
            # loop only when nothing was named is what lets an authored target
            # reach an inner loop instead of being silently discarded.
            if len(matches) > 1:
                raise CaptureError(
                    HOOK_ORDER_AMBIGUOUS,
                    f"Hook target '{target}' is ambiguous across invocations",
                )
            if matches:
                loop_body = self._enclosing_loop_body(matches[0])
                if loop_body is None:
                    raise CaptureError(
                        HOOK_TARGET_UNRESOLVED,
                        f"Hook target '{target}' is not inside a static loop",
                    )
                return loop_body
            if target in {"loop", self.entrypoint}:
                for control in self.controls:
                    if control.control_kind == "loop" and control.body_region_ids:
                        return control.body_region_ids[0]
                raise CaptureError(
                    HOOK_TARGET_UNRESOLVED,
                    f"Hook target '{target}' has no loop body region",
                )
            raise CaptureError(
                HOOK_TARGET_UNRESOLVED,
                f"Hook target '{target}' has no captured invocation",
            )
        if binding_ref is not None:
            if len(matches) == 1:
                return matches[0]
            if not matches:
                raise CaptureError(
                    HOOK_TARGET_UNRESOLVED,
                    f"Hook target '{target}' has no captured invocation",
                )
            raise CaptureError(
                HOOK_ORDER_AMBIGUOUS,
                f"Hook target '{target}' is ambiguous across invocations",
            )
        raise CaptureError(
            HOOK_TARGET_UNRESOLVED,
            f"Hook target '{target}' is not a static Agent source target",
        )

    def _enclosing_loop_body(self, node_id: str) -> Optional[str]:
        """Walk out from one captured node to the loop body region holding it."""
        loop_bodies = {
            control.body_region_ids[0]
            for control in self.controls
            if control.control_kind == "loop" and control.body_region_ids
        }
        parents = {
            region.region_id: region.parent_region_id for region in self.regions
        }
        region_id: Optional[str] = self._hook_parent_region(node_id)
        while region_id is not None:
            if region_id in loop_bodies:
                return region_id
            region_id = parents.get(region_id)
        return None


def _handler_ast(declaration: HookDecl) -> Optional[ast.AsyncFunctionDef]:
    """Parse one Hook handler's own ``async def`` without evaluating it."""
    handler = declaration.handler
    if handler is None:
        return None
    try:
        source = textwrap.dedent(inspect.getsource(handler))
    except (OSError, TypeError):
        return None
    for node in ast.parse(source).body:
        if isinstance(node, ast.AsyncFunctionDef):
            return node
    return None


def capture_program(
    func: Any,
    *,
    program_id: str,
    input_type_ref: str,
    output_type_ref: str,
    context_type_ref: Optional[str],
    has_default_context: bool,
    bindings: dict[str, Any],
) -> BoundProgram:
    """Parse and fold one authored Agent callback into a bound program."""
    source = textwrap.dedent(inspect.getsource(func))
    module = ast.parse(source)
    func_ast: Optional[ast.AsyncFunctionDef] = None
    for node in module.body:
        if isinstance(node, ast.AsyncFunctionDef):
            func_ast = node
            break
    if func_ast is None:
        raise CaptureError(AGENT_BODY_NOT_ASYNC, "an Agent is one async def")
    source_file = _source_reference(func)

    capture = _Capture(
        program_id=program_id,
        entrypoint=func_ast.name,
        input_type_ref=input_type_ref,
        output_type_ref=output_type_ref,
        context_type_ref=context_type_ref,
        has_default_context=has_default_context,
        bindings=bindings,
        source_file=source_file,
    )
    return capture.capture(func_ast)


def _source_reference(func: Any) -> str:
    """Return a portable source reference for emitted source-map evidence."""
    try:
        source_file = inspect.getsourcefile(func)
    except TypeError:
        source_file = None
    if source_file is None:
        return "<agent>"
    source = Path(source_file).resolve()
    repository_root = Path(__file__).resolve().parents[5]
    try:
        return source.relative_to(repository_root).as_posix()
    except ValueError:
        return "<agent>"
