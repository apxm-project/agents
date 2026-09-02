// The host-fulfilled Capability ids the package being captured declares.
//
// A host capability is declared in `agent.toml` as `[[capabilities.host]]` and
// referred to as `Capability("host:<id>")`. The runtime never executes one; the
// embedding host answers the request (ADR-0025). The declarations live in the
// package manifest, which the confined interpreter never reads, so the trusted
// host bridge supplies them before the submitted module is evaluated and the
// minted Capability set becomes the builtin catalogue united with these.
//
// Nothing here is reachable from the public authoring entrypoint: a program
// that could declare its own host capabilities would be minting its own
// authority.

let declared: ReadonlySet<string> = new Set();

/** Supply the host capability ids the package manifest declares. */
export function setDeclaredHostCapabilities(ids: readonly string[]): void {
  declared = new Set(ids.map((id) => `host:${id}`));
}

/** Whether this package declares the host capability `reference` names. */
export function isDeclaredHostCapability(reference: string): boolean {
  return declared.has(reference);
}

/** The references this package mints, in a stable order, for a diagnostic. */
export function declaredHostCapabilities(): readonly string[] {
  return [...declared].sort();
}
