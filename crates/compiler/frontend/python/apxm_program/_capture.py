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
from pathlib import Path
from typing import Any, Optional

from ._bound_tree import (
    BoundCall,
    BoundContextEdge,
    BoundControl,
    BoundDeclaration,
    BoundHook,
    BoundOperand,
    BoundParameter,
    BoundProgram,
    BoundRegion,
    BoundValue,
    Span,
)
from ._advanced import HookDecl, TaskGroup
from ._markers import (
    CapabilityBinding,
    ContextSchema,
    EventType,
    ModelBinding,
    ToolBinding,
)


class CaptureError(ValueError):
    """A source construct outside the supported authoring subset."""

    def __init__(self, message: str, node: Optional[ast.AST] = None) -> None:
        location = ""
        if node is not None and hasattr(node, "lineno"):
            location = f" (line {node.lineno})"
        super().__init__(f"{message}{location}")


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
        self.capability_requirements: dict[str, bool] = {}
        self.model_requirements: list[str] = []
        self.spans: list[tuple[str, Span, str]] = []

        self._facade_name: Optional[str] = None
        self._input_name: Optional[str] = None
        self._counter = 0
        self._order: dict[str, int] = {}
        self._declared: dict[str, str] = {}
        self._values_by_name: dict[str, str] = {}
        self._instance_programs: dict[str, str] = {}
        self._last_node_by_region: dict[str, str] = {}
        self._pending_context_by_region: dict[str, str] = {}
        self._referenced_names: set[str] = set()

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
                        decl_kind="tool_handler" if binding.handler_digest else "tool_binding",
                        input_type_ref=binding.input_type_ref,
                        output_type_ref=binding.output_type_ref,
                        target_ref=binding.target_ref,
                        handler_digest=binding.handler_digest,
                    )
                )
                self.capability_requirements[binding.target_ref] = True
            elif isinstance(binding, CapabilityBinding):
                decl_id = f"decl.capability.{name}"
                self._declared[name] = decl_id
                self.declarations.append(
                    BoundDeclaration(
                        decl_id=decl_id,
                        decl_kind="capability_handler"
                        if binding.handler_digest
                        else "capability_binding",
                        input_type_ref=binding.input_type_ref,
                        output_type_ref=binding.output_type_ref,
                        target_ref=binding.target_ref,
                        handler_digest=binding.handler_digest,
                    )
                )
                self.capability_requirements[binding.target_ref] = False
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
            elif isinstance(binding, HookDecl):
                if binding.handler_ref is None or binding.handler_digest is None:
                    raise CaptureError(f"Hook '{name}' is missing its decorated async handler")
            else:
                program_reference = getattr(binding, "_program_reference", None)
                if program_reference is not None and name in self._referenced_names:
                    program_ref, digest, entrypoint, identity_requirement = program_reference
                    self._declared[name] = program_ref
                    if not any(ref == program_ref for ref, *_ in self.imported_programs):
                        self.imported_programs.append(
                            (program_ref, digest, entrypoint, identity_requirement)
                        )

    def capture(self, func_ast: ast.AsyncFunctionDef) -> BoundProgram:
        self._referenced_names = {
            node.id
            for node in ast.walk(func_ast)
            if isinstance(node, ast.Name) and isinstance(node.ctx, ast.Load)
        }
        self._declare_bindings()

        args = func_ast.args.args
        if len(args) < 2:
            raise CaptureError(
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
            capability_requirements=tuple(self.capability_requirements.items()),
            model_requirements=tuple(self.model_requirements),
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
        elif isinstance(stmt, ast.Try):
            self._visit_try(stmt, region_id)
        elif isinstance(stmt, (ast.Import, ast.ImportFrom, ast.Pass)):
            return
        elif isinstance(stmt, ast.AnnAssign) and stmt.value is None:
            return
        else:
            raise CaptureError(
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
            self.values.append(
                BoundValue(
                    value_id=value_id,
                    type_ref=self.context_type_ref or "Context",
                    origin="context_value",
                )
            )
            last_node = self._last_node_by_region.get(region_id)
            if last_node is not None:
                self._pending_context_by_region[region_id] = last_node
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
            raise CaptureError("await targets a typed effect call", await_node)

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
                node_id=node_id,
                intent_kind=intent,
                parent_region_id=region_id,
                execution_order=self._order_in(region_id),
                binding_ref=binding_ref,
                operands=tuple(operands),
                result_value=result_value,
                span=self._span(call),
                receiver_kind=receiver_kind,
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
                    raise CaptureError("Agent.invoke resolves one static Agent reference", call)
                return "agent_invocation", binding_ref, receiver_kind, "input"
            if func.attr == "new":
                binding_ref = self._binding_of(func.value)
                if binding_ref not in {reference[0] for reference in self.imported_programs}:
                    raise CaptureError("Agent.new resolves one static Agent reference", call)
                return "agent_creation", binding_ref, None, "initial_context"
            if func.attr == "wait":
                binding_ref = self._binding_of(func.value)
                if binding_ref not in {decl.decl_id for decl in self.declarations if decl.decl_kind == "event_type"}:
                    raise CaptureError("Event.wait resolves one static Event value", call)
                return "event_wait", binding_ref, None, "event_ref"
            raise CaptureError(f"unsupported call '.{func.attr}(...)'", call)
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
                f"call target '{func.id}' is not a bound Model, Tool, or Capability",
                call,
            )
        raise CaptureError("computed or dynamic call targets are not supported", call)

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
        source = self._pending_context_by_region.pop(region_id, None)
        if source is not None and source != node_id:
            self.context_edges.append(
                BoundContextEdge(
                    from_node=source,
                    to_node=node_id,
                    context_type_ref=self.context_type_ref or "Context",
                )
            )
        self._last_node_by_region[region_id] = node_id

    def _value_for_expression(self, expression: ast.AST, node_id: str) -> str:
        """Resolve one source expression to a prior value or a typed literal."""
        if isinstance(expression, ast.Name) and expression.id in self._values_by_name:
            return self._values_by_name[expression.id]
        value_id = self._next("value")
        self.values.append(
            BoundValue(
                value_id=value_id,
                type_ref="ArgumentValue",
                origin="literal",
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
        if not call.args and not call.keywords:
            return operands
        expression = call.args[0] if call.args else call.keywords[0].value
        value_id = self._value_for_expression(expression, node_id)
        operands.append(BoundOperand(value_id=value_id, slot=slot))
        return operands

    def _visit_yield(
        self, call: ast.Call, region_id: str, assign_to: Optional[str]
    ) -> None:
        """Capture the explicit stateful yield boundary and its resume input."""
        if assign_to is None:
            raise CaptureError("agent.yield_ assigns its typed resume input", call)
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
                node_id=node_id,
                control_kind="yield",
                parent_region_id=region_id,
                execution_order=self._order_in(region_id),
                body_region_ids=(),
                operands=tuple(self._call_operands(call, node_id, "output", None)),
                result_value=resume_value,
                span=self._span(call),
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
            raise CaptureError("only Agent.new may initialize a program instance", call)
        if binding_ref is None or binding_ref not in {
            reference[0] for reference in self.imported_programs
        }:
            raise CaptureError("Agent.new resolves one statically imported Agent", call)
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
                node_id=node_id,
                intent_kind=intent,
                parent_region_id=region_id,
                execution_order=self._order_in(region_id),
                binding_ref=binding_ref,
                operands=tuple(self._call_operands(call, node_id, slot, receiver_kind)),
                result_value=result_value,
                span=self._span(call),
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
        control = BoundControl(
            node_id=node_id,
            control_kind="loop",
            parent_region_id=region_id,
            execution_order=self._order_in(region_id),
            body_region_ids=(body_region,),
            operands=(),
            result_value=None,
            span=self._span(stmt),
        )
        self.controls.append(control)
        self._record_node(region_id, node_id)
        self.regions.append(
            BoundRegion(
                region_id=body_region,
                region_role="loop_body",
                parent_region_id=region_id,
                execution_order=self._order_in(region_id),
            )
        )
        self._visit_block(stmt.body, body_region)

    def _visit_task_group(self, stmt: ast.AsyncWith, region_id: str) -> None:
        """Capture one lexical TaskGroup scope whose exit joins all child work."""
        if len(stmt.items) != 1:
            raise CaptureError("TaskGroup has one static scope expression", stmt)
        expression = stmt.items[0].context_expr
        if not (
            isinstance(expression, ast.Call)
            and isinstance(expression.func, ast.Name)
            and self.bindings.get(expression.func.id) is TaskGroup
        ):
            raise CaptureError("async with uses the imported TaskGroup marker", stmt)
        if expression.args or expression.keywords:
            raise CaptureError("TaskGroup accepts no dynamic constructor arguments", expression)

        node_id = self._next("task_group")
        scope_region = f"{node_id}.scope"
        self.controls.append(
            BoundControl(
                node_id=node_id,
                control_kind="task_group",
                parent_region_id=region_id,
                execution_order=self._order_in(region_id),
                body_region_ids=(scope_region,),
                operands=(),
                result_value=None,
                span=self._span(stmt),
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
                node_id=node_id,
                control_kind="conditional",
                parent_region_id=region_id,
                execution_order=self._order_in(region_id),
                body_region_ids=(f"{node_id}.then", f"{node_id}.else")
                if stmt.orelse
                else (f"{node_id}.then",),
                operands=(),
                result_value=None,
                span=self._span(stmt),
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
                node_id=node_id,
                control_kind="try_catch",
                parent_region_id=region_id,
                execution_order=self._order_in(region_id),
                body_region_ids=tuple(body_regions),
                operands=(),
                result_value=None,
                span=self._span(stmt),
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
                node_id=node_id,
                control_kind="return",
                parent_region_id=region_id,
                execution_order=self._order_in(region_id),
                body_region_ids=(),
                operands=tuple(operands),
                result_value=None,
                span=self._span(stmt),
            )
        )
        self._record_node(region_id, node_id)

    def _capture_hooks(self) -> None:
        """Bind Hook decorators after all static source targets are known."""
        for name in sorted(self.bindings):
            declaration = self.bindings[name]
            if not isinstance(declaration, HookDecl):
                continue
            if (
                declaration.target_selector not in self._referenced_names
                and declaration.target_selector not in {self.program_id, self.entrypoint}
                and declaration.target_selector != "loop"
            ):
                continue
            if declaration.handler_ref is None or declaration.handler_digest is None:
                raise CaptureError(f"Hook '{name}' is missing its decorated async handler")
            self.hooks.append(
                BoundHook(
                    hook_id=f"hook.{name}",
                    scope=declaration.scope,
                    phase=declaration.phase,
                    target_selector=self._hook_target(declaration),
                    declaration_order=len(self.hooks),
                    handler_ref=declaration.handler_ref,
                    handler_digest=declaration.handler_digest,
                    input_type_ref=declaration.input_type_ref,
                    output_type_ref=declaration.output_type_ref,
                    return_mode=declaration.return_mode,
                )
            )

    def _hook_target(self, declaration: HookDecl) -> str:
        """Resolve a friendly Hook target to one captured intent or region."""
        target = declaration.target_selector
        if target in {region.region_id for region in self.regions}:
            return target
        if target in {call.node_id for call in self.calls}:
            return target
        if target in {control.node_id for control in self.controls}:
            return target
        if declaration.scope == "agent" and target in {self.program_id, self.entrypoint}:
            return self.body_region_id
        if declaration.scope == "loop" and target in {"loop", self.entrypoint}:
            for control in self.controls:
                if control.control_kind == "loop":
                    return control.node_id
        binding_ref = self._declared.get(target)
        if binding_ref is not None:
            matches = [call.node_id for call in self.calls if call.binding_ref == binding_ref]
            if len(matches) == 1:
                return matches[0]
            if not matches:
                raise CaptureError(f"Hook target '{target}' has no captured invocation")
            raise CaptureError(f"Hook target '{target}' is ambiguous across invocations")
        raise CaptureError(f"Hook target '{target}' is not a static Agent source target")


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
        raise CaptureError("an Agent is one async def")
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
