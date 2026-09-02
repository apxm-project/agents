// Capture one submitted TypeScript source text into `apxm.frontend-graph`.
//
// The port embeds this harness and runs it as the program text of a Node
// process. It reads `{"frontend_root", "entrypoint", "source",
// "host_capabilities"}` on stdin and
// writes `{"frontend_graph": ...}` on stdout. It emits typed source intent only:
// AIR lowering belongs to Rust, so this harness never prints AIR. Every
// rejection exits non-zero with one closed reason token on the first stderr line
// and its detail below, and prints nothing at all on stdout — there is no
// partial graph.
//
// Capturing typed intent from TypeScript source requires the TypeScript
// authoring frontend to run, and the frontend binds an authored callback through
// the module system, so the submitted text is evaluated. That boundary is
// explicit and constrained:
//
// * The submitted text is never materialized on disk. It is typechecked against
//   an in-memory compiler host and evaluated through in-memory module hooks, so
//   no directory, no symlink, and no temporary file is created for it.
// * Its module specifiers resolve through a closed table: the declared frontend
//   package and its Node host bridge. Every other specifier is rejected, so the
//   submitted text reaches no Node builtin and no package.
// * The typecheck runs first and rejects before any evaluation, so a source that
//   does not typecheck never executes.
// * The process runs under Node's permission model with read access to the
//   declared frontend package alone and write access to nothing. The harness
//   also removes process and network globals before evaluating submitted code;
//   a submission therefore cannot use ambient fetch or process bindings as an
//   alternate path around the closed module table.
//
// The submitted text shares this process's standard output. It cannot forge a
// result by writing to it: the port decodes the entire standard output as
// exactly one JSON document, so any injected byte makes the whole document
// unparseable and the port rejects with a diagnostic instead of returning a
// graph.

import { registerHooks } from "node:module";
import path from "node:path";

/** Closed reason tokens. The Rust port maps each to one typed diagnostic code. */
const REASON_REQUEST = "harness_request_invalid";
const REASON_FRONTEND = "frontend_unavailable";
const REASON_SOURCE = "source_rejected";
const REASON_ENTRYPOINT = "entrypoint_not_an_agent_program";

/** In-memory identities. Nothing on disk carries any of these names. */
const ROOT = "/apxm-submitted";
const ENTRY_FILE = `${ROOT}/submitted_source.ts`;
const ENTRY_URL = "apxm-submitted:///submitted_source.js";
const FRONTEND_SPECIFIER = "@apxm/frontend";
const FRONTEND_NODE_SPECIFIER = "@apxm/frontend/node";

// Keep the harness's own I/O handles before process is hidden from submitted
// code. The submitted module never needs process, and exposing it would make
// Node's internal bindings an ambient filesystem/network escape hatch.
const runtimeProcess = process;
const readStdin = runtimeProcess.stdin;
const writeStdout = runtimeProcess.stdout.write.bind(runtimeProcess.stdout);
const writeStderr = runtimeProcess.stderr.write.bind(runtimeProcess.stderr);
const exitProcess = runtimeProcess.exit.bind(runtimeProcess);
// The submitted module can replace the mutable global JSON.stringify. Keep
// output serialization on the harness-owned intrinsic so a forged serializer
// cannot replace the graph after the trusted handle has returned it.
const serializeOutput = JSON.stringify.bind(JSON);
// Capture code and graph emission execute in this child, after the submitted
// module has been typechecked. Freeze the standard intrinsic objects before
// evaluation so source cannot poison Array#map, Object#values, or another
// collection primitive that the trusted frontend uses while it is emitting the
// graph. The source still has its normal language surface; only process-wide
// mutable prototype state is closed. Bind the global names as well, otherwise
// source could replace a constructor property on globalThis and use the forged
// constructor for later calls.
const freezeIntrinsic = Object.freeze.bind(Object);
const intrinsicGlobals = [
  ["Object", Object],
  ["Function", Function],
  ["Array", Array],
  ["Map", Map],
  ["Set", Set],
  ["WeakMap", WeakMap],
  ["WeakSet", WeakSet],
  ["Promise", Promise],
  ["String", String],
  ["Number", Number],
  ["Boolean", Boolean],
  ["RegExp", RegExp],
  ["Date", Date],
  ["Error", Error],
  ["TypeError", TypeError],
  ["Uint8Array", Uint8Array],
  ["Uint16Array", Uint16Array],
  ["Uint32Array", Uint32Array],
  ["Int8Array", Int8Array],
  ["Int16Array", Int16Array],
  ["Int32Array", Int32Array],
  ["Float32Array", Float32Array],
  ["Float64Array", Float64Array],
];
for (const [name, intrinsic] of intrinsicGlobals) {
  freezeIntrinsic(intrinsic.prototype);
  freezeIntrinsic(intrinsic);
  Object.defineProperty(globalThis, name, {
    value: intrinsic,
    writable: false,
    configurable: false,
  });
}

