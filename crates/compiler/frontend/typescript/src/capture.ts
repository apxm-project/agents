// Static capture of an authored Agent callback into the FrontendGraph.
//
// Parses the callback with the TypeScript compiler API, binds the declaration
// markers by the names visible at the definition site, and folds the supported
// control-flow subset into the language-neutral graph. It never executes the
// callback body: behavior is read from syntax and resolved declarations. Stable
// node and region identities come from the program identity plus lexical
// preorder, matching the Python frontend so paired goldens converge.

import ts from "typescript";

import {
  FRONTEND_GRAPH_VERSION,
  SOURCE_MAP_VERSION,
  type Json,
} from "./bridge.js";
import type {
  CapabilityBinding,
  ContextSchema,
  EventTypeBinding,
  ModelBinding,
  ToolBinding,
} from "./markers.js";

export type Binding =
  | ModelBinding
  | ToolBinding
  | CapabilityBinding
  | EventTypeBinding
  | ContextSchema;

type CallRecord = {
  node_id: string;
  intent_kind: string;
  parent_region_id: string;
  execution_order: number;
  binding_ref?: string;
  receiver_kind?: string;
  operand_values?: string[];
  result_value?: string;
};

type ControlRecord = {
  node_id: string;
  control_kind: string;
  parent_region_id: string;
  execution_order: number;
  body_region_ids?: string[];
  operand_values?: string[];
  result_value?: string;
};

export type CaptureInput = {
  programId: string;
  entrypoint: string;
  inputTypeRef: string;
  outputTypeRef: string;
  contextTypeRef?: string;
  hasDefaultContext: boolean;
  bindings: Map<string, Binding>;
  bindingDeclIds: Map<string, string>;
  callbackSource: string;
};

export class CaptureError extends Error {}

class Capture {
  private counter = 0;
  private readonly order = new Map<string, number>();
  private facadeName = "agent";
  private inputName = "input";

  private readonly declarations: Json[] = [];
  private readonly values: Json[] = [];
  private readonly regions: Json[] = [];
  private readonly calls: CallRecord[] = [];
  private readonly controls: ControlRecord[] = [];
  private readonly dataEdges: Json[] = [];
  private readonly capabilityRequirements = new Map<string, boolean>();
  private readonly modelRequirements: string[] = [];
  private readonly nodeSpans: Json[] = [];
  private readonly bodyRegionId: string;

  constructor(private readonly input: CaptureInput) {
    this.bodyRegionId = `${input.programId}.body`;
  }

  private next(prefix: string): string {
    this.counter += 1;
    return `${this.input.programId}.${prefix}.${this.counter}`;
  }

  private orderIn(regionId: string): number {
    const current = this.order.get(regionId) ?? 0;
    this.order.set(regionId, current + 1);
    return current;
  }

  private declareBindings(): void {
    for (const [name, binding] of this.input.bindings) {
      const declId = this.input.bindingDeclIds.get(name);
      if (declId === undefined) {
        continue;
      }
      if (binding.kind === "model_binding") {
        this.declarations.push({
          decl_id: declId,
          decl_kind: "model_binding",
          input_type_ref: binding.inputTypeRef,
          output_type_ref: binding.outputTypeRef,
          target_ref: binding.targetRef,
        });
        this.modelRequirements.push(binding.targetRef);
      } else if (binding.kind === "tool_binding" || binding.kind === "tool_handler") {
        const record: Json = {
          decl_id: declId,
          decl_kind: binding.kind,
          input_type_ref: binding.inputTypeRef,
          output_type_ref: binding.outputTypeRef,
          target_ref: binding.targetRef,
        };
        if (binding.handlerDigest !== undefined) {
          record.handler_digest = binding.handlerDigest;
        }
        this.declarations.push(record);
        this.capabilityRequirements.set(binding.targetRef, true);
      } else if (
        binding.kind === "capability_binding" ||
        binding.kind === "capability_handler"
      ) {
        const record: Json = {
          decl_id: declId,
          decl_kind: binding.kind,
          input_type_ref: binding.inputTypeRef,
          output_type_ref: binding.outputTypeRef,
          target_ref: binding.targetRef,
        };
        if (binding.handlerDigest !== undefined) {
          record.handler_digest = binding.handlerDigest;
        }
        this.declarations.push(record);
        this.capabilityRequirements.set(binding.targetRef, false);
      } else if (binding.kind === "event_type") {
        this.declarations.push({
          decl_id: declId,
          decl_kind: "event_type",
          input_type_ref: binding.typeRef,
          output_type_ref: binding.typeRef,
        });
      } else if (binding.kind === "context") {
        this.declarations.push({
          decl_id: declId,
          decl_kind: "context",
          input_type_ref: binding.typeRef,
          output_type_ref: binding.typeRef,
          context_default_present: binding.defaultPresent,
        });
      }
    }
  }

