// Capture one submitted TypeScript source text into `apxm.frontend-graph.v2`.
//
// The port embeds this harness and runs it as the program text of a Node
// process. It reads `{"frontend_root", "entrypoint", "source"}` on stdin and
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
//   package and the harness-provided static-source module. Every other specifier
//   is rejected, so the submitted text reaches no Node builtin and no package.
// * The typecheck runs first and rejects before any evaluation, so a source that
//   does not typecheck never executes.
// * The process runs under Node's permission model with read access to the
//   declared frontend package alone and write access to nothing. That wall is
//   enforced below the module system, so it also holds against the filesystem
//   reachable through the running process itself, which names no module and
//   therefore passes both the typecheck and the resolve table above.
//
// The submitted text shares this process's standard output. It cannot forge a
// result by writing to it: the port decodes the entire standard output as
// exactly one JSON document, so any injected byte makes the whole document
// unparseable and the port rejects with a diagnostic instead of returning a
// graph.

import { register } from "node:module";
import path from "node:path";

/** Closed reason tokens. The Rust port maps each to one typed diagnostic code. */
const REASON_REQUEST = "harness_request_invalid";
const REASON_FRONTEND = "frontend_unavailable";
const REASON_SOURCE = "source_rejected";
const REASON_ENTRYPOINT = "entrypoint_not_an_agent_program";

/** In-memory identities. Nothing on disk carries any of these names. */
const ROOT = "/apxm-submitted";
const ENTRY_FILE = `${ROOT}/submitted_source.ts`;
const SOURCE_DECLARATION = `${ROOT}/apxm-source.d.ts`;
const ENTRY_URL = "apxm-submitted:///submitted_source.js";
const SOURCE_URL = "apxm-submitted:///apxm-source.js";
const SOURCE_SPECIFIER = "apxm:source";
const FRONTEND_SPECIFIER = "@apxm/frontend";
const FRONTEND_NODE_SPECIFIER = "@apxm/frontend/node";

function reject(reason, detail) {
  process.stderr.write(`${reason}\n${detail}\n`);
  process.exit(1);
}

function readRequest() {
  return new Promise((resolve, fail) => {
    let buffer = "";
    process.stdin.setEncoding("utf8");
    process.stdin.on("data", (chunk) => {
      buffer += chunk;
    });
    process.stdin.on("end", () => resolve(buffer));
    process.stdin.on("error", fail);
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
const { frontend_root: frontendPackage, entrypoint, source } = request;
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

// The frontend reads the authored callback back from a static source token. The
// harness supplies that token from the submitted text it already holds, so the
// evaluated module never reads a file to recover its own source.
const sourceModuleText =
  `export function staticSource() { return { fileName: ${JSON.stringify(
    path.basename(ENTRY_FILE),
  )}, text: ${JSON.stringify(source)} }; }\n`;
const sourceDeclarationText =
  "export declare function staticSource(): { fileName: string; text: string };\n";

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
    [SOURCE_SPECIFIER]: [SOURCE_DECLARATION],
  },
};

const memory = new Map([
  [ENTRY_FILE, source],
  [SOURCE_DECLARATION, sourceDeclarationText],
]);

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
// reachable, so the evaluated text has no ambient module surface. The
// `register` API is available on the repository's Node toolchain; newer
// `registerHooks` is not, so the hooks are carried by a data URL loader.
const loader = `
const ENTRY_URL = ${JSON.stringify(ENTRY_URL)};
const SOURCE_URL = ${JSON.stringify(SOURCE_URL)};
const FRONTEND_SPECIFIER = ${JSON.stringify(FRONTEND_SPECIFIER)};
const FRONTEND_NODE_SPECIFIER = ${JSON.stringify(FRONTEND_NODE_SPECIFIER)};
const SOURCE_SPECIFIER = ${JSON.stringify(SOURCE_SPECIFIER)};
const frontendModule = ${JSON.stringify(frontendModule)};
const frontendNodeModule = ${JSON.stringify(frontendNodeModule)};
const transpiled = ${JSON.stringify(transpiled)};
const sourceModuleText = ${JSON.stringify(sourceModuleText)};

export function resolve(specifier, context, next) {
  if (context.parentURL === ENTRY_URL) {
    if (specifier === FRONTEND_SPECIFIER) {
      return { url: frontendModule, shortCircuit: true };
    }
    if (specifier === FRONTEND_NODE_SPECIFIER) {
      return { url: frontendNodeModule, shortCircuit: true };
    }
    if (specifier === SOURCE_SPECIFIER) {
      return { url: SOURCE_URL, shortCircuit: true };
    }
    throw new Error(
      \`the submitted source may not import '\${specifier}'; only the authoring frontend and its static source token are reachable\`,
    );
  }
  return next(specifier, context);
}

export function load(url, context, next) {
  if (url === ENTRY_URL) {
    return { format: "module", source: transpiled, shortCircuit: true };
  }
  if (url === SOURCE_URL) {
    return { format: "module", source: sourceModuleText, shortCircuit: true };
  }
  return next(url, context);
}
`;
register(`data:text/javascript,${encodeURIComponent(loader)}`, import.meta.url);

try {
  await import(frontendNodeModule);
} catch (error) {
  reject(
    REASON_FRONTEND,
    `the TypeScript authoring frontend compiler bridge is not installed at the declared package root: ${error.message}`,
  );
}

let captured;
try {
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

process.stdout.write(JSON.stringify({ frontend_graph: captured }));