function reject(reason, detail) {
  writeStderr(`${reason}\n${detail}\n`);
  exitProcess(1);
}

function readRequest() {
  return new Promise((resolve, fail) => {
    let buffer = "";
    readStdin.setEncoding("utf8");
    readStdin.on("data", (chunk) => {
      buffer += chunk;
    });
    readStdin.on("end", () => resolve(buffer));
    readStdin.on("error", fail);
  });
}

let request;
try {
  request = JSON.parse(await readRequest());
} catch (error) {
  reject(REASON_REQUEST, `harness request is not JSON: ${error.message}`);
}
if (request === null || typeof request !== "object" || Array.isArray(request)) {
  reject(REASON_REQUEST, "harness request is not a JSON object");
}
const {
  frontend_root: frontendPackage,
  entrypoint,
  source,
  host_capabilities: hostCapabilities = [],
} = request;
if (
  typeof frontendPackage !== "string" ||
  typeof entrypoint !== "string" ||
  typeof source !== "string"
) {
  reject(
    REASON_REQUEST,
    "harness request requires string 'frontend_root', 'entrypoint', and 'source'",
  );
}
if (
  !Array.isArray(hostCapabilities) ||
  hostCapabilities.some((id) => typeof id !== "string")
) {
  reject(
    REASON_REQUEST,
    "harness request 'host_capabilities' is an array of declared host capability ids",
  );
}

let ts;
try {
  ts = (
    await import(
      path.join(frontendPackage, "node_modules", "typescript", "lib", "typescript.js")
    )
  ).default;
} catch (error) {
  reject(
    REASON_FRONTEND,
    `the TypeScript compiler is not resolvable inside the declared frontend package root: ${error.message}`,
  );
}

const frontendTypes = path.join(frontendPackage, "dist", "index.d.ts");
const frontendModule = new URL(
  path.join(frontendPackage, "dist", "index.js"),
  "file:///",
).href;
const frontendNodeModule = new URL(
  path.join(frontendPackage, "dist", "node.js"),
  "file:///",
).href;
const frontendHostModule = new URL(
  path.join(frontendPackage, "dist", "host.js"),
  "file:///",
).href;

const options = {
  module: ts.ModuleKind.ESNext,
  moduleResolution: ts.ModuleResolutionKind.Bundler,
  noEmit: true,
  skipLibCheck: true,
  strict: true,
  target: ts.ScriptTarget.ES2022,
  baseUrl: ROOT,
  paths: {
    [FRONTEND_SPECIFIER]: [frontendTypes],
    [FRONTEND_NODE_SPECIFIER]: [path.join(frontendPackage, "dist", "node.d.ts")],
  },
};

const memory = new Map([[ENTRY_FILE, source]]);

let base;
try {
  base = ts.createCompilerHost(options, true);
} catch (error) {
  reject(REASON_FRONTEND, `the TypeScript compiler host is unavailable: ${error.message}`);
}

const host = {
  ...base,
  fileExists: (fileName) => memory.has(fileName) || base.fileExists(fileName),
  readFile: (fileName) => memory.get(fileName) ?? base.readFile(fileName),
  getSourceFile: (fileName, languageVersion, onError) =>
    memory.has(fileName)
      ? ts.createSourceFile(fileName, memory.get(fileName), languageVersion, true)
      : base.getSourceFile(fileName, languageVersion, onError),
  getCurrentDirectory: () => ROOT,
  writeFile: () => {},
};

