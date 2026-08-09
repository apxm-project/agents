"""Capture one submitted Python source text into `apxm.frontend-graph.v2`.

The port embeds this harness and runs it as the program text of an isolated
interpreter. It reads `{"frontend_root", "entrypoint", "source"}` on stdin and
writes `{"frontend_graph": ...}` on stdout. It emits typed source intent only:
AIR lowering belongs to Rust, so this harness never prints AIR. Every rejection
exits non-zero with one closed reason token on the first stderr line and its
detail below, and prints nothing at all on stdout — there is no partial graph.

Capturing typed intent from Python source requires the Python authoring frontend
to run, and the frontend resolves an authored callback through the import
system, so the submitted text is executed. That boundary is explicit and
constrained:

* The submitted text is never materialized on disk. It is compiled from memory
  and bound through an in-memory loader, with its lines registered in
  `linecache` so the frontend's static reader sees exactly the submitted text.
* The interpreter runs isolated, so no ambient `PYTHONPATH`, user site
  directory, or `PYTHON*` environment variable reaches it, and `sys.path` holds
  the standard library plus the one declared frontend package root.
* The frontend package is resolved before the lockdown closes, so the submitted
  text runs against an already-closed module graph: an audit hook rejects
  importing any module that is not already resolved, and rejects process,
  network, dynamic code loading, and filesystem events outright.
* A zero file-size limit, a CPU-time limit, and an address-space limit bound
  what the submitted text can consume where no audit event exists.

The submitted text shares this process's standard output, so it can write raw
bytes to that descriptor. It cannot forge a result by doing so: the port decodes
the entire standard output as exactly one JSON document, so any injected byte
makes the whole document unparseable and the port rejects with a diagnostic
instead of returning a graph.
"""

from __future__ import annotations

import importlib.abc
import importlib.util
import json
import linecache
import resource
import sys
import sysconfig

#: Closed reason tokens. The Rust port maps each to one typed diagnostic code.
REASON_REQUEST = "harness_request_invalid"
REASON_FRONTEND = "frontend_unavailable"
REASON_SOURCE = "source_rejected"
REASON_ENTRYPOINT = "entrypoint_not_an_agent_program"

#: The name the submitted module is bound under and the synthetic file name its
#: compiled code carries. Nothing on disk has either name.
MODULE_NAME = "apxm_submitted_source"
SOURCE_FILE_NAME = "submitted_source.py"

#: Audit-event prefixes the submitted text may never raise. `import` is handled
#: separately: importing an already-resolved module is legitimate.
DENIED_EVENT_PREFIXES = (
    "socket.",
    "subprocess.",
    "urllib.",
    "ftplib.",
    "smtplib.",
    "imaplib.",
    "poplib.",
    "http.client.",
    "ctypes.",
    "shutil.",
    "pty.",
    "webbrowser.",
    "glob.",
    "mmap.",
    "winreg.",
    "cpython.run_",
    "os.system",
    "os.exec",
    "os.fork",
    "os.posix_spawn",
    "os.spawn",
    "os.startfile",
    "os.kill",
    "os.remove",
    "os.rename",
    "os.mkdir",
    "os.rmdir",
    "os.link",
    "os.symlink",
    "os.chmod",
    "os.chown",
    "os.truncate",
    "os.putenv",
    "os.unsetenv",
    "os.listdir",
    "os.scandir",
    "os.chdir",
    "sys.addaudithook",
    "sys.setprofile",
    "sys.settrace",
    "sys._current_frames",
    "code.",
    "pickle.",
    "marshal.",
)

#: Exact audit events denied outright. The frontend opens no file after it is
#: resolved, and neither does the submitted text.
DENIED_EVENTS = ("open", "builtins.input", "builtins.breakpoint")

#: CPU seconds and address-space bytes the submitted text may consume.
CPU_LIMIT_SECONDS = 15
ADDRESS_SPACE_LIMIT_BYTES = 2 * 1024 * 1024 * 1024


class Rejected(Exception):
    """One closed rejection reason and its detail."""

    def __init__(self, reason: str, detail: str) -> None:
        super().__init__(detail)
        self.reason = reason
        self.detail = detail


def _read_request() -> tuple[str, str, str]:
    try:
        request = json.loads(sys.stdin.read())
    except ValueError as error:
        raise Rejected(REASON_REQUEST, f"harness request is not JSON: {error}") from None
    if not isinstance(request, dict):
        raise Rejected(REASON_REQUEST, "harness request is not a JSON object")
    frontend_root = request.get("frontend_root")
    entrypoint = request.get("entrypoint")
    source = request.get("source")
    if (
        not isinstance(frontend_root, str)
        or not isinstance(entrypoint, str)
        or not isinstance(source, str)
    ):
        raise Rejected(
            REASON_REQUEST,
            "harness request requires string 'frontend_root', 'entrypoint', and 'source'",
        )
    return frontend_root, entrypoint, source


