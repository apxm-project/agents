"""Capture one submitted Python source text into `apxm.frontend-graph`.

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
  and bound through an in-memory loader. The exact request is sealed in the
  frontend's native bridge before evaluation, so static capture does not trust
  the process-wide mutable `linecache`.
* The interpreter runs isolated, so no ambient `PYTHONPATH`, user site
  directory, or `PYTHON*` environment variable reaches it, and `sys.path` holds
  the standard library plus the one declared frontend package root.
* The frontend package is resolved before the lockdown closes, so the submitted
  text runs against an already-closed module graph: an audit hook rejects
  importing any module that is not already resolved, and rejects process,
  network, dynamic code loading, and filesystem events outright.
* A zero file-size limit, a CPU-time limit, an address-space limit, a small
  descriptor limit, and a zero-process limit bound what the submitted text can
  consume where no audit event exists. The last two limits make resource
  exhaustion and child-process escape fail closed even if an interpreter or
  platform ever omits an audit event.

The submitted text shares this process's standard output, so it can write raw
bytes to that descriptor. It cannot forge a result by doing so: the port decodes
the entire standard output as exactly one JSON document, so any injected byte
makes the whole document unparseable and the port rejects with a diagnostic
instead of returning a graph.
"""

from __future__ import annotations

import ast
import importlib.abc
import importlib.util
import json
import linecache
import resource
import sys
import sysconfig
import types

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
DENIED_EVENTS = (
    "open",
    "builtins.input",
    "builtins.breakpoint",
    # Frame objects expose the harness's active fast locals. On newer Python
    # versions writes through ``frame.f_locals`` can update those locals, so a
    # submitted module could otherwise replace a post-evaluation trusted
    # callable even when module globals are snapshotted.
    "sys._getframe",
)

#: CPU seconds and address-space bytes the submitted text may consume.
CPU_LIMIT_SECONDS = 15
ADDRESS_SPACE_LIMIT_BYTES = 2 * 1024 * 1024 * 1024
OPEN_FILE_LIMIT = 64
CHILD_PROCESS_LIMIT = 0

# Keep the final harness serialization independent of globals the submitted
# module may mutate during evaluation.
_json_dump = json.dump


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


def _source_contract(
    source: str,
) -> tuple[frozenset[str], frozenset[str], dict[tuple[str, str], str]]:
    """Summarize source-owned model refs and callback bindings before eval.

    The summary is computed by the harness, before submitted code can mutate
    frontend modules or marker instances. It is an integrity check on the
    native graph returned after evaluation, not a replacement for frontend
    capture: every graph model declaration and call must still be accounted for
    by the original source callback.
    """
    try:
        tree = ast.parse(source)
    except SyntaxError as error:
        raise Rejected(REASON_SOURCE, f"SyntaxError: {error}") from None

    string_names = {
        target.id: value.value
        for statement in tree.body
        if isinstance(statement, ast.Assign)
        and len(statement.targets) == 1
        and isinstance(statement.targets[0], ast.Name)
        and isinstance(value := statement.value, ast.Constant)
        and isinstance(value.value, str)
        for target in (statement.targets[0],)
    }
    marker_names: dict[str, str] = {}
    marker_targets: dict[tuple[str, str], str] = {}
    model_targets: set[str] = set()
    for statement in tree.body:
        if not isinstance(statement, (ast.Assign, ast.AnnAssign)):
            continue
        value = statement.value
        if not isinstance(value, ast.Call) or not isinstance(value.func, ast.Subscript):
            continue
        marker = value.func.value
        if not isinstance(marker, ast.Name) or marker.id not in {
            "Model",
            "Tool",
            "Capability",
            "Event",
        }:
            continue
        targets = statement.targets if isinstance(statement, ast.Assign) else [statement.target]
        if len(targets) != 1 or not isinstance(targets[0], ast.Name):
            continue
        name = targets[0].id
        kind = marker.id.lower()
        marker_names[name] = kind
        if value.args:
            target = value.args[0]
            target_ref = None
            if isinstance(target, ast.Constant) and isinstance(target.value, str):
                target_ref = target.value
            elif isinstance(target, ast.Name):
                target_ref = string_names.get(target.id)
            if target_ref is not None:
                marker_targets[(kind, name)] = target_ref
                if kind == "model":
                    model_targets.add(target_ref)

    callback_bindings: set[str] = set()
    # Agent Programs may declare Hook callbacks alongside the entrypoint. All
    # authored async functions are part of the submitted source contract, so
    # account for marker calls in each one rather than treating the entrypoint
    # as the only callback body.
    callbacks = [node for node in ast.walk(tree) if isinstance(node, ast.AsyncFunctionDef)]
    if callbacks:
        for callback in callbacks:
            for node in ast.walk(callback):
                if not isinstance(node, ast.Call):
                    continue
                if isinstance(node.func, ast.Name) and node.func.id in marker_names:
                    callback_bindings.add(f"decl.{marker_names[node.func.id]}.{node.func.id}")
                elif isinstance(node.func, ast.Attribute) and isinstance(node.func.value, ast.Name):
                    name = node.func.value.id
                    if name in marker_names:
                        callback_bindings.add(f"decl.{marker_names[name]}.{name}")
    return frozenset(model_targets), frozenset(callback_bindings), marker_targets