let errors;
try {
  const program = ts.createProgram([ENTRY_FILE], options, host);
  errors = ts
    .getPreEmitDiagnostics(program)
    .filter((diagnostic) => diagnostic.category === ts.DiagnosticCategory.Error);
} catch (error) {
  reject(REASON_SOURCE, `${error.name}: ${error.message}`);
}
if (errors.length > 0) {
  reject(
    REASON_SOURCE,
    errors
      .map((diagnostic) => {
        const message = ts.flattenDiagnosticMessageText(diagnostic.messageText, " ");
        if (diagnostic.file === undefined || diagnostic.start === undefined) {
          return message;
        }
        const { line, character } = diagnostic.file.getLineAndCharacterOfPosition(
          diagnostic.start,
        );
        return `${line + 1}:${character + 1}: ${message}`;
      })
      .join("\n"),
  );
}

let transpiled;
try {
  transpiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
  }).outputText;
} catch (error) {
  reject(REASON_SOURCE, `${error.name}: ${error.message}`);
}

// The submitted module resolves through this closed table only. Any other
// specifier — a Node builtin, an installed package, a relative path — is not
// reachable, so the evaluated text has no ambient module surface. Synchronous
// `registerHooks` is intentional: the asynchronous `register`
// API starts a loader worker, which would require granting submitted code
// worker authority under Node's permission model.
registerHooks({
  resolve(specifier, context, next) {
    if (context.parentURL === ENTRY_URL) {
      if (specifier === FRONTEND_SPECIFIER) {
        return { url: frontendModule, shortCircuit: true };
      }
      if (specifier === FRONTEND_NODE_SPECIFIER) {
        return { url: frontendNodeModule, shortCircuit: true };
      }
      throw new Error(
        "the submitted source may not import '" +
          specifier +
          "'; only the authoring frontend and its Node host bridge are reachable",
      );
    }
    return next(specifier, context);
  },

  load(url, context, next) {
    if (url === ENTRY_URL) {
      return { format: "module", source: transpiled, shortCircuit: true };
    }
    return next(url, context);
  },
});

// The frontend reads the authored callback back from the module's own source
// text. The harness supplies that text from the submission it already holds, so
// the evaluated module never reads a file to recover its own source.
try {
  const bridge = await import(frontendHostModule);
  bridge.submitAuthoredSource({ fileName: path.basename(ENTRY_FILE), text: source });
  // The package manifest is not in this process. The trusted bridge is the one
  // path by which the minted Capability set stops being only the catalogue, and
  // it closes before any submitted code runs.
  bridge.declareHostCapabilities(hostCapabilities);
} catch (error) {
  reject(
    REASON_FRONTEND,
    `the TypeScript authoring frontend compiler bridge is not installed at the declared package root: ${error.message}`,
  );
}

let captured;
try {
  // These globals are not part of the authoring API. Remove them immediately
  // before evaluation, after the trusted frontend bridge has loaded.
  for (const name of ["process", "fetch", "WebSocket", "EventSource", "XMLHttpRequest"]) {
    Object.defineProperty(globalThis, name, {
      value: undefined,
      writable: false,
      configurable: false,
    });
  }
  const module = await import(ENTRY_URL);
  const definition = module[entrypoint];
  if (definition === undefined) {
    reject(
      REASON_ENTRYPOINT,
      `entrypoint '${entrypoint}' is not exported by the submitted source`,
    );
  }
  if (definition === null || typeof definition.frontendGraph !== "function") {
    reject(
      REASON_ENTRYPOINT,
      `entrypoint '${entrypoint}' is not an authored Agent program`,
    );
  }
  captured = definition.frontendGraph();
} catch (error) {
  reject(REASON_SOURCE, `${error.name}: ${error.message}`);
}

writeStdout(serializeOutput({ frontend_graph: captured }));
