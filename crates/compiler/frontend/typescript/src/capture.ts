// Static capture of an authored Agent callback into the bound tree.
//
// Parses the callback with the TypeScript compiler API, binds the declarations
// the module made above the Agent, and folds the supported control-flow subset
// into an immutable BoundProgram. It never executes the
// callback body: behavior is read from syntax and resolved declarations. Stable
// node and region identities come from the program identity plus lexical
// preorder, matching the Python frontend so paired goldens converge.
//
// The AST walk and the marker ergonomics live here; turning the captured tree
// into the language-neutral FrontendGraph is `emit.ts`, exactly as `_capture.py`
// and `_emit.py` split in the Python frontend.

import ts from "typescript";

import type {
  BoundCall,
  BoundCapabilityRequirement,
  BoundContextEdge,
  BoundControl,
  BoundDeclaration,
  BoundHook,
  BoundOperand,
  BoundPredicate,
  BoundProgram,
  BoundRegion,
  BoundValue,
  Span,
} from "./bound-tree.js";
import type { AuthoredSource } from "./authored-source.js";
import { emitFrontendGraph } from "./emit.js";
import { type Json } from "./contract.js";
import {
  HOOK_SCOPE_AGENT,
  HOOK_SCOPE_LOOP,
  HOOK_SCOPE_NODE,
  HOOK_SCOPES,
  HOOK_RETURN_MODE_OBSERVE,
  HOOK_RETURN_MODE_REPLACE_RESULT,
  INPUT_CONTRACT_ACCEPTS_EMPTY_OBJECT,
  REGION_ROLE_HOOK_BODY,
  type HookScope,
  type InputContract,
  type RegionRole,
} from "./generated/frontend-graph.js";
import type {
  CallIntent,
  PredicateLiteral,
  SkillRequirement,
  ValueExpression,
} from "./generated/frontend-records.js";
import {
    AGENT_BODY_NOT_ASYNC,
    AGENT_DYNAMIC_ARGUMENT,
    AGENT_MISSING_INPUT_OUTPUT,
    CAPABILITY_REF_NOT_EXACT,
    CONTEXT_NOT_TYPED,
    EVENT_NOT_TYPED,
    HOOK_DYNAMIC_REGISTRATION,
  HOOK_ORDER_AMBIGUOUS,
  HOOK_SCOPE_UNRESOLVED,
    HOOK_TARGET_UNRESOLVED,
    SKILL_LOAD_OUTSIDE_BODY,
    MODEL_UNTYPED_SCHEMA,
    type DiagnosticCode,
} from "./generated/diagnostics.js";
import * as scopeVocabulary from "./generated/scopes.js";
import { READ_SKILL } from "./generated/capabilities.js";
import type { Permission } from "./generated/permissions.js";
import type {
  CapabilityBinding,
  ContextSchema,
  EventTypeBinding,
  ModelBinding,
  SkillBinding,
  ToolBinding,
} from "./markers.js";
import { stableDigest } from "./markers.js";

/**
 * The declaration every `Skill(...).load()` invokes. One synthetic Capability
 * binding serves every skill a program declares: loading instructions is one
 * authority, not one per skill, and it is the authority the artifact states.
 */
const SKILL_READER_DECL_ID = "decl.capability.read_skill";

/** The typed interface of that binding: an identity in, a document out. */
const SKILL_READER_INPUT = "SkillRequest";
const SKILL_READER_OUTPUT = "SkillInstructions";

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
  | SkillBinding
  | ProgramBinding;

/** Whether two declarations request the very same decision, reason included. */
function sameRequestedPermission(
  left: Permission | undefined,
  right: Permission | undefined,
): boolean {
  if (left === undefined || right === undefined) {
    return left === right;
  }
  return left.decision === right.decision && left.reason === right.reason;
}

/** Source text an Agent capture reads the authored callback back from. */
export type StaticSource = AuthoredSource;

/**
 * Bound Agent declaration inputs consumed by the static AST capture pass.
 *
 * Only the identity an Agent states and the declarations its module has already
 * created are inputs. Type references, bindings, and their declaration ids are
 * read out of the authored source, exactly as the Python frontend reads them
 * out of the decorated function and its module globals.
 */
export type CaptureInput = {
  programId: string;
  entrypoint: string;
  contextSchema?: ContextSchema;
  declared: readonly object[];
  source: StaticSource;
};

/** The module-scope factories whose calls declare a binding an Agent can use. */
const MARKER_FACTORIES = ["Model", "Tool", "Capability", "Event", "Context", "Skill", "Agent"] as const;

/** The binding each factory produces, so a resolved pairing can be checked. */
const MARKER_BINDING_KINDS: Readonly<Record<string, Binding["kind"]>> = {
  Model: "model_binding",
  Tool: "tool_binding",
  Capability: "capability_binding",
  Event: "event_type",
  Skill: "skill",
  Context: "context",
  Agent: "agent_definition",
};

/**
 * The scope each published vocabulary name states, read off the generated
 * module rather than restated here. `scope: CAPABILITY` is the spelling the
 * surface teaches, so the capture resolves the symbol the author imported
 * instead of only recognizing the string it happens to equal.
 */
const SCOPE_BY_SYMBOL: ReadonlyMap<string, HookScope> = new Map(
  Object.entries(scopeVocabulary).filter(
    (entry): entry is [string, HookScope] =>
      typeof entry[1] === "string" && (HOOK_SCOPES as readonly string[]).includes(entry[1]),
  ),
);

/** The stable declaration identity a bound name carries into the graph. */
function declarationIdFor(name: string, binding: Binding): string {
  switch (binding.kind) {
    case "model_binding":
      return `decl.model.${name}`;
    case "tool_binding":
      return `decl.tool.${name}`;
    case "capability_binding":
      return `decl.capability.${name}`;
    case "event_type":
      return `decl.event.${name}`;
    case "skill":
      return SKILL_READER_DECL_ID;
    case "context":
      return `decl.context.${name}`;
    case "agent_definition":
      return binding.programId;
  }
}

/** Source diagnostic raised when an Agent construct is outside the closed subset. */
export class CaptureError extends Error {
  readonly code: DiagnosticCode;

  constructor(code: DiagnosticCode, message: string) {
    super(`${code}: ${message}`);
    this.name = "CaptureError";
    this.code = code;
  }
}

class Capture {
  private counter = 0;
  private readonly order = new Map<string, number>();
  private inputName = "input";