def _validate_graph_provenance(
    graph: object,
    source_contract: tuple[frozenset[str], frozenset[str], dict[tuple[str, str], str]],
) -> None:
    """Reject graph declarations/calls absent from the caller's source."""
    if not isinstance(graph, dict):
        raise Rejected(REASON_SOURCE, "the Python frontend returned a non-object graph")
    model_targets, callback_bindings, marker_targets = source_contract
    declarations = graph.get("declarations")
    if not isinstance(declarations, list):
        raise Rejected(REASON_SOURCE, "the Python frontend graph has no declarations list")
    for declaration in declarations:
        if not isinstance(declaration, dict):
            raise Rejected(REASON_SOURCE, "the Python frontend graph has an invalid declaration")
        if declaration.get("decl_kind") == "model_binding":
            target = declaration.get("target_ref")
            if not isinstance(target, str) or target not in model_targets:
                raise Rejected(
                    REASON_SOURCE,
                    "the Python frontend graph contains a model target absent from the submitted source",
                )
        target = declaration.get("target_ref")
        decl_id = declaration.get("decl_id")
        if (
            isinstance(target, str)
            and isinstance(decl_id, str)
            and decl_id.startswith("decl.")
        ):
            _, kind, name = decl_id.split(".", 2) if decl_id.count(".") >= 2 else ("", "", "")
            expected = marker_targets.get((kind, name))
            if expected is not None and target != expected:
                raise Rejected(
                    REASON_SOURCE,
                    "the Python frontend graph changed a declared binding target "
                    f"for '{name}' after source capture",
                )

    requirements = graph.get("model_requirements")
    if not isinstance(requirements, list) or any(
        not isinstance(requirement, dict)
        or requirement.get("model_target_ref") not in model_targets
        for requirement in requirements
    ):
        raise Rejected(
            REASON_SOURCE,
            "the Python frontend graph contains an unaccounted model requirement",
        )

    calls = graph.get("call_intents")
    if not isinstance(calls, list):
        raise Rejected(REASON_SOURCE, "the Python frontend graph has no call-intents list")
    if any(
        isinstance(call, dict)
        and isinstance(binding := call.get("binding_ref"), str)
        and binding.startswith("decl.")
        and binding != "decl.capability.read_skill"
        and binding not in callback_bindings
        for call in calls
    ):
        raise Rejected(
            REASON_SOURCE,
            "the Python frontend graph contains a callback binding absent from the submitted source",
        )

    requirements = graph.get("capability_requirements")
    if not isinstance(requirements, list):
        raise Rejected(REASON_SOURCE, "the Python frontend graph has no capability requirements list")
    expected_capability_targets = {
        target
        for (kind, _name), target in marker_targets.items()
        if kind in {"tool", "capability"}
    }
    if expected_capability_targets and any(
        isinstance(requirement, dict)
        and isinstance(target := requirement.get("capability_ref"), str)
        and target not in expected_capability_targets
        and target != "read_skill"
        for requirement in requirements
    ):
        raise Rejected(
            REASON_SOURCE,
            "the Python frontend graph contains a capability requirement absent from the submitted source",
        )