def _load_frontend(frontend_root: str) -> None:
    """Resolve the declared authoring frontend before the lockdown closes."""
    interpreter_roots = (
        sysconfig.get_path("stdlib"),
        sysconfig.get_path("platstdlib"),
        sysconfig.get_config_var("DESTSHARED"),
    )
    sys.path[:] = list(
        dict.fromkeys(
            [frontend_root, *(root for root in interpreter_roots if root is not None)]
        )
    )
    try:
        import apxm_program  # noqa: F401
    except BaseException as error:  # noqa: BLE001 - any resolution failure is unavailability
        raise Rejected(
            REASON_FRONTEND,
            "the Python authoring frontend is not resolvable at the declared "
            f"package root: {type(error).__name__}: {error}",
        ) from None


def _lock_down() -> None:
    """Close the interpreter around the already-resolved frontend."""
    resource.setrlimit(resource.RLIMIT_FSIZE, (0, 0))
    resource.setrlimit(resource.RLIMIT_CPU, (CPU_LIMIT_SECONDS, CPU_LIMIT_SECONDS))
    try:
        resource.setrlimit(
            resource.RLIMIT_AS, (ADDRESS_SPACE_LIMIT_BYTES, ADDRESS_SPACE_LIMIT_BYTES)
        )
    except ValueError:
        # Darwin exposes RLIMIT_AS as an unlimited sentinel that cannot be
        # lowered after the interpreter has started. Keep the CPU, file-size,
        # and audit-hook guards active; Linux and other platforms remain strict.
        if sys.platform != "darwin":
            raise
    resolved = frozenset(sys.modules)

    def audit(event: str, args: tuple) -> None:
        if event == "import":
            module = args[0]
            if module not in resolved:
                raise PermissionError(
                    f"the submitted source may not import '{module}'; only the "
                    "authoring frontend and the modules it already resolved are "
                    "reachable"
                )
            return
        if event in DENIED_EVENTS or event.startswith(DENIED_EVENT_PREFIXES):
            raise PermissionError(
                f"the submitted source may not reach '{event}'; source capture "
                "reads typed authoring declarations and control flow only"
            )

    sys.addaudithook(audit)


def _bind(entrypoint: str, source: str) -> object:
    """Compile and bind the submitted text from memory, never from disk."""
    try:
        code = compile(source, SOURCE_FILE_NAME, "exec", dont_inherit=True)
    except BaseException as error:  # noqa: BLE001 - a compile failure is a source rejection
        raise Rejected(REASON_SOURCE, f"{type(error).__name__}: {error}") from None

    class InMemoryLoader(importlib.abc.InspectLoader):
        """Binds the submitted text without giving it a path on the filesystem."""

        def get_source(self, fullname: str) -> str:
            return source

        def get_code(self, fullname: str):
            return code

        def exec_module(self, module) -> None:
            exec(code, module.__dict__)  # noqa: S102 - the frontend requires evaluation

    # The frontend reads the authored callback back through `inspect`, which
    # resolves source text through `linecache`. Registering the submitted text
    # there is what lets a memory-only module be read statically.
    linecache.cache[SOURCE_FILE_NAME] = (
        len(source),
        None,
        source.splitlines(keepends=True),
        SOURCE_FILE_NAME,
    )

    spec = importlib.util.spec_from_loader(
        MODULE_NAME, InMemoryLoader(), origin=SOURCE_FILE_NAME
    )
    module = importlib.util.module_from_spec(spec)
    module.__file__ = SOURCE_FILE_NAME
    sys.modules[MODULE_NAME] = module

    _lock_down()

    try:
        spec.loader.exec_module(module)
    except BaseException as error:  # noqa: BLE001 - any authoring failure rejects
        raise Rejected(REASON_SOURCE, f"{type(error).__name__}: {error}") from None

    if not hasattr(module, entrypoint):
        raise Rejected(
            REASON_ENTRYPOINT,
            f"entrypoint '{entrypoint}' is not defined by the submitted source",
        )
    return getattr(module, entrypoint)


def _capture(entrypoint: str, definition: object) -> object:
    graph = getattr(definition, "frontend_graph", None)
    if not callable(graph):
        raise Rejected(
            REASON_ENTRYPOINT,
            f"entrypoint '{entrypoint}' is not an authored Agent program",
        )
    try:
        return graph()
    except BaseException as error:  # noqa: BLE001 - any capture failure rejects
        raise Rejected(REASON_SOURCE, f"{type(error).__name__}: {error}") from None


def main() -> int:
    try:
        frontend_root, entrypoint, source = _read_request()
        _load_frontend(frontend_root)
        captured = _capture(entrypoint, _bind(entrypoint, source))
    except Rejected as rejection:
        print(f"{rejection.reason}\n{rejection.detail}", file=sys.stderr)
        return 1
    json.dump({"frontend_graph": captured}, sys.stdout, sort_keys=True)
    return 0


raise SystemExit(main())