  private readonly declarations: BoundDeclaration[] = [];
  private readonly values: BoundValue[] = [];
  private readonly regions: BoundRegion[] = [];
  private readonly calls: BoundCall[] = [];
  private readonly controls: BoundControl[] = [];
  private readonly contextEdges: BoundContextEdge[] = [];
  private readonly hooks: BoundHook[] = [];
  private readonly importedPrograms = new Map<
    string,
    readonly [string, string, string, string]
  >();
  private readonly instances = new Map<ts.Symbol, string>();
  /** Named source values resolve to the value ids they produce. */
  private readonly valuesBySymbol = new Map<ts.Symbol, string>();
  private readonly bindingSymbols = new Map<ts.Symbol, string>();
  private readonly lastNodeByRegion = new Map<string, string>();
  private readonly pendingContextByRegion = new Map<string, { source: string; valueId: string }>();
  private readonly hookBodyRegions = new Set<string>();
  private readonly hookContextAssignment = new Map<string, string>();
  private readonly capabilityRequirements: BoundCapabilityRequirement[] = [];
  private readonly modelRequirements: string[] = [];
  private readonly skillRequirements: SkillRequirement[] = [];
  /** Declared skill bindings, resolved from the name the body loads through. */
  private readonly skillsByName = new Map<string, string>();
  private readonly spans: Array<readonly [string, Span, string]> = [];
  private readonly bodyRegionId: string;
  private programBindingSymbol: ts.Symbol | undefined;
  private facadeSymbol: ts.Symbol | undefined;
  private checker!: ts.TypeChecker;

