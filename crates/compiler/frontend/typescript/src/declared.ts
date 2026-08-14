// The declarations a module has created, in the order it created them.
//
// Python resolves an Agent's declarations from its module globals. JavaScript
// exposes no module scope to read, so the frontend keeps the one fact the
// language does not: creation order. An Agent capture pairs it with the lexical
// order of the marker calls in the authored source above the Agent, and checks
// every pairing against the marker the source actually called — so an author
// never restates, in a `use` map, the names their own body already names.
//
// The registry is `object` rather than the binding union because `capture.ts`
// owns that union and imports the markers; typing it here would close a cycle.

const declared: object[] = [];

/** Record one module-scope declaration and return it unchanged. */
export function recordDeclaration<T extends object>(declaration: T): T {
  declared.push(declaration);
  return declaration;
}

/** Every declaration created before this point, oldest first. */
export function declaredSoFar(): readonly object[] {
  return declared;
}
