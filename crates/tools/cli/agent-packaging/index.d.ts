/**
 * One declared argument: what it is, and whether the Capability requires it.
 *
 * `required` is on the property rather than in a sibling list, so there is no
 * second place that can disagree about which arguments a Capability needs.
 */
export interface ToolProperty<Value, Required extends boolean> {
  readonly type: string;
  readonly required: Required;
  readonly __value?: Value;
}

/** Any declared property, whatever it carries and however it is required. */
export type AnyToolProperty = ToolProperty<unknown, boolean>;

type RequiredNames<Properties> = {
  [Name in keyof Properties]: Properties[Name] extends ToolProperty<unknown, true>
    ? Name
    : never;
}[keyof Properties];

type ValueOf<Property> = Property extends ToolProperty<infer Value, boolean>
  ? Value
  : never;

/**
 * The argument object a schema literal describes.
 *
 * The properties are the only place the shape is written, so a handler cannot
 * declare one shape and receive another.
 */
export type ArgumentsOf<Properties> =
  & { [Name in RequiredNames<Properties>]: ValueOf<Properties[Name]> }
  & { [Name in Exclude<keyof Properties, RequiredNames<Properties>>]?: ValueOf<Properties[Name]> };

/**
 * The closed argument schema one package-local Capability accepts.
 *
 * It carries the declared properties themselves rather than a widened map, so
 * `Tool.define` recovers which arguments are required from the same literal
 * that declared them.
 */
export interface ToolInput<Properties> {
  readonly type: "object";
  readonly properties: Properties;
  readonly required: readonly string[];
  readonly additionalProperties: boolean;
}

/** One typed business result from a package-local Capability handler. */
export interface ToolAnswer<Value extends object = object> {
  readonly kind: "apxm.tool-answer";
  readonly value: Value;
}

/** A complete TypeScript package-local Capability handler declaration. */
export interface ToolDefinition<Properties> {
  readonly name: string;
  readonly description: string;
  /** Whether this Capability is guaranteed not to mutate state outside itself. */
  readonly readOnly: boolean;
  readonly input: ToolInput<Properties>;
  readonly run: (
    args: ArgumentsOf<Properties>,
  ) => ToolAnswer | Promise<ToolAnswer>;
}

/**
 * The exact Capability reference a shipped handler both names and implements.
 *
 * `@apxm/frontend`'s `Tool` and `Capability` accept it directly, so a program
 * references the handler it ships rather than a string that has to agree with
 * one.
 */
export interface CapabilityId<Properties = unknown> {
  readonly kind: "tool";
  readonly capabilityId: string;
  readonly name: string;
  readonly description: string;
  readonly read_only: boolean;
  readonly schema: ToolInput<Properties>;
  readonly handler_id: string;
  readonly module: string;
  readonly qualname: string;
  readonly fn: (args: ArgumentsOf<Properties>) => ToolAnswer | Promise<ToolAnswer>;
}

/** The package-local Capability handler authoring object. */
export declare const Tool: Readonly<{
  define<Input>(definition: ToolDefinition<Input>): CapabilityId<Input>;
  // `Properties` is deliberately unconstrained: constraining it would give each
  // declared property a contextual type, and a contextual `boolean` widens the
  // `required: true` an author wrote back into `boolean` — which is exactly the
  // fact the argument type is derived from.
  object<Properties>(
    schema: {
      readonly additionalProperties: boolean;
      readonly properties: Properties;
    },
  ): ToolInput<Properties>;
  text<Required extends boolean>(options: {
    readonly required: Required;
    readonly minLength?: number;
    readonly description?: string;
  }): ToolProperty<string, Required>;
  integer<Required extends boolean>(options: {
    readonly required: Required;
    readonly minimum?: number;
    readonly description?: string;
  }): ToolProperty<number, Required>;
  answer<Value extends object>(value: Value): ToolAnswer<Value>;
}>;

export declare function isFunctionTool(value: unknown): value is CapabilityId<object>;
export declare function isToolAnswer(value: unknown): value is ToolAnswer;
export declare function makeHandlerId(module: string, qualname: string): string;
