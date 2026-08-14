// The authored source text an Agent capture reads itself back from.
//
// Python recovers the authored text through `inspect`; JavaScript has no
// equivalent, so a module states its own source once with `source(import.meta.url)`
// from `@apxm/frontend/node` and every Agent defined afterwards in that module
// reads it here. It is not an Agent argument: an Agent's source is a fact about
// where it was written, not a decision its author makes.
//
// A host that already holds the text — the compiler's source port, which
// evaluates submitted text that was never written to disk — supplies it
// directly, and that supply wins: the submitted module has no file to read.

export type AuthoredSource = {
  readonly fileName: string;
  readonly text: string;
  readonly line?: number;
};

let hostSupplied: AuthoredSource | undefined;
let moduleDeclared: AuthoredSource | undefined;

/** Record the source text of the module currently being evaluated. */
export function setAuthoredSource(source: AuthoredSource): void {
  moduleDeclared = source;
}

/** Supply source text the host already holds, ahead of any module's own. */
export function setHostSuppliedSource(source: AuthoredSource): void {
  hostSupplied = source;
}

/** Whether a host has already supplied the text being captured. */
export function hasHostSuppliedSource(): boolean {
  return hostSupplied !== undefined;
}

/** The authored source an Agent definition is captured from. */
export function authoredSource(): AuthoredSource {
  const source = hostSupplied ?? moduleDeclared;
  if (source === undefined) {
    throw new Error(
      "an Agent module states its own source once with `source(import.meta.url)` from \"@apxm/frontend/node\"",
    );
  }
  return source;
}