  capture(): Json {
    this.declareBindings();

    // A method-shorthand callback (`async run(agent, input) { ... }`) is not a
    // standalone parse; wrap it as an object literal member so the compiler API
    // yields a function-like node.
    const raw = this.input.callbackSource.trimStart();
    const wrapped =
      raw.startsWith("function") || raw.startsWith("async function") || raw.includes("=>")
        ? `const __agent__ = (${raw});`
        : `const __agent__ = { ${raw} };`;
    const source = ts.createSourceFile(
      "agent.ts",
      wrapped,
      ts.ScriptTarget.Latest,
      true,
    );
    const fn = this.findCallback(source);
    if (fn === undefined) {
      throw new CaptureError("an Agent run callback is one function");
    }
    const params = fn.parameters;
    if (params.length >= 1 && ts.isIdentifier(params[0].name)) {
      this.facadeName = params[0].name.text;
    }
    if (params.length >= 2 && ts.isIdentifier(params[1].name)) {
      this.inputName = params[1].name.text;
    }

    this.values.push({
      value_id: `${this.input.programId}.param.input`,
      type_ref: this.input.inputTypeRef,
      origin: "parameter",
      origin_id: this.input.entrypoint,
    });
    this.regions.push({
      region_id: this.bodyRegionId,
      region_role: "function_body",
      execution_order: 0,
    });

    if (fn.body !== undefined && ts.isBlock(fn.body)) {
      this.visitBlock(fn.body.statements, this.bodyRegionId, source);
    }

    return this.build();
  }

  private findCallback(
    source: ts.SourceFile,
  ): ts.FunctionLikeDeclarationBase | undefined {
    let found: ts.FunctionLikeDeclarationBase | undefined;
    const visit = (node: ts.Node): void => {
      if (found !== undefined) {
        return;
      }
      if (
        ts.isFunctionDeclaration(node) ||
        ts.isFunctionExpression(node) ||
        ts.isArrowFunction(node) ||
        ts.isMethodDeclaration(node)
      ) {
        found = node;
        return;
      }
      ts.forEachChild(node, visit);
    };
    ts.forEachChild(source, visit);
    return found;
  }

  private visitBlock(
    statements: ts.NodeArray<ts.Statement>,
    regionId: string,
    source: ts.SourceFile,
  ): void {
    for (const stmt of statements) {
      this.visitStatement(stmt, regionId, source);
    }
  }

  private visitStatement(
    stmt: ts.Statement,
    regionId: string,
    source: ts.SourceFile,
  ): void {
    if (ts.isExpressionStatement(stmt)) {
      this.visitExpression(stmt.expression, regionId, source);
    } else if (ts.isVariableStatement(stmt)) {
      for (const decl of stmt.declarationList.declarations) {
        if (decl.initializer !== undefined) {
          this.visitExpression(decl.initializer, regionId, source);
        }
      }
    } else if (ts.isWhileStatement(stmt) || ts.isForStatement(stmt)) {
      this.visitLoop(stmt, regionId, source);
    } else if (ts.isIfStatement(stmt)) {
      this.visitConditional(stmt, regionId, source);
    } else if (ts.isReturnStatement(stmt)) {
      this.visitReturn(stmt, regionId, source);
    } else if (ts.isTryStatement(stmt)) {
      this.visitTry(stmt, regionId, source);
    } else if (ts.isBlock(stmt)) {
      this.visitBlock(stmt.statements, regionId, source);
    }
  }

