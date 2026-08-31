// Private host bridge. This module is loaded by the trusted source-port
// harness directly from the package root; submitted modules can only resolve
// the public `@apxm/frontend/node` entrypoint, which intentionally excludes
// source injection APIs.

// Keep the compiler bridge initialization on the trusted Node-only path. This
// module is not exported by the public package and is used only by host-owned
// conformance/capture code.
import "./node.js";
import { setHostSuppliedSource, type AuthoredSource } from "./authored-source.js";

/** Supply authored source held by the trusted compiler host. */
export function submitAuthoredSource(source: AuthoredSource): void {
  setHostSuppliedSource(source);
}
