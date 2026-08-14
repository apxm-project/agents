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
} from "./contract.js";
import type {
  CapabilityBinding,
  ContextSchema,
  EventTypeBinding,
  ModelBinding,
  ToolBinding,
} from "./markers.js";
import { stableDigest } from "./markers.js";

/** Static program reference metadata consumed by composition capture. */
export type ProgramBinding = {
  readonly kind: "agent_definition";
  readonly programId: string;
  readonly artifactDigest: string;
  readonly entrypoint: string;
  readonly targetAgentIdentityRequirement: string;
};

export type Binding =
  | ModelBinding<any, any>
  | ToolBinding<any, any>
  | CapabilityBinding<any, any>
  | EventTypeBinding<any>
  | ContextSchema
  | ProgramBinding;

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
  predicate?: {
    root_value_id: string;
    property_path: string[];
    comparator: "truthy" | "equals" | "not_equals";
    literal?: { scalar_type: "boolean" | "string" | "integer" | "null"; value?: boolean | string | number };
  };
  operand_values?: string[];
  result_value?: string;
};

type ValueExpression =
  | { kind: "ssa"; value_id: string }
  | { kind: "context"; property_path: string[] }
  | { kind: "projection"; root: ValueExpression; property_path: string[] }
  | { kind: "object"; fields: Array<{ name: string; value: ValueExpression }> }
  | { kind: "array"; items: ValueExpression[] }
  | { kind: "string"; value: string }
  | { kind: "integer"; value: number }
  | { kind: "boolean"; value: boolean }
  | { kind: "null" };

/**
 * One authored Capability declaration.
 *
 * Declarations are held one per authored binding, never one per
 * `capability_ref`: the same capability declared as both a Tool and a plain
 * Capability is two distinct requirements and both reach the FrontendGraph.
 */
type CapabilityRequirement = {
  capability_ref: string;
  tool_schema_present: boolean;
  requested_permission?: string;
};

/** Source text registered by the host compiler bridge before Agent capture. */
export type StaticSource = {
  readonly fileName: string;
  readonly text: string;
  readonly line?: number;
};

/** Bound Agent declaration inputs consumed by the static AST capture pass. */
export type CaptureInput = {
  programId: string;
  entrypoint: string;
  inputTypeRef: string;
  outputTypeRef: string;
  contextTypeRef?: string;
  hasDefaultContext: boolean;
  bindings: Map<string, Binding>;
  bindingDeclIds: Map<string, string>;
  source: StaticSource;
};

/** Source diagnostic raised when an Agent construct is outside the closed subset. */
export class CaptureError extends Error {}

class Capture {
  private counter = 0;
  private readonly order = new Map<string, number>();
  private inputName = "input";

  private readonly declarations: Json[] = [];
  private readonly values: Json[] = [];
  private readonly blocks: Json[] = [];
  private readonly regions: Json[] = [];
  private readonly calls: CallRecord[] = [];
  private readonly controls: ControlRecord[] = [];
  private readonly dataEdges: Json[] = [];
  private readonly contextFlow: Json[] = [];
  private readonly hooks: Json[] = [];
  private readonly importedProgramRefs = new Map<string, Json>();
  private readonly instances = new Map<ts.Symbol, string>();
  /** Named source values resolve to the value ids they produce. */
  private readonly valuesBySymbol = new Map<ts.Symbol, string>();
  private readonly bindingSymbols = new Map<ts.Symbol, string>();
  private readonly lastNodeByRegion = new Map<string, string>();
  private readonly pendingContextByRegion = new Map<string, { source: string; valueId: string }>();
  private readonly capabilityRequirements: CapabilityRequirement[] = [];
  private readonly modelRequirements: string[] = [];
  private readonly nodeSpans: Json[] = [];
  private readonly bodyRegionId: string;
  private programBindingSymbol: ts.Symbol | undefined;
  private facadeSymbol: ts.Symbol | undefined;
  private checker!: ts.TypeChecker;

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