  private visitExpression(
    expr: ts.Expression,
    regionId: string,
    source: ts.SourceFile,
  ): void {
    if (ts.isAwaitExpression(expr)) {
      this.visitAwait(expr, regionId, source);
    } else if (
      ts.isBinaryExpression(expr) &&
      expr.operatorToken.kind === ts.SyntaxKind.EqualsToken
    ) {
      // agent.context = ... produces an explicit typed Context transition.
      if (
        ts.isPropertyAccessExpression(expr.left) &&
        ts.isIdentifier(expr.left.expression) &&
        expr.left.expression.text === this.facadeName &&
        expr.left.name.text === "context"
      ) {
        this.values.push({
          value_id: this.next("context"),
          type_ref: this.input.contextTypeRef ?? "Context",
          origin: "context_value",
          origin_id: this.bodyRegionId,
        });
      } else if (ts.isAwaitExpression(expr.right)) {
        this.visitAwait(expr.right, regionId, source);
      }
    }
  }

  private visitAwait(
    expr: ts.AwaitExpression,
    regionId: string,
    source: ts.SourceFile,
  ): void {
    const call = expr.expression;
    if (!ts.isCallExpression(call)) {
      throw new CaptureError("await targets a typed effect call");
    }
    const resolved = this.resolveCallTarget(call);
    const nodeId = this.next(resolved.intent);
    let resultValue: string | undefined;
    if (resolved.intent !== "event_wait") {
      resultValue = this.next("value");
      this.values.push({
        value_id: resultValue,
        type_ref: this.resultType(resolved.bindingRef),
        origin: "call_result",
        origin_id: nodeId,
      });
    } else {
      resultValue = this.next("value");
      this.values.push({
        value_id: resultValue,
        type_ref: "EventPayload",
        origin: "resume_input",
        origin_id: nodeId,
      });
    }

    const operandValues = this.callOperands(call, nodeId, resolved.slot);
    const record: CallRecord = {
      node_id: nodeId,
      intent_kind: resolved.intent,
      parent_region_id: regionId,
      execution_order: this.orderIn(regionId),
    };
    if (resolved.bindingRef !== undefined) {
      record.binding_ref = resolved.bindingRef;
    }
    if (resolved.receiverKind !== undefined) {
      record.receiver_kind = resolved.receiverKind;
    }
    if (operandValues.length > 0) {
      record.operand_values = operandValues;
    }
    if (resultValue !== undefined) {
      record.result_value = resultValue;
    }
    this.calls.push(record);

    const { line } = source.getLineAndCharacterOfPosition(call.getStart(source));
    this.nodeSpans.push({
      node_id: nodeId,
      source_file: "<agent>",
      span: {
        start_line: line + 1,
        start_column: 0,
        end_line: line + 1,
        end_column: 1,
      },
      semantic_annotation: resolved.intent,
    });
  }

  private resolveCallTarget(call: ts.CallExpression): {
    intent: string;
    bindingRef?: string;
    receiverKind?: string;
    slot: string;
  } {
    const callee = call.expression;
    if (ts.isPropertyAccessExpression(callee)) {
      const method = callee.name.text;
      const receiverName = ts.isIdentifier(callee.expression)
        ? callee.expression.text
        : undefined;
      if (method === "invoke") {
        const receiverKind =
          receiverName !== undefined && this.input.bindings.has(receiverName)
            ? "program_ref"
            : "program_instance_ref";
        return {
          intent: "agent_invocation",
          bindingRef: this.bindingRefOf(receiverName),
          receiverKind,
          slot: "input",
        };
      }
      if (method === "new") {
        return {
          intent: "agent_creation",
          bindingRef: this.bindingRefOf(receiverName),
          slot: "initial_context",
        };
      }
      if (method === "wait") {
        return {
          intent: "event_wait",
          bindingRef: this.bindingRefOf(receiverName),
          slot: "event_ref",
        };
      }
      throw new CaptureError(`unsupported call '.${method}(...)'`);
    }
    if (ts.isIdentifier(callee)) {
      const binding = this.input.bindings.get(callee.text);
      const declId = this.input.bindingDeclIds.get(callee.text);
      if (binding?.kind === "model_binding") {
        return { intent: "model_invocation", bindingRef: declId, slot: "request" };
      }
      if (binding?.kind === "tool_binding" || binding?.kind === "tool_handler") {
        return { intent: "tool_invocation", bindingRef: declId, slot: "arguments" };
      }
      if (
        binding?.kind === "capability_binding" ||
        binding?.kind === "capability_handler"
      ) {
        return {
          intent: "capability_invocation",
          bindingRef: declId,
          slot: "arguments",
        };
      }
      throw new CaptureError(
        `call target '${callee.text}' is not a bound Model, Tool, or Capability`,
      );
    }
    throw new CaptureError("computed or dynamic call targets are not supported");
  }

