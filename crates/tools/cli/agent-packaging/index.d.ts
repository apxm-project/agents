/** A typed field declared without authoring JSON Schema directly. */
export interface ToolField<T> {
  readonly type: string;
  readonly __value?: T;
}

/** A typed input object accepted by one package-local Tool. */
export interface ToolInput<T extends object> {
  readonly type: "object";
  readonly properties: { readonly [K in keyof T]: ToolField<T[K]> };
  readonly required: readonly (keyof T & string)[];
  readonly additionalProperties: false;
}

/** One typed business result from a package-local Tool. */
export interface ToolAnswer<T extends object> {
  readonly kind: "apxm.tool-answer.v1";
  readonly value: T;
}

/** A complete TypeScript package-local Tool definition. */
export interface ToolDefinition<I extends object, O extends object> {
  readonly name: string;
  readonly description: string;
  readonly input: ToolInput<I>;
  readonly run: (input: I) => ToolAnswer<O> | Promise<ToolAnswer<O>>;
}

/** A compiled private handler descriptor, consumed by the Rust-owned manifest. */
export interface FunctionTool<I extends object, O extends object> {
  readonly kind: "tool";
  readonly name: string;
  readonly description: string;
  readonly schema: ToolInput<I>;
  readonly handler_id: string;
  readonly module: string;
  readonly qualname: string;
  readonly fn: (input: I) => ToolAnswer<O> | Promise<ToolAnswer<O>>;
}

/** The package-local Tool implementation object. */
export declare const Tool: Readonly<{
  define<I extends object, O extends object>(definition: ToolDefinition<I, O>): FunctionTool<I, O>;
  object<I extends object>(fields: { readonly [K in keyof I]: ToolField<I[K]> }): ToolInput<I>;
  text(options?: { readonly minLength?: number }): ToolField<string>;
  answer<O extends object>(value: O): ToolAnswer<O>;
}>;

export declare function isFunctionTool(value: unknown): value is FunctionTool<object, object>;
export declare function isToolAnswer(value: unknown): value is ToolAnswer<object>;
export declare function makeHandlerId(module: string, qualname: string): string;
