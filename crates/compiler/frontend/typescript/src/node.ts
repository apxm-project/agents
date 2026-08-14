// Node-only host bridge kept outside the browser authoring entrypoint.
//
// It installs the compiler service and provides the one thing a JavaScript
// module cannot recover about itself: the text it was written as.

import { readFileSync } from "node:fs";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import {
  hasHostSuppliedSource,
  setAuthoredSource,
  setHostSuppliedSource,
  type AuthoredSource,
} from "./authored-source.js";
import { canonicalAirJson, compileArtifact, verifyGraph } from "./bridge.js";
import { installCompilerService } from "./compiler-service.js";

installCompilerService({
  verifyGraph,
  canonicalAir: canonicalAirJson,
  artifact: compileArtifact,
});

/**
 * State this module's own source, once, above its Agent definitions:
 * `source(import.meta.url)`.
 *
 * It is a no-op when a host has already supplied the text — the compiler's
 * source port evaluates submitted text that was never written to disk, so
 * there is no file for the module to read.
 */
export function source(url: string): void {
  if (hasHostSuppliedSource()) {
    return;
  }
  const absoluteFileName = fileURLToPath(url);
  const emitted = readFileSync(absoluteFileName, "utf8");
  setAuthoredSource(
    authoredBehind(absoluteFileName, emitted) ?? {
      fileName: relative(process.cwd(), absoluteFileName),
      text: emitted,
    },
  );
}

/**
 * Recover the authored source a build emitted this module from.
 *
 * A module that runs is usually not the module that was written: `tsc` erases
 * the types and moves the file. Capturing the emitted JavaScript would mean
 * capturing a program whose typed interface has already been deleted, and
 * attributing every span to a build artifact nobody wrote. A source map with
 * `inlineSources` carries the authored text, so the frontend reads that and
 * falls back to the running file only when the build states no map — which is
 * the case for a JavaScript module that was authored as it runs.
 */
function authoredBehind(
  emittedFileName: string,
  emitted: string,
): AuthoredSource | undefined {
  const reference = /\/\/# sourceMappingURL=(\S+)\s*$/m.exec(emitted)?.[1];
  if (reference === undefined) {
    return undefined;
  }
  const directory = dirname(emittedFileName);
  try {
    const raw = reference.startsWith("data:")
      ? decodeInlineSourceMap(reference)
      : readFileSync(resolve(directory, reference), "utf8");
    const map = JSON.parse(raw) as {
      sources?: unknown[];
      sourcesContent?: unknown[];
    };
    const [authoredName] = map.sources ?? [];
    const [authoredText] = map.sourcesContent ?? [];
    if (typeof authoredName !== "string" || typeof authoredText !== "string") {
      return undefined;
    }
    return {
      fileName: relative(process.cwd(), resolve(directory, authoredName)),
      text: authoredText,
    };
  } catch {
    return undefined;
  }
}

function decodeInlineSourceMap(reference: string): string {
  const payload = reference.slice(reference.indexOf(",") + 1);
  return reference.includes(";base64,")
    ? Buffer.from(payload, "base64").toString("utf8")
    : decodeURIComponent(payload);
}

/** Supply authored source a host already holds, ahead of any module's own. */
export function submitAuthoredSource(source: AuthoredSource): void {
  setHostSuppliedSource(source);
}