# These modules form the trusted capture path. Submitted code is evaluated in
# the same interpreter, so merely sealing the graph is insufficient if source
# can replace a parser, factory, bridge, or capture helper before the decorator
# runs. The snapshot is deliberately narrow: authoring registries that are
# expected to grow during evaluation (for example declared bindings) are not
# included, while code/module/class identities and callable state are.
_INTEGRITY_MODULES = (
    "apxm_program._agent",
    "apxm_program._bridge",
    "apxm_program._capture",
    "apxm_program._emit",
    "apxm_program._markers",
    "apxm_program._native",
    "ast",
    "inspect",
    "json",
    "linecache",
    "textwrap",
)


def _integrity_value(value: object) -> tuple[object, ...]:
    """Return the identity/state that source must not alter in one value."""
    if isinstance(value, types.FunctionType):
        return (
            id(value),
            id(value.__code__),
            id(value.__defaults__),
            id(value.__kwdefaults__),
            tuple(id(cell.cell_contents) for cell in (value.__closure__ or ())),
        )
    if isinstance(value, types.BuiltinFunctionType):
        return (id(value),)
    if isinstance(value, type):
        members = tuple(
            (name, _integrity_value(member))
            for name, member in vars(value).items()
        )
        return (id(value), members)
    return (id(value),)


def _integrity_snapshot() -> tuple[tuple[str, tuple[tuple[str, tuple[object, ...]], ...]], ...]:
    snapshot = []
    for module_name in _INTEGRITY_MODULES:
        module = sys.modules.get(module_name)
        if module is None:
            continue
        snapshot.append(
            (
                module_name,
                tuple(
                    (name, _integrity_value(value))
                    for name, value in vars(module).items()
                    if not name.startswith("__")
                ),
            )
        )
    return tuple(snapshot)


def _frontend_integrity_guard() -> object:
    """Capture a one-shot post-evaluation integrity check for the frontend."""
    expected = _integrity_snapshot()

    def verify() -> None:
        if _integrity_snapshot() != expected:
            raise Rejected(
                REASON_SOURCE,
                "the Python authoring frontend was modified while capturing source",
            )

    return verify


def _load_frontend(
    frontend_root: str,
) -> tuple[object | None, object | None, object | None]:
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
        for submodule in ("capabilities", "handlers", "permissions", "scopes"):
            try:
                __import__(f"apxm_program.{submodule}")
            except ImportError:
                # A declared root may be a test stand-in that only publishes
                # the package itself. Public frontend submodules are imported
                # when they exist so authored source can name them.
                continue
    except BaseException as error:  # noqa: BLE001 - any resolution failure is unavailability
        raise Rejected(
            REASON_FRONTEND,
            "the Python authoring frontend is not resolvable at the declared "
            f"package root: {type(error).__name__}: {error}",
        ) from None
    try:
        from apxm_program import _native
    except ImportError:
        # The source-port contract tests include a deliberately tiny stand-in
        # frontend. Production APXM packages ship the native bridge below;
        # stand-ins retain the old public method path solely for those tests.
        return None, None, None
    setter = getattr(_native, "set_authored_source", None)
    native_graph = getattr(_native, "frontend_graph", None)
    if not callable(setter) or not callable(native_graph):
        raise Rejected(
            REASON_FRONTEND,
            "the Python authoring frontend native bridge does not provide the "
            "sealed source and graph capture hooks",
        )
    return native_graph, setter, _frontend_integrity_guard()