  private bindingRefOf(name: string | undefined): string | undefined {
    if (name === undefined) {
      return undefined;
    }
    return this.input.bindingDeclIds.get(name) ?? name;
  }

  private resultType(bindingRef: string | undefined): string {
    for (const decl of this.declarations) {
      if (decl.decl_id === bindingRef) {
        return decl.output_type_ref as string;
      }
    }
    return this.input.outputTypeRef;
  }

  private callOperands(
    call: ts.CallExpression,
    nodeId: string,
    slot: string,
  ): string[] {
    if (call.arguments.length === 0) {
      return [];
    }
    const valueId = this.next("value");
    this.values.push({
      value_id: valueId,
      type_ref: "ArgumentValue",
      origin: "literal",
      origin_id: nodeId,
    });
    this.dataEdges.push({
      from_value: valueId,
      to_consumer: nodeId,
      consumer_slot: slot,
    });
    return [valueId];
  }

  private visitLoop(
    stmt: ts.WhileStatement | ts.ForStatement,
    regionId: string,
    source: ts.SourceFile,
  ): void {
    const nodeId = this.next("loop");
    const bodyRegion = `${nodeId}.body`;
    this.controls.push({
      node_id: nodeId,
      control_kind: "loop",
      parent_region_id: regionId,
      execution_order: this.orderIn(regionId),
      body_region_ids: [bodyRegion],
    });
    this.regions.push({
      region_id: bodyRegion,
      region_role: "loop_body",
      parent_region_id: regionId,
      execution_order: this.orderIn(regionId),
    });
    if (stmt.statement !== undefined) {
      this.visitBodyStatement(stmt.statement, bodyRegion, source);
    }
  }

  private visitConditional(
    stmt: ts.IfStatement,
    regionId: string,
    source: ts.SourceFile,
  ): void {
    const nodeId = this.next("cond");
    const thenRegion = `${nodeId}.then`;
    const bodyRegions = [thenRegion];
    if (stmt.elseStatement !== undefined) {
      bodyRegions.push(`${nodeId}.else`);
    }
    this.controls.push({
      node_id: nodeId,
      control_kind: "conditional",
      parent_region_id: regionId,
      execution_order: this.orderIn(regionId),
      body_region_ids: bodyRegions,
    });
    this.regions.push({
      region_id: thenRegion,
      region_role: "conditional_arm",
      parent_region_id: regionId,
      execution_order: this.orderIn(regionId),
    });
    this.visitBodyStatement(stmt.thenStatement, thenRegion, source);
    if (stmt.elseStatement !== undefined) {
      const elseRegion = `${nodeId}.else`;
      this.regions.push({
        region_id: elseRegion,
        region_role: "conditional_arm",
        parent_region_id: regionId,
        execution_order: this.orderIn(regionId),
      });
      this.visitBodyStatement(stmt.elseStatement, elseRegion, source);
    }
  }

