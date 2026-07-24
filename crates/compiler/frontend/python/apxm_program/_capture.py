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
from typing import Any, Optional

from ._bound_tree import (
    BoundCall,
    BoundContextEdge,
    BoundControl,
    BoundDeclaration,
    BoundOperand,
    BoundParameter,
    BoundProgram,
    BoundRegion,
    BoundValue,
    Span,
)
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
        self.capability_requirements: dict[str, bool] = {}
        self.model_requirements: list[str] = []
        self.spans: list[tuple[str, Span, str]] = []

        self._facade_name: Optional[str] = None
        self._input_name: Optional[str] = None
        self._counter = 0
        self._order: dict[str, int] = {}
        self._declared: dict[str, str] = {}

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
        for name, binding in self.bindings.items():
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

    def capture(self, func_ast: ast.AsyncFunctionDef) -> BoundProgram:
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
                value_id=f"{self.program_id}.param.input",
                type_ref=self.input_type_ref,
                origin="parameter",
                origin_id=self.entrypoint,
            )
        )

        self.regions.append(
            BoundRegion(
                region_id=self.body_region_id,
                region_role="function_body",
                parent_region_id=None,
                execution_order=0,
            )
        )
        self._visit_block(func_ast.body, self.body_region_id)

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
        elif isinstance(stmt, ast.Assign):
            self._visit_assign(stmt, region_id)
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
                    origin_id=self.body_region_id,
                )
            )
            return
        # name = await <call>  binds the call result to a named value.
        if isinstance(stmt.value, ast.Await) and isinstance(target, ast.Name):
            self._visit_await(stmt.value, region_id, assign_to=target.id)
            return
        if isinstance(stmt.value, ast.Await):
            self._visit_await(stmt.value, region_id, assign_to=None)
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
        node_id = self._next(intent)
        result_value: Optional[str] = None
        if assign_to is not None or intent != "event_wait":
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
        span = self._span(call)
        if span is not None:
            self.spans.append((node_id, span, intent))

    def _resolve_call_target(
        self, call: ast.Call
    ) -> tuple[str, Optional[str], Optional[str], str]:
        func = call.func
        # instance.invoke(...) or Definition.invoke(...) -> agent_invocation
        if isinstance(func, ast.Attribute):
            if func.attr == "invoke":
                receiver_kind = "program_instance_ref"
                if isinstance(func.value, ast.Name) and func.value.id in self.bindings:
                    receiver_kind = "program_ref"
                return "agent_invocation", self._binding_of(func.value), receiver_kind, "input"
            if func.attr == "new":
                return "agent_creation", self._binding_of(func.value), None, "initial_context"
            if func.attr == "wait":
                return "event_wait", self._binding_of(func.value), None, "event_ref"
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
            return self._declared.get(node.id, node.id)
        return None

    def _result_type(self, binding_ref: Optional[str]) -> str:
        for decl in self.declarations:
            if decl.decl_id == binding_ref:
                return decl.output_type_ref
        return self.output_type_ref

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
        value_id = self._next("value")
        self.values.append(
            BoundValue(
                value_id=value_id,
                type_ref="ArgumentValue",
                origin="literal",
                origin_id=node_id,
            )
        )
        operands.append(BoundOperand(value_id=value_id, slot=slot))
        return operands

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
        self.regions.append(
            BoundRegion(
                region_id=body_region,
                region_role="loop_body",
                parent_region_id=region_id,
                execution_order=self._order_in(region_id),
            )
        )
        self._visit_block(stmt.body, body_region)

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
    try:
        source_file = inspect.getsourcefile(func) or "<agent>"
    except TypeError:
        source_file = "<agent>"

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