def _lock_down() -> None:
    """Close the interpreter around the already-resolved frontend."""
    resource.setrlimit(resource.RLIMIT_FSIZE, (0, 0))
    resource.setrlimit(resource.RLIMIT_CPU, (CPU_LIMIT_SECONDS, CPU_LIMIT_SECONDS))
    for limit_name, limit_value in (
        ("RLIMIT_NOFILE", OPEN_FILE_LIMIT),
        ("RLIMIT_NPROC", CHILD_PROCESS_LIMIT),
    ):
        limit = getattr(resource, limit_name, None)
        if limit is None:
            raise Rejected(
                REASON_FRONTEND,
                f"the Python authoring frontend does not expose {limit_name}",
            )
        try:
            resource.setrlimit(limit, (limit_value, limit_value))
        except (OSError, ValueError) as error:
            raise Rejected(
                REASON_FRONTEND,
                f"the Python authoring frontend could not install {limit_name}: {error}",
            ) from None
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

    # Keep this compatibility registration for lightweight frontend stand-ins
    # that still use `inspect`; the production frontend reads the native source
    # handoff sealed before evaluation and does not trust this mutable cache.
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


def _capture(entrypoint: str, definition: object, trusted_graph=None) -> object:
    """Read the graph through the frontend method captured before evaluation."""
    if trusted_graph is None:
        # Lightweight stand-in packages used by contract tests may not expose
        # the production AgentDefinition class. Preserve their public method
        # contract; real packages always take the trusted path below.
        graph = getattr(definition, "frontend_graph", None)
        if not callable(graph):
            raise Rejected(
                REASON_ENTRYPOINT,
                f"entrypoint '{entrypoint}' is not an authored Agent program",
            )
        trusted_graph = lambda _definition: graph()
    try:
        return trusted_graph(definition)
    except KeyError:
        raise Rejected(
            REASON_ENTRYPOINT,
            f"entrypoint '{entrypoint}' is not an authored Agent program",
        ) from None
    except BaseException as error:  # noqa: BLE001 - any capture failure rejects
        raise Rejected(REASON_SOURCE, f"{type(error).__name__}: {error}") from None


def main() -> int:
    # These callables are invoked after the submitted module has executed.
    # Keep their identities in fast locals before evaluation: ``__main__`` is
    # already resolved and therefore importable by submitted source, so a
    # module-global lookup here can otherwise be replaced and restored around
    # the existing frontend-integrity snapshot.
    trusted_capture = _capture
    trusted_validate_graph_provenance = _validate_graph_provenance
    trusted_json_dump = _json_dump
    trusted_stdout = sys.stdout
    trusted_stderr = sys.stderr
    try:
        frontend_root, entrypoint, source = _read_request()
        source_contract = _source_contract(source)
        native_graph, set_authored_source, integrity_guard = _load_frontend(frontend_root)
        if set_authored_source is not None:
            # Seal the exact caller-supplied source before any submitted code
            # runs. The native bridge accepts this only once and keeps it out
            # of Python's reflective globals and linecache.
            set_authored_source(source)
        # Snapshot the canonical method before submitted code can replace the
        # class attribute. The implementation itself reads only the immutable
        # snapshot registry, not a source-dispatched helper.
        trusted_graph = native_graph
        captured = trusted_capture(entrypoint, _bind(entrypoint, source), trusted_graph)
        if native_graph is not None:
            integrity_guard()
            trusted_validate_graph_provenance(captured, source_contract)
    except Rejected as rejection:
        print(f"{rejection.reason}\n{rejection.detail}", file=trusted_stderr)
        return 1
    trusted_json_dump({"frontend_graph": captured}, trusted_stdout, sort_keys=True)
    return 0


raise SystemExit(main())