  private visitTry(
    stmt: ts.TryStatement,
    regionId: string,
    source: ts.SourceFile,
  ): void {
    const nodeId = this.next("try");
    const tryRegion = `${nodeId}.try`;
    const bodyRegions = [tryRegion];
    if (stmt.catchClause !== undefined) {
      bodyRegions.push(`${nodeId}.catch.1`);
    }
    this.controls.push({
      node_id: nodeId,
      control_kind: "try_catch",
      parent_region_id: regionId,
      execution_order: this.orderIn(regionId),
      body_region_ids: bodyRegions,
    });
    this.regions.push({
      region_id: tryRegion,
      region_role: "try_body",
      parent_region_id: regionId,
      execution_order: this.orderIn(regionId),
    });
    this.visitBlock(stmt.tryBlock.statements, tryRegion, source);
    if (stmt.catchClause !== undefined) {
      const catchRegion = `${nodeId}.catch.1`;
      this.regions.push({
        region_id: catchRegion,
        region_role: "catch_body",
        parent_region_id: regionId,
        execution_order: this.orderIn(regionId),
      });
      this.visitBlock(stmt.catchClause.block.statements, catchRegion, source);
    }
  }

  private visitReturn(
    stmt: ts.ReturnStatement,
    regionId: string,
    source: ts.SourceFile,
  ): void {
    if (stmt.expression !== undefined && ts.isAwaitExpression(stmt.expression)) {
      this.visitAwait(stmt.expression, regionId, source);
    }
    this.controls.push({
      node_id: this.next("return"),
      control_kind: "return",
      parent_region_id: regionId,
      execution_order: this.orderIn(regionId),
    });
  }

  private visitBodyStatement(
    stmt: ts.Statement,
    regionId: string,
    source: ts.SourceFile,
  ): void {
    if (ts.isBlock(stmt)) {
      this.visitBlock(stmt.statements, regionId, source);
    } else {
      this.visitStatement(stmt, regionId, source);
    }
  }

  private build(): Json {
    const definition: Json = {
      program_id: this.input.programId,
      entrypoint: this.input.entrypoint,
      input_type_ref: this.input.inputTypeRef,
      output_type_ref: this.input.outputTypeRef,
      has_default_context: this.input.hasDefaultContext,
    };
    if (this.input.contextTypeRef !== undefined) {
      definition.context_type_ref = this.input.contextTypeRef;
    }

    const regionAnnotations = this.controls
      .filter(
        (control) =>
          control.control_kind === "loop" &&
          control.body_region_ids !== undefined &&
          control.body_region_ids.length > 0,
      )
      .map((control) => ({
        region_id: (control.body_region_ids as string[])[0],
        annotation: "structural_loop",
      }));

    return {
      schema_version: FRONTEND_GRAPH_VERSION,
      source_language: "typescript",
      program_definitions: [definition],
      imported_program_refs: [],
      declarations: this.declarations,
      functions: [
        {
          function_id: this.input.entrypoint,
          parameters: [
            {
              value_id: `${this.input.programId}.param.agent`,
              type_ref: "AgentFacade",
              role: "agent_facade",
            },
            {
              value_id: `${this.input.programId}.param.input`,
              type_ref: this.input.inputTypeRef,
              role: "input",
            },
          ],
          result_type_ref: this.input.outputTypeRef,
          body_region_id: this.bodyRegionId,
          is_entrypoint: true,
        },
      ],
      values: this.values,
      blocks: [],
      regions: this.regions,
      data_edges: this.dataEdges,
      call_intents: this.calls as unknown as Json[],
      control_intents: this.controls as unknown as Json[],
      context_flow: [],
      hook_bindings: [],
      capability_requirements: Array.from(
        this.capabilityRequirements,
        ([ref, tool]) => ({
          capability_ref: ref,
          tool_schema_present: tool,
        }),
      ),
      model_requirements: this.modelRequirements.map((ref) => ({
        model_target_ref: ref,
      })),
      source_map: {
        schema_version: SOURCE_MAP_VERSION,
        source_language: "typescript",
        node_spans: this.nodeSpans,
        region_annotations: regionAnnotations,
      },
    };
  }
}

export function captureProgram(input: CaptureInput): Json {
  return new Capture(input).capture();
}
