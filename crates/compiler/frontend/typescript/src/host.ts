// Private host bridge. This module is loaded by the trusted source-port
// harness directly from the package root; submitted modules can only resolve
// the public `@apxm/frontend/node` entrypoint, which intentionally excludes
// source injection APIs.

// Keep the compiler bridge initialization on the trusted Node-only path. This
// module is not exported by the public package and is used only by host-owned
// conformance/capture code.
import "./node.js";
import { setHostSuppliedSource, type AuthoredSource } from "./authored-source.js";
import { setDeclaredHostCapabilities } from "./host-capabilities.js";

/** Supply authored source held by the trusted compiler host. */
export function submitAuthoredSource(source: AuthoredSource): void {
  setHostSuppliedSource(source);
}

/**
 * Supply the host capability ids the package manifest declares, without the
 * reserved `host:` prefix.
 *
 * The manifest is not visible inside the confined interpreter, so this is how
 * the minted Capability set stops being only the builtin catalogue. It is on
 * the private bridge rather than the public entrypoint because a program that
 * could declare its own host capabilities would be minting its own authority.
 */
export function declareHostCapabilities(ids: readonly string[]): void {
  setDeclaredHostCapabilities(ids);
}