  /**
   * Declare bindings in sorted-name order, which is also the order the Python
   * frontend declares them in. Every collection this fills — declarations,
   * model requirements, capability requirements — is compared across the two
   * languages, so the order is fixed here rather than left to how an author
   * happened to write the `use` object.
   */
  private declareBindings(): void {
    const declared = [...this.input.bindings.entries()].sort(([left], [right]) =>
      left < right ? -1 : left > right ? 1 : 0,
    );
    for (const [name, binding] of declared) {
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
      } else if (binding.kind === "tool_binding") {
        this.declarations.push({
          decl_id: declId,
          decl_kind: "tool_binding",
          input_type_ref: binding.inputTypeRef,
          output_type_ref: binding.outputTypeRef,
          target_ref: binding.targetRef,
        });
        this.requireCapability(binding, true);
      } else if (binding.kind === "capability_binding") {
        this.declarations.push({
          decl_id: declId,
          decl_kind: "capability_binding",
          input_type_ref: binding.inputTypeRef,
          output_type_ref: binding.outputTypeRef,
          target_ref: binding.targetRef,
        });
        this.requireCapability(binding, false);
      } else if (binding.kind === "event_type") {
        this.declarations.push({
          decl_id: declId,
          decl_kind: "event_type",
          input_type_ref: binding.typeRef,
          output_type_ref: binding.typeRef,
          target_ref: binding.targetRef,
        });
      } else if (binding.kind === "context") {
        this.declarations.push({
          decl_id: declId,
          decl_kind: "context",
          input_type_ref: binding.typeRef,
          output_type_ref: binding.typeRef,
          context_default_present: binding.defaultPresent,
        });
      } else if (binding.kind === "agent_definition") {
        this.importedProgramRefs.set(binding.programId, {
          program_ref: binding.programId,
          artifact_digest: binding.artifactDigest,
          entrypoint: binding.entrypoint,
          target_agent_identity_requirement: binding.targetAgentIdentityRequirement,
        });
      }
    }
  }

  /**
   * Record one authored Capability declaration in declaration order.
   *
   * Requirements are held as a list rather than keyed by `capability_ref` so a
   * reference declared both as a Tool and as a plain Capability keeps both
   * declarations. Only a declaration identical in every field collapses.
   */
  private requireCapability(
    binding: { targetRef: string; requestedPermission?: string },
    toolSchemaPresent: boolean,
  ): void {
    const requirement: CapabilityRequirement = {
      capability_ref: binding.targetRef,
      tool_schema_present: toolSchemaPresent,
    };
    if (binding.requestedPermission !== undefined) {
      requirement.requested_permission = binding.requestedPermission;
    }
    const present = this.capabilityRequirements.some(
      (existing) =>
        existing.capability_ref === requirement.capability_ref &&
        existing.tool_schema_present === requirement.tool_schema_present &&
        existing.requested_permission === requirement.requested_permission,
    );
    if (!present) {
      this.capabilityRequirements.push(requirement);
    }
  }

  capture(): Json {
    this.declareBindings();
    const { checker, source } = createBoundSource(this.input.source);
    this.checker = checker;
    this.bindDeclaredSymbols(source);
    const definition = this.findAgentDefinition(source);
    const fn = definition.callback;
    this.programBindingSymbol = definition.bindingName === undefined
      ? undefined
      : this.symbolForTopLevelName(source, definition.bindingName);
    const params = fn.parameters;
    if (params.length >= 1 && ts.isIdentifier(params[0].name)) {
      this.facadeSymbol = this.symbolAt(params[0].name);
    }
    if (params.length >= 2 && ts.isIdentifier(params[1].name)) {
      this.inputName = params[1].name.text;
    }

    const inputValueId = `${this.input.programId}.param.input`;
    this.values.push({
      value_id: `${this.input.programId}.param.agent`,
      type_ref: "AgentFacade",
      origin: "parameter",
      origin_id: this.input.entrypoint,
    });
    this.values.push({
      value_id: inputValueId,
      type_ref: this.input.inputTypeRef,
      origin: "parameter",
      origin_id: this.input.entrypoint,
    });
    if (params.length >= 2 && ts.isIdentifier(params[1].name)) {
      const inputSymbol = this.symbolAt(params[1].name);
      if (inputSymbol !== undefined) {
        this.valuesBySymbol.set(inputSymbol, inputValueId);
      }
    }
    this.addRegion(this.bodyRegionId, "function_body", undefined, 0);

    if (fn.body !== undefined && ts.isBlock(fn.body)) {
      this.visitBlock(fn.body.statements, this.bodyRegionId, source);
    }
    this.captureStaticHooks(source);

    return this.build();
  }

  private findAgentDefinition(source: ts.SourceFile): {
    callback: ts.FunctionLikeDeclarationBase;
    bindingName?: string;
  } {
    const candidates: Array<{
      call: ts.CallExpression;
      callback: ts.FunctionLikeDeclarationBase;
      bindingName?: string;
      distance: number;
    }> = [];
    const requestedPosition = this.input.source.line === undefined
      ? undefined
      : source.getPositionOfLineAndCharacter(Math.max(this.input.source.line - 1, 0), 0);
    const visit = (node: ts.Node): void => {
      if (
        ts.isCallExpression(node) && this.isFrontendAgentFactory(node.expression)
      ) {
        const callback = callbackFromAgentCall(node);
        if (callback !== undefined) {
          const bindingName = bindingNameOfAgentCall(node);
          const contained = requestedPosition !== undefined &&
            node.getStart(source) <= requestedPosition && requestedPosition <= node.getEnd();
          const namedForProgram = agentNameFromCall(node) === this.input.programId;
          candidates.push({
            call: node,
            callback,
            bindingName,
            distance: namedForProgram
              ? 0
              : contained
                ? node.getWidth(source)
                : requestedPosition === undefined
                  ? Number.MAX_SAFE_INTEGER
                  : Math.abs(node.getStart(source) - requestedPosition),
          });
        }
      }
      ts.forEachChild(node, visit);
    };
    ts.forEachChild(source, visit);
    candidates.sort((left, right) => left.distance - right.distance);
    const selected = candidates[0];
    if (selected === undefined) {
      throw new CaptureError("Agent requires a statically declared run callback");
    }
    return { callback: selected.callback, bindingName: selected.bindingName };
  }

  private bindDeclaredSymbols(source: ts.SourceFile): void {
    for (const statement of source.statements) {
      if (!ts.isVariableStatement(statement)) {
        continue;
      }
      for (const declaration of statement.declarationList.declarations) {
        if (!ts.isIdentifier(declaration.name) || !this.input.bindings.has(declaration.name.text)) {
          continue;
        }
        const symbol = this.symbolAt(declaration.name);
        if (symbol !== undefined) {
          this.bindingSymbols.set(symbol, declaration.name.text);
        }
      }
    }
  }

  private symbolForTopLevelName(source: ts.SourceFile, name: string): ts.Symbol | undefined {
    for (const statement of source.statements) {
      if (!ts.isVariableStatement(statement)) {
        continue;
      }
      for (const declaration of statement.declarationList.declarations) {
        if (ts.isIdentifier(declaration.name) && declaration.name.text === name) {
          return this.symbolAt(declaration.name);
        }
      }
    }
    return undefined;
  }

  private symbolAt(node: ts.Node): ts.Symbol | undefined {
    return this.checker.getSymbolAtLocation(node);
  }

  private valueSymbolAt(node: ts.Identifier): ts.Symbol | undefined {
    if (ts.isShorthandPropertyAssignment(node.parent) && node.parent.name === node) {
      return this.checker.getShorthandAssignmentValueSymbol(node.parent) ?? this.symbolAt(node);
    }
    return this.symbolAt(node);
  }

  private bindingNameFor(node: ts.Identifier): string | undefined {
    const symbol = this.symbolAt(node);
    return symbol === undefined ? undefined : this.bindingSymbols.get(symbol);
  }

  private isFacadeIdentifier(node: ts.Identifier): boolean {
    return this.facadeSymbol !== undefined && this.symbolAt(node) === this.facadeSymbol;
  }

  private isFrontendAgentFactory(expression: ts.Expression): boolean {
    return ts.isIdentifier(expression) && this.isFrontendMarker(expression, "Agent");
  }

  private isFrontendMarker(identifier: ts.Identifier, expectedName: string): boolean {
    const symbol = this.symbolAt(identifier);
    return symbol?.declarations?.some((declaration) => {
      if (ts.isImportSpecifier(declaration)) {
        const importedName = declaration.propertyName?.text ?? declaration.name.text;
        const importDeclaration = findImportDeclaration(declaration);
        return importedName === expectedName && isFrontendImportDeclaration(importDeclaration);
      }
      return this.isFrontendNamespaceBinding(declaration, expectedName);
    }) ?? false;
  }

  private isFrontendNamespaceBinding(declaration: ts.Declaration, expectedName: string): boolean {
    if (!ts.isBindingElement(declaration) || !ts.isIdentifier(declaration.name)) {
      return false;
    }
    const projectedName = declaration.propertyName?.getText() ?? declaration.name.text;
    if (projectedName !== expectedName) {
      return false;
    }
    const variable = findVariableDeclaration(declaration);
    const initializer = variable?.initializer;
    if (initializer === undefined || !ts.isIdentifier(initializer)) {
      return false;
    }
    const namespaceSymbol = this.symbolAt(initializer);
    return namespaceSymbol?.declarations?.some(
      (candidate) => ts.isNamespaceImport(candidate) &&
        isFrontendImportDeclaration(findImportDeclaration(candidate)),
    ) ?? false;
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
          const creation = this.visitExpression(decl.initializer, regionId, source);
          if (creation === undefined || !ts.isIdentifier(decl.name)) {
            continue;
          }
          const nameSymbol = this.symbolAt(decl.name);
          if (nameSymbol === undefined) {
            continue;
          }
          if (creation.programRef !== undefined) {
            this.instances.set(nameSymbol, creation.programRef);
          }
          if (creation.valueId !== undefined) {
            this.valuesBySymbol.set(nameSymbol, creation.valueId);
          }
        }
      }
    } else if (ts.isWhileStatement(stmt) || ts.isForStatement(stmt)) {
      this.visitLoop(stmt, regionId, source);
    } else if (ts.isIfStatement(stmt)) {
      this.visitConditional(stmt, regionId, source);
    } else if (ts.isReturnStatement(stmt)) {
      this.visitReturn(stmt, regionId, source);
    } else if (ts.isThrowStatement(stmt)) {
      this.visitThrow(stmt, regionId);
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
  ): { programRef?: string; valueId?: string } | undefined {
    if (ts.isAwaitExpression(expr)) {
      const valueId = this.visitAwait(expr, regionId, source);
      return valueId === undefined ? undefined : { valueId };
    } else if (
      ts.isBinaryExpression(expr) &&
      expr.operatorToken.kind === ts.SyntaxKind.EqualsToken
    ) {
      // agent.context = ... produces an explicit typed Context transition.
      if (
        ts.isPropertyAccessExpression(expr.left) &&
        ts.isIdentifier(expr.left.expression) &&
        this.isFacadeIdentifier(expr.left.expression) &&
        expr.left.name.text === "context"
      ) {
        const contextNode = this.next("context");
        const expression = this.valueExpressionFor(expr.right);
        this.values.push({
          value_id: contextNode,
          type_ref: this.input.contextTypeRef ?? "Context",
          origin: "context_value",
          expression,
        });
        // Anchor an initial replacement to the lexical region entry so it is
        // executable rather than silently discarded before the first effect.
        const source = this.lastNodeByRegion.get(regionId) ?? regionId;
        this.pendingContextByRegion.set(regionId, { source, valueId: contextNode });
      } else if (ts.isAwaitExpression(expr.right)) {
        const valueId = this.visitAwait(expr.right, regionId, source);
        if (valueId !== undefined && ts.isIdentifier(expr.left)) {
          const nameSymbol = this.symbolAt(expr.left);
          if (nameSymbol !== undefined) {
            this.valuesBySymbol.set(nameSymbol, valueId);
          }
        }
        return valueId === undefined ? undefined : { valueId };
      }
    } else if (ts.isCallExpression(expr) && this.isAgentCreation(expr)) {
      return this.recordAgentCreation(expr, regionId, source);
    }
    return undefined;
  }

  private visitAwait(
    expr: ts.AwaitExpression,
    regionId: string,
    source: ts.SourceFile,
  ): string | undefined {
    const call = expr.expression;
    if (!ts.isCallExpression(call)) {
      throw new CaptureError("await targets a typed effect call");
    }
    if (this.isYield(call)) {
      return this.recordYield(call, regionId, source);
    }
    if (this.isTaskGroup(call)) {
      this.recordTaskGroup(call, regionId, source);
      return undefined;
    }
    const resolved = this.resolveCallTarget(call);
    const nodeId = this.next(resolved.intent);
    // Event waits produce a continuation value; ordinary effects produce a
    // result value.
    const resultValue = resolved.intent === "event_wait"
      ? this.next("resume")
      : this.next("value");
    this.values.push({
      value_id: resultValue,
      type_ref: this.resultType(resolved.bindingRef),
      origin: "call_result",
      origin_id: nodeId,
    });

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
    record.result_value = resultValue;
    this.calls.push(record);

    this.recordNode(regionId, nodeId);
    this.recordSpan(nodeId, resolved.intent, call, source);
    return resultValue;
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
      const receiver = ts.isIdentifier(callee.expression)
        ? callee.expression
        : undefined;
      const bindingName = receiver === undefined
        ? undefined
        : this.bindingNameFor(receiver);
      if (method === "invoke") {
        const binding = bindingName === undefined
          ? undefined
          : this.input.bindings.get(bindingName);
        if (binding?.kind === "agent_definition") {
          return {
            intent: "agent_invocation",
            bindingRef: binding.programId,
            receiverKind: "program_ref",
            slot: "input",
          };
        }
        const instance = receiver === undefined
          ? undefined
          : this.instances.get(this.symbolAt(receiver) as ts.Symbol);
        if (instance === undefined) {
          throw new CaptureError("invoke receiver is a statically bound Agent definition or instance");
        }
        return {
          intent: "agent_invocation",
          bindingRef: instance,
          receiverKind: "program_instance_ref",
          slot: "input",
        };
      }
      if (method === "new") {
        const programRef = this.programRefOfBinding(bindingName);
        if (programRef === undefined) {
          throw new CaptureError("new receiver is a statically bound Agent definition");
        }
        return {
          intent: "agent_creation",
          bindingRef: programRef,
          slot: "initial_context",
        };
      }
      if (method === "wait") {
        const bindingRef = this.bindingRefOfBinding(bindingName);
        if (bindingRef === undefined) {
          throw new CaptureError("wait receiver is a statically bound Event");
        }
        return {
          intent: "event_wait",
          bindingRef,
          slot: "event_ref",
        };
      }
      throw new CaptureError(`unsupported call '.${method}(...)'`);
    }
    if (ts.isIdentifier(callee)) {
      const bindingName = this.bindingNameFor(callee);
      const binding = bindingName === undefined
        ? undefined
        : this.input.bindings.get(bindingName);
      const declId = bindingName === undefined
        ? undefined
        : this.input.bindingDeclIds.get(bindingName);
      if (binding?.kind === "model_binding") {
        return { intent: "model_invocation", bindingRef: declId, slot: "request" };
      }
      if (binding?.kind === "tool_binding") {
        return { intent: "tool_invocation", bindingRef: declId, slot: "arguments" };
      }
      if (binding?.kind === "capability_binding") {
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

  private bindingRefOfBinding(name: string | undefined): string | undefined {
    if (name === undefined) {
      return undefined;
    }
    return this.input.bindingDeclIds.get(name) ?? name;
  }

  private programRefOfBinding(name: string | undefined): string | undefined {
    if (name === undefined) {
      return undefined;
    }
    const binding = this.input.bindings.get(name);
    return binding?.kind === "agent_definition" ? binding.programId : undefined;
  }

  private resultType(bindingRef: string | undefined): string {
    for (const decl of this.declarations) {
      if (decl.decl_id === bindingRef) {
        return decl.output_type_ref as string;
      }
    }
    return this.input.outputTypeRef;
  }

  // A value expression stands for data, not for an effect. The only call a
  // value position admits is constructing a bound Context schema. A Model,
  // Tool, or Capability call is an effect and reaches the graph only through
  // await; any other callee is unresolved. Both are rejected here so an
  // unresolved or effectful call can never be silently folded into an untyped
  // literal operand.
  private rejectUnboundCalls(expression: ts.Expression): void {
    const visit = (node: ts.Node): void => {
      if (ts.isCallExpression(node)) {
        this.rejectValuePositionCall(node);
      }
      ts.forEachChild(node, visit);
    };
    visit(expression);
  }

  private rejectValuePositionCall(call: ts.CallExpression): void {
    const callee = call.expression;
    if (!ts.isIdentifier(callee)) {
      throw new CaptureError(
        "value expressions call only a bound Context schema by name",
      );
    }
    const bindingName = this.bindingNameFor(callee);
    const binding = bindingName === undefined
      ? undefined
      : this.input.bindings.get(bindingName);
    if (binding?.kind === "context") {
      return;
    }
    if (
      binding?.kind === "model_binding" ||
      binding?.kind === "tool_binding" ||
      binding?.kind === "capability_binding"
    ) {
      throw new CaptureError(
        `'${callee.text}' is a typed effect and is called with await, not used as a value`,
      );
    }
    throw new CaptureError(
      `call target '${callee.text}' is not a bound Context schema`,
    );
  }

  private valueForExpression(expression: ts.Expression): string {
    // Reuse a prior named value when the operand is that identifier.
    if (ts.isIdentifier(expression)) {
      const symbol = this.valueSymbolAt(expression);
      if (symbol !== undefined) {
        const existing = this.valuesBySymbol.get(symbol);
        if (existing !== undefined) {
          return existing;
        }
      }
    }
    const assembled = this.valueExpressionFor(expression);
    const valueId = this.next("value");
    this.values.push({
      value_id: valueId,
      type_ref: "ArgumentValue",
      origin: "literal",
      expression: assembled,
    });
    return valueId;
  }

  private valueExpressionFor(expression: ts.Expression): ValueExpression {
    this.rejectUnboundCalls(expression);
    if (ts.isIdentifier(expression)) {
      const symbol = this.valueSymbolAt(expression);
      const valueId = symbol === undefined ? undefined : this.valuesBySymbol.get(symbol);
      if (valueId !== undefined) {
        return { kind: "ssa", value_id: valueId };
      }
    }
    if (ts.isStringLiteral(expression) || ts.isNoSubstitutionTemplateLiteral(expression)) {
      return { kind: "string", value: expression.text };
    }
    if (ts.isNumericLiteral(expression)) {
      const value = Number(expression.text);
      if (!Number.isSafeInteger(value)) {
        throw new CaptureError("integer exceeds the shared safe integer domain");
      }
      return { kind: "integer", value };
    }
    if (
      ts.isPrefixUnaryExpression(expression) &&
      expression.operator === ts.SyntaxKind.MinusToken &&
      ts.isNumericLiteral(expression.operand)
    ) {
      const value = -Number(expression.operand.text);
      if (!Number.isSafeInteger(value)) {
        throw new CaptureError("integer exceeds the shared safe integer domain");
      }
      return { kind: "integer", value };
    }
    if (expression.kind === ts.SyntaxKind.TrueKeyword) {
      return { kind: "boolean", value: true };
    }
    if (expression.kind === ts.SyntaxKind.FalseKeyword) {
      return { kind: "boolean", value: false };
    }
    if (expression.kind === ts.SyntaxKind.NullKeyword) {
      return { kind: "null" };
    }
    if (ts.isObjectLiteralExpression(expression)) {
      const fields = expression.properties.map((property) => {
        if (ts.isPropertyAssignment(property)) {
          const name = this.staticPropertyName(property.name);
          return { name, value: this.valueExpressionFor(property.initializer) };
        }
        if (ts.isShorthandPropertyAssignment(property)) {
          return { name: property.name.text, value: this.valueExpressionFor(property.name) };
        }
        throw new CaptureError("object expressions admit only static fields");
      });
      return { kind: "object", fields };
    }
    if (ts.isArrayLiteralExpression(expression)) {
      if (expression.elements.some(ts.isSpreadElement)) {
        throw new CaptureError("array expressions do not admit spreads");
      }
      return {
        kind: "array",
        items: expression.elements.map((item) => this.valueExpressionFor(item)),
      };
    }
    const projection = this.projectionParts(expression);
    if (projection !== undefined) {
      if (projection.root === "context") {
        return { kind: "context", property_path: projection.path };
      }
      return {
        kind: "projection",
        root: { kind: "ssa", value_id: projection.root },
        property_path: projection.path,
      };
    }
    throw new CaptureError("unsupported pure value expression");
  }

  private staticPropertyName(name: ts.PropertyName): string {
    if (ts.isIdentifier(name) || ts.isStringLiteral(name) || ts.isNumericLiteral(name)) {
      return name.text;
    }
    throw new CaptureError("object expression keys are static strings");
  }

  private projectionParts(
    expression: ts.Expression,
  ): { root: string; path: string[] } | undefined {
    const path: string[] = [];
    let node: ts.Expression = expression;
    while (true) {
      if (
        ts.isPropertyAccessExpression(node) &&
        ts.isIdentifier(node.expression) &&
        this.isFacadeIdentifier(node.expression) &&
        node.name.text === "context"
      ) {
        return { root: "context", path };
      }
      if (ts.isPropertyAccessExpression(node)) {
        path.unshift(node.name.text);
        node = node.expression;
        continue;
      }
      if (ts.isElementAccessExpression(node) && ts.isStringLiteral(node.argumentExpression)) {
        path.unshift(node.argumentExpression.text);
        node = node.expression;
        continue;
      }
      break;
    }
    if (ts.isIdentifier(node) && path.length > 0) {
      const symbol = this.valueSymbolAt(node);
      const valueId = symbol === undefined ? undefined : this.valuesBySymbol.get(symbol);
      if (valueId !== undefined) {
        return { root: valueId, path };
      }
    }
    return undefined;
  }

  private callOperands(
    call: ts.CallExpression,
    nodeId: string,
    slot: string,
  ): string[] {
    if (call.arguments.length === 0) {
      return [];
    }
    if (call.arguments.length !== 1) {
      throw new CaptureError("typed effect calls accept exactly one authored operand");
    }
    for (const argument of call.arguments) {
      if (ts.isSpreadElement(argument)) {
        throw new CaptureError("typed effect operands do not admit spreads");
      }
      // Every non-identifier argument still fails closed on unbound or
      // effectful calls before it is recorded as a literal value.
      if (!ts.isIdentifier(argument)) {
        this.rejectUnboundCalls(argument);
      }
    }
    let operandExpression = call.arguments[0];
    if (slot === "initial_context" && ts.isObjectLiteralExpression(operandExpression)) {
      const contextProperty = operandExpression.properties.find(
        (property): property is ts.PropertyAssignment =>
          ts.isPropertyAssignment(property) &&
          this.staticPropertyName(property.name) === "context",
      );
      if (contextProperty !== undefined) {
        operandExpression = contextProperty.initializer;
      }
    }
    const valueId = this.valueForExpression(operandExpression);
    this.dataEdges.push({
      from_value: valueId,
      to_consumer: nodeId,
      consumer_slot: slot,
    });
    return [valueId];
  }

  private isYield(call: ts.CallExpression): boolean {
    return (
      ts.isPropertyAccessExpression(call.expression) &&
        ts.isIdentifier(call.expression.expression) &&
        this.isFacadeIdentifier(call.expression.expression) &&
        call.expression.name.text === "yield_"
    );
  }

  private isTaskGroup(call: ts.CallExpression): boolean {
    return (
      ts.isPropertyAccessExpression(call.expression) &&
        ts.isIdentifier(call.expression.expression) &&
        this.isFrontendMarker(call.expression.expression, "TaskGroup") &&
        call.expression.name.text === "run"
    );
  }

  private isAgentCreation(call: ts.CallExpression): boolean {
    return (
      ts.isPropertyAccessExpression(call.expression) &&
        ts.isIdentifier(call.expression.expression) &&
      call.expression.name.text === "new" &&
      this.input.bindings.get(
        this.bindingNameFor(call.expression.expression) ?? "",
      )?.kind === "agent_definition"
    );
  }

  private recordAgentCreation(
    call: ts.CallExpression,
    regionId: string,
    source: ts.SourceFile,
  ): { programRef: string; valueId: string } {
    const resolved = this.resolveCallTarget(call);
    const nodeId = this.next("agent_creation");
    // Program-instance results use a distinct identity prefix.
    const resultValue = this.next("instance");
    this.values.push({
      value_id: resultValue,
      type_ref: "ProgramInstanceRef",
      origin: "call_result",
      origin_id: nodeId,
    });
    const operandValues = this.callOperands(call, nodeId, resolved.slot);
    this.calls.push({
      node_id: nodeId,
      intent_kind: resolved.intent,
      parent_region_id: regionId,
      execution_order: this.orderIn(regionId),
      binding_ref: resolved.bindingRef,
      operand_values: operandValues,
      result_value: resultValue,
    });
    this.recordNode(regionId, nodeId);
    this.recordSpan(nodeId, "agent_creation", call, source);
    return {
      programRef: resolved.bindingRef ?? "unknown_program",
      valueId: resultValue,
    };
  }

  private recordYield(
    call: ts.CallExpression,
    regionId: string,
    source: ts.SourceFile,
  ): string {
    const nodeId = this.next("yield");
    const resultValue = this.next("resume");
    this.values.push({
      value_id: resultValue,
      type_ref: this.input.inputTypeRef,
      origin: "resume_input",
      origin_id: nodeId,
    });
    this.addBlockArgument(regionId, resultValue);
    const operandValues = this.callOperands(call, nodeId, "output");
    this.controls.push({
      node_id: nodeId,
      control_kind: "yield",
      parent_region_id: regionId,
      execution_order: this.orderIn(regionId),
      operand_values: operandValues,
      result_value: resultValue,
    });
    this.recordNode(regionId, nodeId);
    this.recordSpan(nodeId, "yield", call, source);
    return resultValue;
  }

  private recordTaskGroup(
    call: ts.CallExpression,
    regionId: string,
    source: ts.SourceFile,
  ): void {
    const scope = call.arguments[0];
    if (
      scope === undefined ||
      (!ts.isArrowFunction(scope) && !ts.isFunctionExpression(scope))
    ) {
      throw new CaptureError("TaskGroup.run requires one static callback");
    }
    const nodeId = this.next("task_group");
    // Keep the region identity language-independent: the Python frontend
    // represents the lexical TaskGroup body as `<node>.scope`.
    const childRegion = `${nodeId}.scope`;
    this.controls.push({
      node_id: nodeId,
      control_kind: "task_group",
      parent_region_id: regionId,
      execution_order: this.orderIn(regionId),
      body_region_ids: [childRegion],
    });
    this.recordNode(regionId, nodeId);
    this.addRegion(childRegion, "task_scope", regionId, this.orderIn(regionId));
    if (ts.isBlock(scope.body)) {
      this.visitBlock(scope.body.statements, childRegion, source);
    } else {
      this.visitExpression(scope.body, childRegion, source);
    }
    this.recordSpan(nodeId, "task_group", call, source);
  }

  private recordSpan(
    nodeId: string,
    annotation: string,
    node: ts.Node,
    source: ts.SourceFile,
  ): void {
    const start = source.getLineAndCharacterOfPosition(node.getStart(source));
    const end = source.getLineAndCharacterOfPosition(node.getEnd());
    this.nodeSpans.push({
      node_id: nodeId,
      source_file: portableSourceFile(this.input.source.fileName),
      span: {
        start_line: start.line + 1,
        start_column: start.character,
        end_line: end.line + 1,
        end_column: end.character,
      },
      semantic_annotation: annotation,
    });
  }

  private recordNode(regionId: string, nodeId: string): void {
    const pending = this.pendingContextByRegion.get(regionId);
    if (pending !== undefined && pending.source !== nodeId) {
      this.contextFlow.push({
        from_node: pending.source,
        to_node: nodeId,
        context_type_ref: this.input.contextTypeRef ?? "Context",
        value_id: pending.valueId,
      });
      this.pendingContextByRegion.delete(regionId);
    }
    this.lastNodeByRegion.set(regionId, nodeId);
  }

  private addRegion(
    regionId: string,
    regionRole: string,
    parentRegionId: string | undefined,
    executionOrder: number,
  ): void {
    this.regions.push({
      region_id: regionId,
      region_role: regionRole,
      parent_region_id: parentRegionId,
      execution_order: executionOrder,
    });

    const blockId = `${regionId}.block.0`;
    this.blocks.push({
      block_id: blockId,
      region_id: regionId,
      block_arguments: [],
      execution_order: 0,
    });
  }

  private addBlockArgument(regionId: string, valueId: string): void {
    const block = this.blocks.find(
      (candidate) => candidate.block_id === `${regionId}.block.0`,
    );
    if (block === undefined) {
      throw new CaptureError(`region '${regionId}' has no lexical entry block`);
    }
    const arguments_ = block.block_arguments;
    if (!Array.isArray(arguments_)) {
      throw new CaptureError(`block '${block.block_id}' has invalid block arguments`);
    }
    arguments_.push(valueId);
  }

  private visitLoop(
    stmt: ts.WhileStatement | ts.ForStatement,
    regionId: string,
    source: ts.SourceFile,
  ): void {
    const nodeId = this.next("loop");
    const bodyRegion = `${nodeId}.body`;
    let predicate = ts.isWhileStatement(stmt)
      ? this.predicateForExpression(stmt.expression)
      : undefined;
    const sourceSymbol = predicate === undefined
      ? undefined
      : [...this.valuesBySymbol.entries()].find(([, valueId]) =>
          valueId === predicate?.root_value_id
        )?.[0];
    const controlIndex = this.controls.length;
    this.controls.push({
      node_id: nodeId,
      control_kind: "loop",
      parent_region_id: regionId,
      execution_order: this.orderIn(regionId),
      body_region_ids: [bodyRegion],
      ...(predicate === undefined ? {} : { predicate }),
    });
    this.recordNode(regionId, nodeId);
    this.addRegion(bodyRegion, "loop_body", regionId, this.orderIn(regionId));
    const operandValues: string[] = [];
    let resultValue: string | undefined;
    let initialValue: string | undefined;
    if (predicate !== undefined && sourceSymbol !== undefined) {
      initialValue = predicate.root_value_id;
      resultValue = this.next("value");
      const initialRecord = this.values.find((value) =>
        typeof value === "object" && value !== null && value.value_id === initialValue
      ) as { type_ref?: string } | undefined;
      this.values.push({
        value_id: resultValue,
        type_ref: initialRecord?.type_ref ?? "ArgumentValue",
        origin: "block_argument",
        origin_id: `${bodyRegion}.block.0`,
      });
      this.addBlockArgument(bodyRegion, resultValue);
      this.valuesBySymbol.set(sourceSymbol, resultValue);
      predicate = { ...predicate, root_value_id: resultValue };
    }
    if (stmt.statement !== undefined) {
      this.visitBodyStatement(stmt.statement, bodyRegion, source);
    }
    if (resultValue !== undefined && initialValue !== undefined && sourceSymbol !== undefined) {
      const carriedValue = this.valuesBySymbol.get(sourceSymbol);
      if (carriedValue !== undefined) {
        operandValues.push(initialValue, carriedValue);
        this.dataEdges.push(
          { from_value: initialValue, to_consumer: nodeId, consumer_slot: "initial" },
          { from_value: carriedValue, to_consumer: nodeId, consumer_slot: "carried" },
        );
      }
      this.valuesBySymbol.set(sourceSymbol, resultValue);
    }
    this.controls[controlIndex] = {
      node_id: nodeId,
      control_kind: "loop",
      parent_region_id: regionId,
      execution_order: this.orderIn(regionId),
      body_region_ids: [bodyRegion],
      ...(predicate === undefined ? {} : { predicate }),
      ...(operandValues.length === 0 ? {} : { operand_values: operandValues }),
      ...(resultValue === undefined ? {} : { result_value: resultValue }),
    };
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
    const predicate = this.predicateForExpression(stmt.expression);
    this.controls.push({
      node_id: nodeId,
      control_kind: "conditional",
      parent_region_id: regionId,
      execution_order: this.orderIn(regionId),
      body_region_ids: bodyRegions,
      predicate,
    });
    this.recordNode(regionId, nodeId);
    this.addRegion(thenRegion, "conditional_arm", regionId, this.orderIn(regionId));
    this.visitBodyStatement(stmt.thenStatement, thenRegion, source);
    if (stmt.elseStatement !== undefined) {
      const elseRegion = `${nodeId}.else`;
      this.addRegion(elseRegion, "conditional_arm", regionId, this.orderIn(regionId));
      this.visitBodyStatement(stmt.elseStatement, elseRegion, source);
    }
  }

  private predicateProjection(expression: ts.Expression): {
    root_value_id: string;
    property_path: string[];
  } {
    if (ts.isIdentifier(expression)) {
      const symbol = this.symbolAt(expression);
      const root = symbol === undefined ? undefined : this.valuesBySymbol.get(symbol);
      if (root !== undefined) {
        return { root_value_id: root, property_path: [] };
      }
    }
    if (ts.isPropertyAccessExpression(expression)) {
      const projection = this.predicateProjection(expression.expression);
      projection.property_path.push(expression.name.text);
      return projection;
    }
    if (
      ts.isElementAccessExpression(expression) &&
      expression.argumentExpression !== undefined &&
      ts.isStringLiteral(expression.argumentExpression)
    ) {
      const projection = this.predicateProjection(expression.expression);
      projection.property_path.push(expression.argumentExpression.text);
      return projection;
    }
    throw new CaptureError(
      "control predicate reads a prior typed value through static properties",
    );
  }

  private predicateLiteral(expression: ts.Expression): {
    scalar_type: "boolean" | "string" | "integer" | "null";
    value?: boolean | string | number;
  } {
    if (expression.kind === ts.SyntaxKind.TrueKeyword) {
      return { scalar_type: "boolean", value: true };
    }
    if (expression.kind === ts.SyntaxKind.FalseKeyword) {
      return { scalar_type: "boolean", value: false };
    }
    if (expression.kind === ts.SyntaxKind.NullKeyword) {
      return { scalar_type: "null" };
    }
    if (ts.isStringLiteral(expression)) {
      return { scalar_type: "string", value: expression.text };
    }
    let integer: number | undefined;
    if (ts.isNumericLiteral(expression)) {
      integer = Number(expression.text);
    } else if (
      ts.isPrefixUnaryExpression(expression) &&
      expression.operator === ts.SyntaxKind.MinusToken &&
      ts.isNumericLiteral(expression.operand)
    ) {
      integer = -Number(expression.operand.text);
    }
    if (integer !== undefined) {
      if (!Number.isSafeInteger(integer)) {
        throw new CaptureError(
          "predicate integer literal is within the shared safe-integer domain",
        );
      }
      return { scalar_type: "integer", value: integer };
    }
    throw new CaptureError("predicate equality compares with a scalar literal");
  }

  private predicateForExpression(expression: ts.Expression): NonNullable<ControlRecord["predicate"]> | undefined {
    if (expression.kind === ts.SyntaxKind.TrueKeyword) {
      return undefined;
    }
    if (
      ts.isBinaryExpression(expression) &&
      (expression.operatorToken.kind === ts.SyntaxKind.EqualsEqualsEqualsToken ||
        expression.operatorToken.kind === ts.SyntaxKind.EqualsEqualsToken ||
        expression.operatorToken.kind === ts.SyntaxKind.ExclamationEqualsEqualsToken ||
        expression.operatorToken.kind === ts.SyntaxKind.ExclamationEqualsToken)
    ) {
      return {
        ...this.predicateProjection(expression.left),
        comparator:
          expression.operatorToken.kind === ts.SyntaxKind.ExclamationEqualsEqualsToken ||
          expression.operatorToken.kind === ts.SyntaxKind.ExclamationEqualsToken
            ? "not_equals"
            : "equals",
        literal: this.predicateLiteral(expression.right),
      };
    }
    return {
      ...this.predicateProjection(expression),
      comparator: "truthy",
    };
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
    this.addRegion(tryRegion, "try_body", regionId, this.orderIn(regionId));
    this.visitBlock(stmt.tryBlock.statements, tryRegion, source);
    if (stmt.catchClause !== undefined) {
      const catchRegion = `${nodeId}.catch.1`;
      this.addRegion(catchRegion, "catch_body", regionId, this.orderIn(regionId));
      this.visitBlock(stmt.catchClause.block.statements, catchRegion, source);
    }
  }

  private visitReturn(
    stmt: ts.ReturnStatement,
    regionId: string,
    source: ts.SourceFile,
  ): void {
    if (stmt.expression !== undefined) {
      if (ts.isAwaitExpression(stmt.expression)) {
        this.visitAwait(stmt.expression, regionId, source);
      } else {
        this.rejectUnboundCalls(stmt.expression);
      }
    }
    const nodeId = this.next("return");
    this.controls.push({
      node_id: nodeId,
      control_kind: "return",
      parent_region_id: regionId,
      execution_order: this.orderIn(regionId),
    });
    this.recordNode(regionId, nodeId);
  }

  private visitThrow(stmt: ts.ThrowStatement, regionId: string): void {
    this.rejectUnboundCalls(stmt.expression);
    const nodeId = this.next("throw");
    this.controls.push({
      node_id: nodeId,
      control_kind: "throw",
      parent_region_id: regionId,
      execution_order: this.orderIn(regionId),
    });
    this.recordNode(regionId, nodeId);
  }

  private captureStaticHooks(source: ts.SourceFile): void {
    const hooks: Array<{
      declaration: ts.VariableDeclaration;
      phase: "before" | "after";
      options: ts.ObjectLiteralExpression;
    }> = [];
    const visit = (node: ts.Node): void => {
      if (
        ts.isVariableDeclaration(node) &&
        node.initializer !== undefined &&
        ts.isCallExpression(node.initializer) &&
        ts.isPropertyAccessExpression(node.initializer.expression) &&
        ts.isIdentifier(node.initializer.expression.expression) &&
        this.isFrontendMarker(node.initializer.expression.expression, "Hook") &&
        (node.initializer.expression.name.text === "before" ||
          node.initializer.expression.name.text === "after")
      ) {
        const options = node.initializer.arguments[0];
        if (options !== undefined && ts.isObjectLiteralExpression(options)) {
          hooks.push({
            declaration: node,
            phase: node.initializer.expression.name.text,
            options,
          });
        }
      }
      ts.forEachChild(node, visit);
    };
    ts.forEachChild(source, visit);

    for (const [order, hook] of hooks.entries()) {
      const agent = identifierProperty(hook.options, "agent");
      if (agent === undefined || this.symbolAt(agent) !== this.programBindingSymbol) {
        continue;
      }
      const target = identifierProperty(hook.options, "target");
      if (target === undefined) {
        throw new CaptureError("Hook target is a static bound declaration");
      }
      const hookName = ts.isIdentifier(hook.declaration.name)
        ? hook.declaration.name.text
        : `hook_${order + 1}`;
      const scope = stringProperty(hook.options, "scope") ?? "node";
      if (!["agent", "loop", "node", "model", "capability"].includes(scope)) {
        throw new CaptureError(`Hook scope '${scope}' is not supported`);
      }
      const replace = booleanProperty(hook.options, "replace") ?? false;
      const run = functionProperty(hook.options, "run");
      if (run === undefined) {
        throw new CaptureError("Hook requires a static run callback");
      }
      const targetSelector = this.resolveHookTarget(target, scope);
      this.hooks.push({
        hook_id: `hook.${hookName}`,
        scope,
        phase: hook.phase,
        target_selector: targetSelector,
        declaration_order: order,
        handler_ref: hookName,
        handler_digest: stableDigest(run.getText(source)),
        input_type_ref: "AgentFacade",
        output_type_ref: replace ? this.input.outputTypeRef : "Unit",
        return_mode: replace ? "replace_result" : "observe",
      });
    }
  }

  private resolveHookTarget(target: ts.Identifier, scope: string): string {
    if (scope === "agent") {
      return this.bodyRegionId;
    }
    if (scope === "loop") {
      const loop = this.controls.find((control) => control.control_kind === "loop");
      const region = loop?.body_region_ids?.[0];
      if (region === undefined) {
        throw new CaptureError("a loop Hook requires a static loop in the Agent body");
      }
      return region;
    }
    const bindingRef = this.bindingRefOfBinding(this.bindingNameFor(target));
    const calls = this.calls.filter((candidate) => candidate.binding_ref === bindingRef);
    if (calls.length === 0) {
      throw new CaptureError(`Hook target '${target.text}' has no static invocation`);
    }
    if (calls.length !== 1) {
      throw new CaptureError(`Hook target '${target.text}' is ambiguous across invocations`);
    }
    return calls[0].node_id;
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
      imported_program_refs: Array.from(this.importedProgramRefs.values()),
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
      blocks: this.blocks,
      regions: this.regions,
      data_edges: this.dataEdges,
      call_intents: this.calls as unknown as Json[],
      control_intents: this.controls as unknown as Json[],
      context_flow: this.contextFlow,
      hook_bindings: this.hooks,
      capability_requirements: this.capabilityRequirements,
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

function portableSourceFile(fileName: string): string {
  const normalized = fileName.replaceAll("\\", "/");
  const segments = normalized.split("/");
  if (
    normalized.startsWith("/") ||
    /^[A-Za-z]:\//.test(normalized) ||
    segments.includes("..")
  ) {
    return "<agent>";
  }
  return normalized;
}

/** Capture one statically registered Agent declaration into FrontendGraph. */
export function captureProgram(input: CaptureInput): Json {
  return new Capture(input).capture();
}

function createBoundSource(input: StaticSource): {
  source: ts.SourceFile;
  checker: ts.TypeChecker;
} {
  const options: ts.CompilerOptions = {
    target: ts.ScriptTarget.Latest,
    module: ts.ModuleKind.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    allowJs: true,
    noLib: true,
    noResolve: true,
    skipLibCheck: true,
  };
  const host: ts.CompilerHost = {
    fileExists: (fileName) => fileName === input.fileName,
    readFile: (fileName) => fileName === input.fileName ? input.text : undefined,
    getSourceFile: (fileName, languageVersion) =>
      fileName === input.fileName
        ? ts.createSourceFile(fileName, input.text, languageVersion, true)
        : undefined,
    getDefaultLibFileName: () => "lib.d.ts",
    writeFile: () => {},
    getCurrentDirectory: () => "",
    getDirectories: () => [],
    getCanonicalFileName: (fileName) => fileName,
    useCaseSensitiveFileNames: () => true,
    getNewLine: () => "\n",
  };
  const program = ts.createProgram({
    rootNames: [input.fileName],
    options,
    host,
  });
  const source = program.getSourceFile(input.fileName);
  if (source === undefined) {
    throw new CaptureError(`static source '${input.fileName}' is unavailable`);
  }
  return { source, checker: program.getTypeChecker() };
}

function findImportDeclaration(node: ts.Node): ts.ImportDeclaration | undefined {
  let current: ts.Node | undefined = node;
  while (current !== undefined && !ts.isSourceFile(current)) {
    if (ts.isImportDeclaration(current)) {
      return current;
    }
    current = current.parent;
  }
  return undefined;
}

function findVariableDeclaration(node: ts.Node): ts.VariableDeclaration | undefined {
  let current: ts.Node | undefined = node;
  while (current !== undefined && !ts.isSourceFile(current)) {
    if (ts.isVariableDeclaration(current)) {
      return current;
    }
    current = current.parent;
  }
  return undefined;
}

function isFrontendImportDeclaration(
  declaration: ts.ImportDeclaration | undefined,
): boolean {
  return declaration !== undefined &&
    ts.isStringLiteral(declaration.moduleSpecifier) &&
    isFrontendModuleSpecifier(declaration.moduleSpecifier.text);
}

function isFrontendModuleSpecifier(specifier: string): boolean {
  return specifier === "@apxm/frontend" || specifier === "../src/index.ts";
}

function callbackFromAgentCall(
  call: ts.CallExpression,
): ts.FunctionLikeDeclarationBase | undefined {
  const config = call.arguments[0];
  if (config === undefined || !ts.isObjectLiteralExpression(config)) {
    return undefined;
  }
  const run = functionProperty(config, "run");
  return run;
}

function bindingNameOfAgentCall(call: ts.CallExpression): string | undefined {
  if (!ts.isVariableDeclaration(call.parent) || !ts.isIdentifier(call.parent.name)) {
    return undefined;
  }
  return call.parent.name.text;
}

function agentNameFromCall(call: ts.CallExpression): string | undefined {
  const config = call.arguments[0];
  if (config === undefined || !ts.isObjectLiteralExpression(config)) {
    return undefined;
  }
  return stringProperty(config, "name");
}

function namedProperty(
  object: ts.ObjectLiteralExpression,
  name: string,
): ts.ObjectLiteralElementLike | undefined {
  return object.properties.find(
    (property) =>
      ts.isPropertyAssignment(property) || ts.isMethodDeclaration(property)
        ? property.name?.getText() === name
        : false,
  );
}

function functionProperty(
  object: ts.ObjectLiteralExpression,
  name: string,
): ts.FunctionLikeDeclarationBase | undefined {
  const property = namedProperty(object, name);
  if (property === undefined) {
    return undefined;
  }
  if (ts.isMethodDeclaration(property)) {
    return property;
  }
  if (
    ts.isPropertyAssignment(property) &&
    (ts.isArrowFunction(property.initializer) || ts.isFunctionExpression(property.initializer))
  ) {
    return property.initializer;
  }
  return undefined;
}

function identifierProperty(
  object: ts.ObjectLiteralExpression,
  name: string,
): ts.Identifier | undefined {
  const property = namedProperty(object, name);
  if (property === undefined || !ts.isPropertyAssignment(property)) {
    return undefined;
  }
  return ts.isIdentifier(property.initializer) ? property.initializer : undefined;
}

function stringProperty(
  object: ts.ObjectLiteralExpression,
  name: string,
): string | undefined {
  const property = namedProperty(object, name);
  if (property === undefined || !ts.isPropertyAssignment(property)) {
    return undefined;
  }
  return ts.isStringLiteral(property.initializer) ? property.initializer.text : undefined;
}

function booleanProperty(
  object: ts.ObjectLiteralExpression,
  name: string,
): boolean | undefined {
  const property = namedProperty(object, name);
  if (property === undefined || !ts.isPropertyAssignment(property)) {
    return undefined;
  }
  return property.initializer.kind === ts.SyntaxKind.TrueKeyword
    ? true
    : property.initializer.kind === ts.SyntaxKind.FalseKeyword
      ? false
      : undefined;
}