  /** Module-scope declarations this Agent's source binds, resolved by name. */
  private readonly bindings = new Map<string, Binding>();
  private readonly bindingDeclIds = new Map<string, string>();
  /** Names the captured callback and the module's Hook bodies actually load. */
  private referencedNames = new Set<string>();
  private contextBindingName: string | undefined;
  private inputTypeRef = "Input";
  private inputContract: InputContract | undefined;
  private outputTypeRef = "Output";
  private contextTypeRef: string | undefined;

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
   * languages, so the order is fixed here rather than left to the order the
   * module happened to declare them in.
   */
  private declareBindings(): void {
    const declared = [...this.bindings.entries()].sort(([left], [right]) =>
      left < right ? -1 : left > right ? 1 : 0,
    );
    for (const [name, binding] of declared) {
      // A declaration the body never loads is not this program's declaration,
      // even though it is in scope. The Agent's own Context is the exception:
      // it is stated on the Agent, so it is declared whether the body reads it
      // or not. This is the rule the Python frontend applies to module globals.
      if (!this.referencedNames.has(name) && name !== this.contextBindingName) {
        continue;
      }
      const declId = declarationIdFor(name, binding);
      this.bindingDeclIds.set(name, declId);
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
      } else if (binding.kind === "skill") {
        this.skillsByName.set(name, binding.skillId);
        this.skillRequirements.push({
          skill_id: binding.skillId,
          instruction_source: binding.instructionSource,
        });
      } else if (binding.kind === "agent_definition") {
        this.importedPrograms.set(binding.programId, [
          binding.programId,
          binding.artifactDigest,
          binding.entrypoint,
          binding.targetAgentIdentityRequirement,
        ]);
      }
    }
    if (this.skillRequirements.length > 0) {
      this.declareSkillReader();
    }
  }

  /**
   * Declare the one Capability every `Skill(...).load()` invokes.
   *
   * It is declared last, after the author's own bindings, so both frontends
   * place it identically. One binding serves every declared skill: reading
   * instructions is a single authority the artifact states once, not one
   * authority per skill.
   */
  private declareSkillReader(): void {
    this.declarations.push({
      decl_id: SKILL_READER_DECL_ID,
      decl_kind: "capability_binding",
      input_type_ref: SKILL_READER_INPUT,
      output_type_ref: SKILL_READER_OUTPUT,
      target_ref: READ_SKILL,
    });
    this.requireCapability({ targetRef: READ_SKILL }, false);
  }

  /**
   * Record one authored Capability declaration in declaration order.
   *
   * Requirements are held as a list rather than keyed by `capability_ref` so a
   * reference declared both as a Tool and as a plain Capability keeps both
   * declarations. Only a declaration identical in every field collapses.
   */
  private requireCapability(
    binding: { targetRef: string; permission?: Permission },
    toolSchemaPresent: boolean,
  ): void {
    const requirement: BoundCapabilityRequirement = {
      capability_ref: binding.targetRef,
      tool_schema_present: toolSchemaPresent,
      requested_permission: binding.permission,
    };
    const present = this.capabilityRequirements.some(
      (existing) =>
        existing.capability_ref === requirement.capability_ref &&
        existing.tool_schema_present === requirement.tool_schema_present &&
        sameRequestedPermission(
          existing.requested_permission,
          requirement.requested_permission,
        ),
    );
    if (!present) {
      this.capabilityRequirements.push(requirement);
    }
  }

  capture(): BoundProgram {
    const { checker, source, emptyObjectType } = createBoundSource(this.input.source);
    this.checker = checker;
    const definition = this.findAgentDefinition(source);
    const fn = definition.callback;
    if ((ts.getCombinedModifierFlags(fn as ts.Declaration) & ts.ModifierFlags.Async) === 0) {
      throw new CaptureError(
        AGENT_BODY_NOT_ASYNC,
        "an Agent body is one async function",
      );
    }
    this.rejectDynamicAgentName(definition.call);
    this.readTypeArguments(definition.call, emptyObjectType);
    this.resolveModuleDeclarations(source, definition.call);
    this.collectReferencedNames(source, fn);
    this.declareBindings();
    this.bindDeclaredSymbols(source);
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
      type_ref: this.inputTypeRef,
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
    call: ts.CallExpression;
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
      throw new CaptureError(
        AGENT_MISSING_INPUT_OUTPUT,
        "Agent requires a statically declared run callback",
      );
    }
    return {
      call: selected.call,
      callback: selected.callback,
      bindingName: selected.bindingName,
    };
  }

  /**
   * Read the program's typed interface off `Agent<Input, Output, Context>`.
   *
   * The type arguments are the types, not strings naming them: an author who
   * renames a type renames its reference, and a reference to a type that does
   * not exist does not compile. Python reads the same three off the real
   * classes passed to `@Agent`.
   */
  private readTypeArguments(call: ts.CallExpression, emptyObjectType: ts.Type): void {
    const typeArguments = call.typeArguments;
    if (typeArguments === undefined || typeArguments.length < 2) {
      // JavaScript has no type arguments to read, so a `.mjs` program keeps the
      // closed default identities. TypeScript source has them, so omitting them
      // there would leave the program's typed interface unstated.
      if (this.sourceStatesTypes()) {
        throw new CaptureError(
          AGENT_MISSING_INPUT_OUTPUT,
          "an Agent states its typed interface as " +
            "Agent<Input, Output> or Agent<Input, Output, Context>",
        );
      }
      return;
    }
    this.inputTypeRef = typeArguments[0].getText().trim();
    const inputType = this.checker.getTypeFromTypeNode(typeArguments[0]);
    if (acceptsEmptyObject(this.checker, typeArguments[0], inputType, emptyObjectType)) {
      this.inputContract = INPUT_CONTRACT_ACCEPTS_EMPTY_OBJECT;
    }
    this.outputTypeRef = typeArguments[1].getText().trim();
  }

  /**
   * Whether this source can state type arguments at all.
   *
   * JavaScript has none to state, so a `.mjs` program keeps the closed default
   * identities; TypeScript source has them, so a declaration that omits them
   * there leaves its typed interface unstated.
   */
  private sourceStatesTypes(): boolean {
    return /\.[cm]?tsx?$/.test(this.input.source.fileName);
  }

  /** An Agent's own arguments are static source, never computed values. */
  private rejectDynamicAgentName(call: ts.CallExpression): void {
    const config = call.arguments[0];
    if (config === undefined || !ts.isObjectLiteralExpression(config)) {
      return;
    }
    if (
      namedProperty(config, "name") !== undefined &&
      stringProperty(config, "name") === undefined
    ) {
      throw new CaptureError(
        AGENT_DYNAMIC_ARGUMENT,
        "an Agent's name is static source, not a value the module computes",
      );
    }
  }

  /**
   * Bind every marker the module declared above this Agent.
   *
   * A module's declarations are its Agent's declarations — the frontend does
   * not ask an author to list again, in a `use` map, the names their own body
   * already names. JavaScript cannot enumerate a module scope, so the source
   * supplies the names in lexical order and the frontend supplies the values in
   * the order the module created them; the two are the same order, and every
   * pairing is checked against the marker the source actually called.
   */
  private resolveModuleDeclarations(
    source: ts.SourceFile,
    agentCall: ts.CallExpression,
  ): void {
    const stop = agentCall.getStart(source);
    const declared: Array<{
      name: string;
      factory: string;
      typeArgument?: string;
      typeArgumentCount: number;
    }> = [];
    for (const statement of source.statements) {
      if (statement.getEnd() >= stop) {
        break;
      }
      if (!ts.isVariableStatement(statement)) {
        continue;
      }
      for (const declaration of statement.declarationList.declarations) {
        const initializer = declaration.initializer;
        if (
          !ts.isIdentifier(declaration.name) ||
          initializer === undefined ||
          !ts.isCallExpression(initializer) ||
          !ts.isIdentifier(initializer.expression)
        ) {
          continue;
        }
        const factory = MARKER_FACTORIES.find((name) =>
          this.isFrontendMarker(initializer.expression as ts.Identifier, name),
        );
        if (factory === undefined) {
          continue;
        }
        declared.push({
          name: declaration.name.text,
          factory,
          typeArgument: initializer.typeArguments?.[0]?.getText().trim(),
          typeArgumentCount: initializer.typeArguments?.length ?? 0,
        });
      }
    }

    const values = this.input.declared.slice(
      this.input.declared.length - declared.length,
    );
    for (const [index, entry] of declared.entries()) {
      this.requireStatedTypes(entry);
      const value = values[index] as Binding | undefined;
      const expected = MARKER_BINDING_KINDS[entry.factory];
      if (value === undefined || value.kind !== expected) {
        throw new CaptureError(
          AGENT_DYNAMIC_ARGUMENT,
          `'${entry.name}' is declared by ${entry.factory} at module scope but the ` +
            "frontend resolved a different declaration; declare markers at module " +
            "scope, unconditionally, above the Agent that uses them",
        );
      }
      if (value.kind === "context") {
        const bound: ContextSchema = {
          ...value,
          typeRef: entry.typeArgument ?? value.typeRef,
        };
        if (value === this.input.contextSchema) {
          this.contextBindingName = entry.name;
          this.contextTypeRef = bound.typeRef;
        }
        this.bindings.set(entry.name, bound);
        continue;
      }
      this.bindings.set(entry.name, value);
    }
  }

  /**
   * A declaration whose types the surface requires states them in the source.
   *
   * Only the declarations whose manifest entry names a diagnostic for the
   * omission are checked: a rejection the published surface does not state is a
   * rejection an author has no way to anticipate.
   */
  private requireStatedTypes(entry: { name: string; factory: string; typeArgumentCount: number }): void {
    if (!this.sourceStatesTypes()) {
      return;
    }
    if (entry.factory === "Model" && entry.typeArgumentCount < 2) {
      throw new CaptureError(
        MODEL_UNTYPED_SCHEMA,
        `Model '${entry.name}' states the request and response it carries as ` +
          "Model<Input, Output>",
      );
    }
    if (entry.factory === "Context" && entry.typeArgumentCount < 1) {
      throw new CaptureError(
        CONTEXT_NOT_TYPED,
        `Context '${entry.name}' states its schema as Context<Schema>`,
      );
    }
  }

  /**
   * Collect the names the captured callback and the module's Hook bodies load.
   *
   * A Hook body is captured source too, so the Models and Capabilities it calls
   * have to be declared alongside the ones the Agent body calls.
   */
  private collectReferencedNames(
    source: ts.SourceFile,
    callback: ts.FunctionLikeDeclarationBase,
  ): void {
    const bodies: ts.Node[] = [callback];
    const visitForHooks = (node: ts.Node): void => {
      if (
        ts.isCallExpression(node) &&
        ts.isPropertyAccessExpression(node.expression) &&
        ts.isIdentifier(node.expression.expression) &&
        this.isFrontendMarker(node.expression.expression, "Hook")
      ) {
        const options = node.arguments[0];
        if (options !== undefined && ts.isObjectLiteralExpression(options)) {
          const run = functionProperty(options, "run");
          if (run !== undefined) {
            bodies.push(run);
          }
        }
      }
      ts.forEachChild(node, visitForHooks);
    };
    ts.forEachChild(source, visitForHooks);

    const names = new Set<string>();
    const collect = (node: ts.Node): void => {
      if (ts.isIdentifier(node)) {
        names.add(node.text);
      }
      ts.forEachChild(node, collect);
    };
    for (const body of bodies) {
      collect(body);
    }
    this.referencedNames = names;
  }

  private bindDeclaredSymbols(source: ts.SourceFile): void {
    for (const statement of source.statements) {
      if (!ts.isVariableStatement(statement)) {
        continue;
      }
      for (const declaration of statement.declarationList.declarations) {
        if (
          !ts.isIdentifier(declaration.name) ||
          !this.bindings.has(declaration.name.text)
        ) {
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
          type_ref: this.contextTypeRef ?? "Context",
          origin: "context_value",
          expression,
        });
        // Inside a captured Hook body the assignment *is* the Hook's return
        // value, so it is carried by the binding rather than wired to the next
        // node in the region. That is what makes an observing Hook
        // distinguishable from a replacing one by looking at the body.
        if (this.hookBodyRegions.has(regionId)) {
          this.hookContextAssignment.set(regionId, contextNode);
          return undefined;
        }
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
      throw new CaptureError(AGENT_DYNAMIC_ARGUMENT, "await targets a typed effect call");
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

    const operands = this.callOperands(call, resolved.slot);
    const contract: CallIntent = {
      node_id: nodeId,
      intent_kind: resolved.intent,
      parent_region_id: regionId,
      execution_order: this.orderIn(regionId),
      binding_ref: resolved.bindingRef,
      receiver_kind: resolved.receiverKind,
      result_value: resultValue,
    };
    this.calls.push({ contract, span: this.spanOf(call, source), operands });

    this.recordNode(regionId, nodeId);
    this.recordSpan(nodeId, resolved.intent, call, source);
    return resultValue;
  }

  private resolveCallTarget(call: ts.CallExpression): {
    intent: CallIntent["intent_kind"];
    bindingRef?: string;
    receiverKind?: CallIntent["receiver_kind"];
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
          : this.bindings.get(bindingName);
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
          throw new CaptureError(
            AGENT_DYNAMIC_ARGUMENT,
            "invoke receiver is a statically bound Agent definition or instance",
          );
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
          throw new CaptureError(
            AGENT_DYNAMIC_ARGUMENT,
            "new receiver is a statically bound Agent definition",
          );
        }
        return {
          intent: "agent_creation",
          bindingRef: programRef,
          slot: "initial_context",
        };
      }
      if (method === "load") {
        if (this.skillIdOfLoad(call) === undefined) {
          throw new CaptureError(
            SKILL_LOAD_OUTSIDE_BODY,
            "load receiver is a statically declared Skill",
          );
        }
        return {
          intent: "capability_invocation",
          bindingRef: SKILL_READER_DECL_ID,
          slot: "arguments",
        };
      }
      if (method === "wait") {
        const bindingRef = this.bindingRefOfBinding(bindingName);
        if (bindingRef === undefined) {
          throw new CaptureError(
            EVENT_NOT_TYPED,
            "wait receiver is a statically bound Event",
          );
        }
        return {
          intent: "event_wait",
          bindingRef,
          slot: "event_ref",
        };
      }
      throw new CaptureError(AGENT_DYNAMIC_ARGUMENT, `unsupported call '.${method}(...)'`);
    }
    if (ts.isIdentifier(callee)) {
      const bindingName = this.bindingNameFor(callee);
      const binding = bindingName === undefined
        ? undefined
        : this.bindings.get(bindingName);
      const declId = bindingName === undefined
        ? undefined
        : this.bindingDeclIds.get(bindingName);
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
        CAPABILITY_REF_NOT_EXACT,
        `call target '${callee.text}' is not a bound Model, Tool, or Capability`,
      );
    }
    throw new CaptureError(
      AGENT_DYNAMIC_ARGUMENT,
      "computed or dynamic call targets are not supported",
    );
  }

  private bindingRefOfBinding(name: string | undefined): string | undefined {
    if (name === undefined) {
      return undefined;
    }
    return this.bindingDeclIds.get(name) ?? name;
  }

  private programRefOfBinding(name: string | undefined): string | undefined {
    if (name === undefined) {
      return undefined;
    }
    const binding = this.bindings.get(name);
    return binding?.kind === "agent_definition" ? binding.programId : undefined;
  }

  private resultType(bindingRef: string | undefined): string {
    for (const decl of this.declarations) {
      if (decl.decl_id === bindingRef) {
        return decl.output_type_ref;
      }
    }
    return this.outputTypeRef;
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
        CONTEXT_NOT_TYPED,
        "value expressions call only a bound Context schema by name",
      );
    }
    const bindingName = this.bindingNameFor(callee);
    const binding = bindingName === undefined
      ? undefined
      : this.bindings.get(bindingName);
    if (binding?.kind === "context") {
      return;
    }
    if (
      binding?.kind === "model_binding" ||
      binding?.kind === "tool_binding" ||
      binding?.kind === "capability_binding"
    ) {
      throw new CaptureError(
        CAPABILITY_REF_NOT_EXACT,
        `'${callee.text}' is a typed effect and is called with await, not used as a value`,
      );
    }
    throw new CaptureError(
      CONTEXT_NOT_TYPED,
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
        throw new CaptureError(
          AGENT_DYNAMIC_ARGUMENT,
          "integer exceeds the shared safe integer domain",
        );
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
        throw new CaptureError(
          AGENT_DYNAMIC_ARGUMENT,
          "integer exceeds the shared safe integer domain",
        );
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
        throw new CaptureError(
          AGENT_DYNAMIC_ARGUMENT,
          "object expressions admit only static fields",
        );
      });
      return { kind: "object", fields };
    }
    if (ts.isArrayLiteralExpression(expression)) {
      if (expression.elements.some(ts.isSpreadElement)) {
        throw new CaptureError(
          AGENT_DYNAMIC_ARGUMENT,
          "array expressions do not admit spreads",
        );
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
    throw new CaptureError(AGENT_DYNAMIC_ARGUMENT, "unsupported pure value expression");
  }

  private staticPropertyName(name: ts.PropertyName): string {
    if (ts.isIdentifier(name) || ts.isStringLiteral(name) || ts.isNumericLiteral(name)) {
      return name.text;
    }
    throw new CaptureError(
      AGENT_DYNAMIC_ARGUMENT,
      "object expression keys are static strings",
    );
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

  /** The skill a `<name>.load()` call names, when it names one. */
  private skillIdOfLoad(call: ts.CallExpression): string | undefined {
    const callee = call.expression;
    if (
      !ts.isPropertyAccessExpression(callee) ||
      callee.name.text !== "load" ||
      !ts.isIdentifier(callee.expression)
    ) {
      return undefined;
    }
    const bindingName = this.bindingNameFor(callee.expression);
    return bindingName === undefined ? undefined : this.skillsByName.get(bindingName);
  }

  /**
   * The typed argument a skill load carries: the skill's own identity.
   *
   * The author writes no operand, so one is assembled here rather than left
   * out. It is an ordinary authored literal, which is what keeps the loaded
   * skill visible in the graph, in AIR, and to anything reading the effect's
   * operands.
   */
  private skillArgumentValue(skillId: string): string {
    const valueId = this.next("value");
    this.values.push({
      value_id: valueId,
      type_ref: "ArgumentValue",
      origin: "literal",
      expression: {
        kind: "object",
        fields: [{ name: "skill_id", value: { kind: "string", value: skillId } }],
      },
    });
    return valueId;
  }

  private callOperands(call: ts.CallExpression, slot: string): BoundOperand[] {
    const skillId = this.skillIdOfLoad(call);
    if (skillId !== undefined) {
      if (call.arguments.length > 0) {
        throw new CaptureError(
          SKILL_LOAD_OUTSIDE_BODY,
          "a Skill load takes no authored operand",
        );
      }
      return [{ value_id: this.skillArgumentValue(skillId), slot }];
    }
    if (call.arguments.length === 0) {
      return [];
    }
    if (call.arguments.length !== 1) {
      throw new CaptureError(
        AGENT_DYNAMIC_ARGUMENT,
        "typed effect calls accept exactly one authored operand",
      );
    }
    for (const argument of call.arguments) {
      if (ts.isSpreadElement(argument)) {
        throw new CaptureError(
          AGENT_DYNAMIC_ARGUMENT,
          "typed effect operands do not admit spreads",
        );
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
    return [{ value_id: this.valueForExpression(operandExpression), slot }];
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
      this.bindings.get(
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
    const operands = this.callOperands(call, resolved.slot);
    this.calls.push({
      contract: {
        node_id: nodeId,
        intent_kind: resolved.intent,
        parent_region_id: regionId,
        execution_order: this.orderIn(regionId),
        binding_ref: resolved.bindingRef,
        result_value: resultValue,
      },
      span: this.spanOf(call, source),
      operands,
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
      type_ref: this.inputTypeRef,
      origin: "resume_input",
      origin_id: nodeId,
    });
    const operands = this.callOperands(call, "output");
    this.controls.push({
      contract: {
        node_id: nodeId,
        control_kind: "yield",
        parent_region_id: regionId,
        execution_order: this.orderIn(regionId),
        result_value: resultValue,
      },
      span: this.spanOf(call, source),
      operands,
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
      throw new CaptureError(
        AGENT_DYNAMIC_ARGUMENT,
        "TaskGroup.run requires one static callback",
      );
    }
    const nodeId = this.next("task_group");
    // Keep the region identity language-independent: the Python frontend
    // represents the lexical TaskGroup body as `<node>.scope`.
    const childRegion = `${nodeId}.scope`;
    this.controls.push({
      contract: {
        node_id: nodeId,
        control_kind: "task_group",
        parent_region_id: regionId,
        execution_order: this.orderIn(regionId),
        body_region_ids: [childRegion],
      },
      span: this.spanOf(call, source),
      operands: [],
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

  private spanOf(node: ts.Node, source: ts.SourceFile): Span {
    const start = source.getLineAndCharacterOfPosition(node.getStart(source));
    const end = source.getLineAndCharacterOfPosition(node.getEnd());
    return {
      source_file: portableSourceFile(this.input.source.fileName),
      start_line: start.line + 1,
      start_column: start.character,
      end_line: end.line + 1,
      end_column: end.character,
    };
  }

  private recordSpan(
    nodeId: string,
    annotation: string,
    node: ts.Node,
    source: ts.SourceFile,
  ): void {
    this.spans.push([nodeId, this.spanOf(node, source), annotation]);
  }

  private recordNode(regionId: string, nodeId: string): void {
    const pending = this.pendingContextByRegion.get(regionId);
    if (pending !== undefined && pending.source !== nodeId) {
      this.contextEdges.push({
        from_node: pending.source,
        to_node: nodeId,
        context_type_ref: this.contextTypeRef ?? "Context",
        value_id: pending.valueId,
      });
      this.pendingContextByRegion.delete(regionId);
    }
    this.lastNodeByRegion.set(regionId, nodeId);
  }

  private addRegion(
    regionId: string,
    regionRole: RegionRole,
    parentRegionId: string | undefined,
    executionOrder: number,
  ): void {
    this.regions.push({
      region_id: regionId,
      region_role: regionRole,
      parent_region_id: parentRegionId,
      execution_order: executionOrder,
    });
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
      contract: {
        node_id: nodeId,
        control_kind: "loop",
        parent_region_id: regionId,
        execution_order: this.orderIn(regionId),
        body_region_ids: [bodyRegion],
      },
      span: this.spanOf(stmt, source),
      operands: [],
      predicate,
    });
    this.recordNode(regionId, nodeId);
    this.addRegion(bodyRegion, "loop_body", regionId, this.orderIn(regionId));
    let operands: BoundOperand[] = [];
    let resultValue: string | undefined;
    let initialValue: string | undefined;
    if (predicate !== undefined && sourceSymbol !== undefined) {
      initialValue = predicate.root_value_id;
      resultValue = this.next("value");
      const initialRecord = this.values.find((value) => value.value_id === initialValue);
      this.values.push({
        value_id: resultValue,
        type_ref: initialRecord?.type_ref ?? "ArgumentValue",
        origin: "block_argument",
        origin_id: `${bodyRegion}.block.0`,
      });
      this.valuesBySymbol.set(sourceSymbol, resultValue);
      predicate = { ...predicate, root_value_id: resultValue };
    }
    if (stmt.statement !== undefined) {
      this.visitBodyStatement(stmt.statement, bodyRegion, source);
    }
    if (resultValue !== undefined && initialValue !== undefined && sourceSymbol !== undefined) {
      const carriedValue = this.valuesBySymbol.get(sourceSymbol);
      if (carriedValue !== undefined) {
        operands = [
          { value_id: initialValue, slot: "initial" },
          { value_id: carriedValue, slot: "carried" },
        ];
      }
      this.valuesBySymbol.set(sourceSymbol, resultValue);
    }
    this.controls[controlIndex] = {
      contract: {
        node_id: nodeId,
        control_kind: "loop",
        parent_region_id: regionId,
        execution_order: this.orderIn(regionId),
        body_region_ids: [bodyRegion],
        result_value: resultValue,
      },
      span: this.spanOf(stmt, source),
      operands,
      predicate,
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
      contract: {
        node_id: nodeId,
        control_kind: "conditional",
        parent_region_id: regionId,
        execution_order: this.orderIn(regionId),
        body_region_ids: bodyRegions,
      },
      span: this.spanOf(stmt, source),
      operands: [],
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
          AGENT_DYNAMIC_ARGUMENT,
          "control predicate reads a prior typed value through static properties",
    );
  }

  private predicateLiteral(expression: ts.Expression): PredicateLiteral {
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
          AGENT_DYNAMIC_ARGUMENT,
          "predicate integer literal is within the shared safe-integer domain",
        );
      }
      return { scalar_type: "integer", value: integer };
    }
    throw new CaptureError(
      AGENT_DYNAMIC_ARGUMENT,
      "predicate equality compares with a scalar literal",
    );
  }

  private predicateForExpression(expression: ts.Expression): BoundPredicate | undefined {
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
      contract: {
        node_id: nodeId,
        control_kind: "try_catch",
        parent_region_id: regionId,
        execution_order: this.orderIn(regionId),
        body_region_ids: bodyRegions,
      },
      span: this.spanOf(stmt, source),
      operands: [],
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
    // A Hook body is not the Agent body: returning from it would otherwise
    // lower to a program return and end the whole invocation. A Hook states its
    // replacement by assigning Context.
    if (this.hookBodyRegions.has(regionId)) {
      if (stmt.expression !== undefined) {
        throw new CaptureError(
          HOOK_DYNAMIC_REGISTRATION,
          "a Hook returns its replacement by assigning agent.context",
        );
      }
      return;
    }
    if (stmt.expression !== undefined) {
      if (ts.isAwaitExpression(stmt.expression)) {
        this.visitAwait(stmt.expression, regionId, source);
      } else {
        this.rejectUnboundCalls(stmt.expression);
      }
    }
    const nodeId = this.next("return");
    this.controls.push({
      contract: {
        node_id: nodeId,
        control_kind: "return",
        parent_region_id: regionId,
        execution_order: this.orderIn(regionId),
      },
      span: this.spanOf(stmt, source),
      operands: [],
    });
    this.recordNode(regionId, nodeId);
  }

  private visitThrow(stmt: ts.ThrowStatement, regionId: string): void {
    this.rejectUnboundCalls(stmt.expression);
    const nodeId = this.next("throw");
    this.controls.push({
      contract: {
        node_id: nodeId,
        control_kind: "throw",
        parent_region_id: regionId,
        execution_order: this.orderIn(regionId),
      },
      operands: [],
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
        if (options === undefined || !ts.isObjectLiteralExpression(options)) {
          throw new CaptureError(
            HOOK_DYNAMIC_REGISTRATION,
            "a Hook is registered statically, so it states one object-literal declaration",
          );
        }
        hooks.push({
          declaration: node,
          phase: node.initializer.expression.name.text,
          options,
        });
      }
      ts.forEachChild(node, visit);
    };
    ts.forEachChild(source, visit);

    // Hooks are visited in the order the module declared them, which is the order
    // `declaration_order` states and therefore the order the schedule runs two
    // same-phase Hooks on one target in. Numbering is dense over the Hooks this
    // Agent keeps, so it does not depend on how many Hooks the module wrote for
    // some other Agent.
    for (const [order, hook] of hooks.entries()) {
      const agent = identifierProperty(hook.options, "agent");
      if (agent === undefined || this.symbolAt(agent) !== this.programBindingSymbol) {
        continue;
      }
      if (namedProperty(hook.options, "target") === undefined) {
        throw new CaptureError(
          HOOK_TARGET_UNRESOLVED,
          "a Hook states the target it wraps",
        );
      }
      const target = identifierProperty(hook.options, "target");
      if (target === undefined) {
        throw new CaptureError(
          HOOK_DYNAMIC_REGISTRATION,
          "a Hook target is one static bound declaration",
        );
      }
      const hookName = ts.isIdentifier(hook.declaration.name)
        ? hook.declaration.name.text
        : `hook_${order + 1}`;
      const scope = this.hookScope(hook.options, hookName);
      const run = functionProperty(hook.options, "run");
      if (run === undefined) {
        throw new CaptureError(
          HOOK_DYNAMIC_REGISTRATION,
          "a Hook body is one static run callback",
        );
      }
      const hookId = `hook.${hookName}`;
      const targetSelector = this.resolveHookTarget(target, scope);
      const bodyRegionId = this.captureHookBody(hookId, targetSelector, run, source);
      const assigned = this.hookContextAssignment.get(bodyRegionId);
      this.hooks.push({
        hook_id: hookId,
        scope,
        phase: hook.phase,
        target_selector: targetSelector,
        declaration_order: this.hooks.length,
        handler_ref: hookName,
        handler_digest: stableDigest(run.getText(source)),
        input_type_ref: "AgentFacade",
        output_type_ref:
          assigned === undefined ? "Unit" : (this.contextTypeRef ?? "Context"),
        return_mode:
          assigned === undefined
            ? HOOK_RETURN_MODE_OBSERVE
            : HOOK_RETURN_MODE_REPLACE_RESULT,
        body_region_id: bodyRegionId,
        assigned_context_value_id: assigned,
      });
    }
  }

  /**
   * Fold one Hook's `run` callback into its own region of the same bound tree.
   *
   * The body is read from syntax exactly like an Agent body, so a Hook's
   * Capability and Model calls become the same typed intents and stay visible
   * to the compiler. Where the region *runs* is decided by scope and phase
   * during lowering, not by where the callback happened to be written.
   */
  private captureHookBody(
    hookId: string,
    targetSelector: string,
    run: ts.FunctionLikeDeclarationBase,
    source: ts.SourceFile,
  ): string {
    const body = run.body;
    if (body === undefined || !ts.isBlock(body)) {
      throw new CaptureError(
        HOOK_DYNAMIC_REGISTRATION,
        `Hook '${hookId}' body is one static block`,
      );
    }
    const bodyRegionId = `${hookId}.body`;
    const parentRegionId = this.hookParentRegion(targetSelector);
    this.addRegion(
      bodyRegionId,
      REGION_ROLE_HOOK_BODY,
      parentRegionId,
      this.orderIn(parentRegionId),
    );
    this.hookBodyRegions.add(bodyRegionId);

    const outerFacade = this.facadeSymbol;
    const parameter = run.parameters[0];
    this.facadeSymbol =
      parameter !== undefined && ts.isIdentifier(parameter.name)
        ? this.symbolAt(parameter.name)
        : undefined;
    try {
      this.visitBlock(body.statements, bodyRegionId, source);
    } catch (error) {
      throw error instanceof CaptureError
        ? new CaptureError(error.code, `in Hook '${hookId}' body: ${error.message}`)
        : error;
    } finally {
      this.facadeSymbol = outerFacade;
    }
    return bodyRegionId;
  }

  /**
   * The region a Hook body is lexically held in. A node target puts the body
   * beside the node; a region target — the Agent body or a loop body — puts it
   * inside that region.
   */
  private hookParentRegion(targetSelector: string): string {
    const call = this.calls.find(
      (candidate) => candidate.contract.node_id === targetSelector,
    );
    if (call !== undefined) {
      return call.contract.parent_region_id;
    }
    const control = this.controls.find(
      (candidate) => candidate.contract.node_id === targetSelector,
    );
    if (control !== undefined) {
      return control.contract.parent_region_id;
    }
    return targetSelector;
  }

  /**
   * The scope a Hook declares, resolved fail-closed.
   *
   * `scope: CAPABILITY` and `scope: "capability"` are the same declaration, so
   * both resolve here. Anything else — a computed value, a name from somewhere
   * other than the scope vocabulary, a literal the vocabulary does not mint —
   * is rejected rather than defaulted: a mis-scoped Hook that still compiles is
   * a Hook that runs somewhere its author never asked for.
   */
  private hookScope(options: ts.ObjectLiteralExpression, hookName: string): HookScope {
    const property = namedProperty(options, "scope");
    if (property === undefined) {
      return HOOK_SCOPE_NODE;
    }
    const scope = ts.isPropertyAssignment(property)
      ? this.resolveScopeExpression(property.initializer)
      : undefined;
    if (scope === undefined) {
      throw new CaptureError(
        HOOK_SCOPE_UNRESOLVED,
        `Hook '${hookName}' states a scope the frontend cannot resolve to one of ` +
          `${HOOK_SCOPES.join(", ")}; write the @apxm/frontend/scopes symbol or ` +
          "the scope it names",
      );
    }
    return scope;
  }

  /** One scope expression, as a vocabulary symbol or as the string it names. */
  private resolveScopeExpression(expression: ts.Expression): HookScope | undefined {
    if (ts.isStringLiteral(expression)) {
      return (HOOK_SCOPES as readonly string[]).includes(expression.text)
        ? (expression.text as HookScope)
        : undefined;
    }
    if (ts.isPropertyAccessExpression(expression)) {
      return this.isScopeNamespace(expression.expression)
        ? SCOPE_BY_SYMBOL.get(expression.name.text)
        : undefined;
    }
    if (!ts.isIdentifier(expression)) {
      return undefined;
    }
    const imported = this.symbolAt(expression)?.declarations?.find(
      (declaration): declaration is ts.ImportSpecifier =>
        ts.isImportSpecifier(declaration) &&
        isScopeModuleSpecifier(findImportDeclaration(declaration)),
    );
    return imported === undefined
      ? undefined
      : SCOPE_BY_SYMBOL.get(imported.propertyName?.text ?? imported.name.text);
  }

  /** Whether a name is a namespace import of the scope vocabulary. */
  private isScopeNamespace(expression: ts.Expression): boolean {
    return (
      ts.isIdentifier(expression) &&
      (this.symbolAt(expression)?.declarations?.some(
        (declaration) =>
          ts.isNamespaceImport(declaration) &&
          isScopeModuleSpecifier(findImportDeclaration(declaration)),
      ) ??
        false)
    );
  }

  private resolveHookTarget(target: ts.Identifier, scope: HookScope): string {
    const named = target.text;
    if (this.regions.some((region) => region.region_id === named)) {
      return named;
    }
    const bindingRef = this.bindingRefOfBinding(this.bindingNameFor(target));
    const calls = this.calls.filter(
      (candidate) => candidate.contract.binding_ref === bindingRef,
    );
    const namesTheProgram = this.symbolAt(target) === this.programBindingSymbol;

    if (scope === HOOK_SCOPE_AGENT) {
      // The scope decides which boundary is wrapped, and the only boundary an
      // Agent scope has is the Agent's own body region. Resolving to the
      // invocation the target happened to name would emit a Hook lowering
      // refuses: it admits a region for this scope and nothing else.
      if (!namesTheProgram && calls.length === 0) {
        throw new CaptureError(
          HOOK_TARGET_UNRESOLVED,
          `Hook target '${named}' is not the Agent or a captured region`,
        );
      }
      return this.bodyRegionId;
    }
    if (scope === HOOK_SCOPE_LOOP) {
      // A loop is not a name an author can write, so a loop Hook selects its
      // loop through something inside it. Falling back to the first loop only
      // when nothing was named is what lets an authored target reach an inner
      // loop instead of being silently discarded.
      if (calls.length > 1) {
        throw new CaptureError(
          HOOK_ORDER_AMBIGUOUS,
          `Hook target '${named}' is ambiguous across invocations`,
        );
      }
      if (calls.length === 1) {
        const loopBody = this.enclosingLoopBody(calls[0].contract.node_id);
        if (loopBody === undefined) {
          throw new CaptureError(
            HOOK_TARGET_UNRESOLVED,
            `Hook target '${named}' is not inside a static loop`,
          );
        }
        return loopBody;
      }
      if (!namesTheProgram) {
        throw new CaptureError(
          HOOK_TARGET_UNRESOLVED,
          `Hook target '${named}' has no static invocation`,
        );
      }
      const loop = this.controls.find(
        (control) => control.contract.control_kind === "loop",
      );
      const region = loop?.contract.body_region_ids?.[0];
      if (region === undefined) {
        throw new CaptureError(
          HOOK_TARGET_UNRESOLVED,
          "a loop Hook requires a static loop in the Agent body",
        );
      }
      return region;
    }
    if (calls.length === 0) {
      throw new CaptureError(
        HOOK_TARGET_UNRESOLVED,
        `Hook target '${named}' has no static invocation`,
      );
    }
    if (calls.length !== 1) {
      throw new CaptureError(
        HOOK_ORDER_AMBIGUOUS,
        `Hook target '${named}' is ambiguous across invocations`,
      );
    }
    return calls[0].contract.node_id;
  }

  /** Walk out from one captured node to the loop body region holding it. */
  private enclosingLoopBody(nodeId: string): string | undefined {
    const loopBodies = new Set(
      this.controls
        .filter((control) => control.contract.control_kind === "loop")
        .flatMap((control) => control.contract.body_region_ids?.slice(0, 1) ?? []),
    );
    const parents = new Map(
      this.regions.map((region) => [region.region_id, region.parent_region_id]),
    );
    let regionId: string | undefined = this.hookParentRegion(nodeId);
    while (regionId !== undefined) {
      if (loopBodies.has(regionId)) {
        return regionId;
      }
      regionId = parents.get(regionId);
    }
    return undefined;
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

  private build(): BoundProgram {
    return {
      program_id: this.input.programId,
      entrypoint: this.input.entrypoint,
      input_type_ref: this.inputTypeRef,
      input_contract: this.inputContract,
      output_type_ref: this.outputTypeRef,
      has_default_context: (this.input.contextSchema?.defaultPresent ?? false),
      context_type_ref: this.contextTypeRef,
      parameters: [
        {
          value_id: `${this.input.programId}.param.agent`,
          type_ref: "AgentFacade",
          role: "agent_facade",
        },
        {
          value_id: `${this.input.programId}.param.input`,
          type_ref: this.inputTypeRef,
          role: "input",
        },
      ],
      body_region_id: this.bodyRegionId,
      declarations: this.declarations,
      values: this.values,
      regions: this.regions,
      calls: this.calls,
      controls: this.controls,
      context_edges: this.contextEdges,
      hooks: this.hooks,
      imported_programs: [...this.importedPrograms.values()],
      capability_requirements: this.capabilityRequirements,
      model_requirements: this.modelRequirements,
      skill_requirements: this.skillRequirements,
      spans: this.spans,
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
  return emitFrontendGraph(new Capture(input).capture());
}

function createBoundSource(input: StaticSource): {
  source: ts.SourceFile;
  checker: ts.TypeChecker;
  emptyObjectType: ts.Type;
} {
  const sourceText = `${input.text}\n\nconst __apxm_empty_object_probe = {};\n`;
  const libraryFileName = "__apxm_minimal_lib.d.ts";
  const libraryText = `
interface Array<T> {
  readonly length: number;
  readonly [index: number]: T;
}
interface ReadonlyArray<T> {
  readonly length: number;
  readonly [index: number]: T;
}
interface Promise<T> {}
`;
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
    fileExists: (fileName) => fileName === input.fileName || fileName === libraryFileName,
    readFile: (fileName) => fileName === input.fileName
      ? sourceText
      : fileName === libraryFileName
        ? libraryText
        : undefined,
    getSourceFile: (fileName, languageVersion) =>
      fileName === input.fileName
        ? ts.createSourceFile(fileName, sourceText, languageVersion, true)
        : fileName === libraryFileName
          ? ts.createSourceFile(fileName, libraryText, languageVersion, true)
        : undefined,
    getDefaultLibFileName: () => libraryFileName,
    writeFile: () => {},
    getCurrentDirectory: () => "",
    getDirectories: () => [],
    getCanonicalFileName: (fileName) => fileName,
    useCaseSensitiveFileNames: () => true,
    getNewLine: () => "\n",
  };
  const program = ts.createProgram({
    rootNames: [input.fileName, libraryFileName],
    options,
    host,
  });
  const source = program.getSourceFile(input.fileName);
  if (source === undefined) {
    throw new CaptureError(
      AGENT_DYNAMIC_ARGUMENT,
      `static source '${input.fileName}' is unavailable`,
    );
  }
  const checker = program.getTypeChecker();
  const probe = source.statements.at(-1);
  if (
    probe === undefined ||
    !ts.isVariableStatement(probe) ||
    probe.declarationList.declarations.length !== 1 ||
    probe.declarationList.declarations[0]?.initializer === undefined
  ) {
    throw new CaptureError(
      AGENT_DYNAMIC_ARGUMENT,
      "the compiler could not construct its empty-object type probe",
    );
  }
  return {
    source,
    checker,
    emptyObjectType: checker.getTypeAtLocation(
      probe.declarationList.declarations[0].initializer as ts.Expression,
    ),
  };
}

/**
 * Prove the caller may submit `{}` from the checked input type itself.
 * Broad unions, dynamic `any`, arrays, and object types with index or callable
 * behaviour stay unsupported even when TypeScript's assignability relation is
 * permissive for them.
 */
function acceptsEmptyObject(
  checker: ts.TypeChecker,
  typeNode: ts.TypeNode,
  inputType: ts.Type,
  emptyObjectType: ts.Type,
): boolean {
  if (
    inputType.flags &
      (ts.TypeFlags.Any | ts.TypeFlags.Never | ts.TypeFlags.Union | ts.TypeFlags.Intersection)
  ) {
    return false;
  }
  if ((inputType.flags & ts.TypeFlags.Unknown) !== 0) {
    return true;
  }
  if (ts.isArrayTypeNode(typeNode) || ts.isTupleTypeNode(typeNode)) {
    return false;
  }
  if (checker.isArrayType(inputType) || checker.isTupleType(inputType)) {
    return false;
  }
  if ((inputType.flags & ts.TypeFlags.Object) === 0) {
    return false;
  }
  if (!checker.isTypeAssignableTo(emptyObjectType, inputType)) {
    return false;
  }
  if (
    checker.getSignaturesOfType(inputType, ts.SignatureKind.Call).length > 0 ||
    checker.getSignaturesOfType(inputType, ts.SignatureKind.Construct).length > 0 ||
    checker.getIndexTypeOfType(inputType, ts.IndexKind.String) !== undefined ||
    checker.getIndexTypeOfType(inputType, ts.IndexKind.Number) !== undefined
  ) {
    return false;
  }
  return checker.getPropertiesOfType(inputType).every(
    (property) => (property.flags & ts.SymbolFlags.Optional) !== 0,
  );
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

/** The public path the Hook scope vocabulary is published on. */
function isScopeModuleSpecifier(declaration: ts.ImportDeclaration | undefined): boolean {
  if (declaration === undefined || !ts.isStringLiteral(declaration.moduleSpecifier)) {
    return false;
  }
  const specifier = declaration.moduleSpecifier.text;
  return specifier === "@apxm/frontend/scopes" || specifier === "../src/scopes.ts";
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
