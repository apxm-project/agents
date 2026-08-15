#!/usr/bin/env python3
"""`dekk agents check-frontend-surface` — frontend-surface conformance gate.

The surface manifest, `contracts/vectors/apxm.frontend-surface.json`, states
every public authoring declaration once: its concept, the semantic node it
binds, the arguments it accepts, and — for each registered language — where that
language projects it and what form each argument takes there.

This gate proves the manifest true against the real sources. It does not compare
identifier names: for every declaration and every registered language it
extracts the projection's *argument shape* from that language's own source and
requires it to be the shape the manifest declares, argument by argument, in the
form the manifest says that language projects it, with the same required-ness.

Two failures are named, because they are the two ways the surface stops being
one surface:

* ``FrontendSurfaceIncomplete{language, declaration}`` — a registered language
  has no projection for a declaration, or its projection is missing an argument
  the manifest declares. Every frontend must implement the whole surface; a
  language that implements part of it is not a conforming implementation, and
  that is an error rather than an omission nobody notices.
* ``FrontendSurfaceUnsynced{artifact}`` — a generated artifact has drifted from
  the manifest it is generated from.

Adding a third language is adding one entry to `languages`, one projection per
declaration, and one `SurfaceLanguage` subclass below. Everything else — the
declarations, the arguments, the diagnostics, the example scan — is inherited.

The sample scan reads every authoring document — `examples/`, `docs/`, and both
frontend READMEs — because a guide that teaches a name the surface does not
publish is wrong in exactly the way an example that imports one is, and the
guides are the copy authors actually copy. It reads them in both directions: a
document that *denies* a published name is wrong the same way and nothing
positive can see it, because such a document misspells nothing and imports
nothing — it just tells the reader a shipped marker does not exist.

A document that *quotes* a name rather than teaching it — the marker's own
rejection message, or an accepted ADR recording the signature it decided before
a later ADR superseded it — says so with `<!-- frontend-surface:quoted NAME
reason -->`. See `quoted_exemptions` for why that is a per-name marker and not a
path exclusion.
"""

from __future__ import annotations

import ast
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional


REPO_ROOT = Path(__file__).resolve().parents[2]
MANIFEST = REPO_ROOT / "contracts" / "vectors" / "apxm.frontend-surface.json"
SCHEMA = REPO_ROOT / "contracts" / "schemas" / "apxm.frontend-surface.json"
def authoring_samples() -> tuple[Path, ...]:
    """Every document that teaches the authoring surface.

    `node_modules` is excluded because a vendored package's README is not this
    repository's teaching material and nothing here can fix one.
    """
    documents = [
        path
        for root in ("examples", "docs")
        for path in sorted((REPO_ROOT / root).glob("**/*.md"))
        if "node_modules" not in path.parts
    ]
    frontends = REPO_ROOT / "crates" / "compiler" / "frontend"
    documents.append(frontends / "python" / "README.md")
    documents.append(frontends / "typescript" / "README.md")
    return tuple(documents)


AUTHORING_SAMPLES = authoring_samples()

#: `<!-- frontend-surface:quoted NAME reason -->` — one name this document
#: quotes rather than teaches, and why.
QUOTED = re.compile(r"<!--\s*frontend-surface:quoted\s+(\S+)\s+(.+?)\s*-->", re.S)


class SurfaceFailure(Exception):
    """One reported surface failure, rendered as its own closed reason."""

    def __str__(self) -> str:  # pragma: no cover - formatting only
        return self.args[0]


def incomplete(language: str, declaration: str, detail: str) -> str:
    """Render the failure a language that does not implement the surface raises."""
    return f"FrontendSurfaceIncomplete{{language={language}, declaration={declaration!r}}}: {detail}"


def unsynced(artifact: str, detail: str) -> str:
    """Render the failure a generated artifact that has drifted raises."""
    return f"FrontendSurfaceUnsynced{{artifact={artifact}}}: {detail}"


def canonical(name: str) -> str:
    """Fold one argument spelling onto the manifest's argument identity.

    `capabilityRef`, `capability_ref`, and `CapabilityRef` are one argument. A
    leading underscore marks an unused parameter, not a different argument.
    """
    stripped = name.lstrip("_")
    return re.sub(r"(?<!^)(?=[A-Z])", "_", stripped).lower()


@dataclass(frozen=True)
class Argument:
    """One accepted argument in one language, with its required-ness."""

    name: str
    required: bool


@dataclass
class ProjectionShape:
    """The argument shape one language's projection of a declaration accepts."""

    type_parameters: list[Argument] = field(default_factory=list)
    parameters: list[Argument] = field(default_factory=list)
    keywords: list[Argument] = field(default_factory=list)
    option_fields: list[Argument] = field(default_factory=list)
    members: set[str] = field(default_factory=set)
    returns_decorator: bool = False
    context_manager: bool = False

    def by_kind(self, kind: str) -> Optional[list[Argument]]:
        return {
            "type_parameter": self.type_parameters,
            "parameter": self.parameters,
            "keyword": self.keywords,
            "option_field": self.option_fields,
        }.get(kind)

    def accepted_names(self) -> set[str]:
        """Every argument name this projection accepts, in any form."""
        return {
            argument.name
            for group in (
                self.type_parameters,
                self.parameters,
                self.keywords,
                self.option_fields,
            )
            for argument in group
        }


class SurfaceLanguage:
    """One registered language's local extraction layer.

    A subclass answers four questions about its own sources. Everything the gate
    decides from the answers — conformance, completeness, drift — is shared.
    """

    id = ""

    #: The generated capability catalogue this language's authors import ids
    #: from. It is not a manifest field: the manifest states the *surface*, and
    #: the catalogue is a generated vocabulary the surface refers to, so where
    #: it lands is a fact about this language's package layout and belongs in
    #: this language's own extraction layer.
    generated_capabilities = ""

    def __init__(self, registration: dict) -> None:
        self.registration = registration
        self.authoring_root = REPO_ROOT / registration["authoring_root"]
        self.generated_diagnostics = REPO_ROOT / registration["generated_diagnostics"]
        self.capability_catalogue = REPO_ROOT / self.generated_capabilities

    def root_exports(self) -> set[str]:
        """The names the authoring root publishes."""
        raise NotImplementedError

    def root_bound_names(self) -> set[str]:
        """Every name bound in the authoring root's namespace."""
        raise NotImplementedError

    def shape(self, projection: dict) -> Optional[ProjectionShape]:
        """The argument shape of one projection, or None when it is absent."""
        raise NotImplementedError

    def diagnostic_codes(self) -> list[str]:
        """The diagnostic codes the generated module projects, in order."""
        raise NotImplementedError

    def import_patterns(self) -> tuple[str, ...]:
        """Regexes matching an authoring import in documentation and examples."""
        return ()

    def catalogue_import_patterns(self) -> tuple[str, ...]:
        """Regexes matching a capability-catalogue import in documentation."""
        return ()

    def catalogue_symbols(self) -> set[str]:
        """The names the generated capability catalogue publishes."""
        raise NotImplementedError

    def catalogue_ids(self) -> set[str]:
        """The capability ids the generated catalogue mints."""
        raise NotImplementedError

    def vocabulary_drift(self, projection: dict) -> Optional[str]:
        """Why the declared import path does not reach the declared module.

        The manifest states the path an author writes, so checking that the file
        exists proves nothing on its own: the path is the promise, and a module
        the package does not publish under it is unreachable however present it
        is on disk. Returns None when the projection holds.
        """
        raise NotImplementedError


# ---------------------------------------------------------------------------
# Python
# ---------------------------------------------------------------------------


class PythonLanguage(SurfaceLanguage):
    """Reads argument shape out of the Python frontend with `ast`."""

    id = "python"
    generated_capabilities = (
        "crates/compiler/frontend/python/apxm_program/_generated/capabilities.py"
    )

    @staticmethod
    def _module(path: Path) -> ast.Module:
        return ast.parse(path.read_text(encoding="utf-8"), filename=str(path))

    def _exported_names(self, path: Path) -> set[str]:
        exported: set[str] = set()
        for statement in self._module(path).body:
            if isinstance(statement, ast.Assign):
                for target in statement.targets:
                    if (
                        isinstance(target, ast.Name)
                        and target.id == "__all__"
                        and isinstance(statement.value, (ast.List, ast.Tuple))
                    ):
                        exported.update(
                            element.value
                            for element in statement.value.elts
                            if isinstance(element, ast.Constant)
                            and isinstance(element.value, str)
                        )
        return exported

    def root_exports(self) -> set[str]:
        return self._exported_names(self.authoring_root)

    def catalogue_symbols(self) -> set[str]:
        return self._exported_names(self.capability_catalogue)

    def catalogue_ids(self) -> set[str]:
        text = self.capability_catalogue.read_text(encoding="utf-8")
        block = re.search(r"CapabilityId: TypeAlias = Literal\[(.*?)\]", text, re.S)
        return set(re.findall(r'"([^"]+)"', block.group(1))) if block else set()

    def catalogue_import_patterns(self) -> tuple[str, ...]:
        return (r"from\s+apxm_program\.capabilities\s+import\s+([^\n]+)",)

    def vocabulary_drift(self, projection: dict) -> Optional[str]:
        # A Python import path *is* a file path, so the manifest states one claim
        # twice and either spelling can be the one that drifted.
        package = self.authoring_root.parent
        module = package.parent.joinpath(*projection["import_path"].split("."))
        declared = REPO_ROOT / projection["module"]
        if module.with_suffix(".py") != declared:
            return (
                f"declares import path {projection['import_path']!r}, which names "
                f"{module.with_suffix('.py').relative_to(REPO_ROOT)} rather than the "
                f"declared {projection['module']}"
            )
        return None

    def root_bound_names(self) -> set[str]:
        bound: set[str] = set()
        for statement in self._module(self.authoring_root).body:
            if isinstance(statement, (ast.Import, ast.ImportFrom)):
                bound.update(alias.asname or alias.name for alias in statement.names)
            elif isinstance(
                statement, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)
            ):
                bound.add(statement.name)
            elif isinstance(statement, ast.Assign):
                bound.update(
                    target.id
                    for target in statement.targets
                    if isinstance(target, ast.Name)
                )
        return bound - {"__all__", "annotations"}

    def shape(self, projection: dict) -> Optional[ProjectionShape]:
        module = self._module(REPO_ROOT / projection["module"])
        classes = {
            node.name: node for node in module.body if isinstance(node, ast.ClassDef)
        }
        functions = {
            node.name: node
            for node in module.body
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        }

        symbol = projection["symbol"]
        shape = ProjectionShape()
        declaration: ast.ClassDef | None = None

        if symbol in functions:
            callable_node = functions[symbol]
        elif symbol in classes:
            declaration = classes[symbol]
        else:
            instance = self._factory_class(module, symbol)
            declaration = classes.get(instance or "")
        if declaration is not None:
            callable_node = self._describe_class(declaration, shape, projection)
            # A class-shaped projection such as `TaskGroup` carries no callable
            # signature of its own; its class-level facts are the whole shape.
            if callable_node is None:
                return shape
        elif symbol not in functions:
            return None

        self._read_signature(callable_node, shape, module, classes)
        shape.returns_decorator = self._returns_decorator(callable_node, classes)
        return shape

    @staticmethod
    def _factory_class(module: ast.Module, symbol: str) -> Optional[str]:
        """Resolve `Model = _ModelFactory()` onto the class that implements it."""
        for statement in module.body:
            if not isinstance(statement, ast.Assign):
                continue
            for target in statement.targets:
                if (
                    isinstance(target, ast.Name)
                    and target.id == symbol
                    and isinstance(statement.value, ast.Call)
                    and isinstance(statement.value.func, ast.Name)
                ):
                    return statement.value.func.id
        return None

    def _describe_class(
        self, node: ast.ClassDef, shape: ProjectionShape, projection: dict
    ) -> Optional[ast.AST]:
        """Fill in the class-level facts and return the node carrying arguments."""
        methods = {
            child.name: child
            for child in node.body
            if isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef))
        }
        shape.members = {name for name in methods if not name.startswith("__")}
        shape.context_manager = "__aenter__" in methods and "__aexit__" in methods
        for child in node.body:
            if isinstance(child, ast.Assign):
                for target in child.targets:
                    if (
                        isinstance(target, ast.Name)
                        and target.id == "_type_parameters"
                        and isinstance(child.value, (ast.Tuple, ast.List))
                    ):
                        shape.type_parameters = [
                            Argument(canonical(element.value), True)
                            for element in child.value.elts
                            if isinstance(element, ast.Constant)
                        ]
        member = projection.get("member")
        if member is not None:
            return methods.get(member)
        return methods.get("__call__")

    def _read_signature(
        self,
        node: ast.FunctionDef | ast.AsyncFunctionDef,
        shape: ProjectionShape,
        module: ast.Module,
        classes: dict[str, ast.ClassDef],
    ) -> None:
        args = node.args
        positional = list(args.posonlyargs) + list(args.args)
        if positional and positional[0].arg in {"self", "cls"}:
            positional = positional[1:]
        defaults = list(args.defaults)
        required_positional = len(positional) - len(defaults)
        for index, argument in enumerate(positional):
            expanded = self._expand_typed_dict(argument, module, classes)
            if expanded is not None:
                shape.option_fields.extend(expanded)
                continue
            shape.parameters.append(
                Argument(canonical(argument.arg), index < required_positional)
            )
        for argument, default in zip(args.kwonlyargs, args.kw_defaults):
            shape.keywords.append(Argument(canonical(argument.arg), default is None))

    @staticmethod
    def _expand_typed_dict(
        argument: ast.arg, module: ast.Module, classes: dict[str, ast.ClassDef]
    ) -> Optional[list[Argument]]:
        """Expand a parameter annotated with a local TypedDict into its fields.

        A definition object is one argument in the source and many in the
        contract; the fields are what the manifest declares, so they are what
        this gate compares.
        """
        annotation = argument.annotation
        if not isinstance(annotation, ast.Name) or annotation.id not in classes:
            return None
        declaration = classes[annotation.id]
        if not any(
            isinstance(base, ast.Name) and base.id == "TypedDict"
            for base in declaration.bases
        ):
            return None
        fields: list[Argument] = []
        for child in declaration.body:
            if isinstance(child, ast.AnnAssign) and isinstance(child.target, ast.Name):
                required = not (
                    isinstance(child.annotation, ast.Subscript)
                    and isinstance(child.annotation.value, ast.Name)
                    and child.annotation.value.id == "NotRequired"
                )
                fields.append(Argument(canonical(child.target.id), required))
        return fields

    @staticmethod
    def _returns_decorator(
        node: ast.FunctionDef | ast.AsyncFunctionDef, classes: dict[str, ast.ClassDef]
    ) -> bool:
        """Whether calling this projection yields something applied to a `def`."""
        returns = node.returns
        if returns is None:
            return False
        if isinstance(returns, ast.Subscript) and isinstance(returns.value, ast.Name):
            return returns.value.id == "Callable"
        name = returns.id if isinstance(returns, ast.Name) else None
        if name is None and isinstance(returns, ast.Constant):
            name = returns.value if isinstance(returns.value, str) else None
        declaration = classes.get(name or "")
        return declaration is not None and any(
            isinstance(child, (ast.FunctionDef, ast.AsyncFunctionDef))
            and child.name == "__call__"
            for child in declaration.body
        )

    def diagnostic_codes(self) -> list[str]:
        text = self.generated_diagnostics.read_text(encoding="utf-8")
        block = re.search(r"DiagnosticCode: TypeAlias = Literal\[(.*?)\]", text, re.S)
        if block is None:
            return []
        return re.findall(r'"([^"]+)"', block.group(1))

    def import_patterns(self) -> tuple[str, ...]:
        return (r"from\s+apxm_program\s+import\s+([^\n]+)",)


# ---------------------------------------------------------------------------
# TypeScript
# ---------------------------------------------------------------------------


def split_top_level(text: str, separator: str = ",") -> list[str]:
    """Split on a separator that is not nested inside brackets or a string."""
    parts: list[str] = []
    depth = 0
    quote: str | None = None
    current: list[str] = []
    for character in text:
        if quote is not None:
            current.append(character)
            if character == quote:
                quote = None
            continue
        if character in "\"'`":
            quote = character
            current.append(character)
            continue
        if character in "([{<":
            depth += 1
        elif character in ")]}>":
            depth -= 1
        if character == separator and depth == 0:
            parts.append("".join(current))
            current = []
            continue
        current.append(character)
    tail = "".join(current)
    if tail.strip():
        parts.append(tail)
    return [part for part in parts if part.strip()]


def split_default(entry: str) -> tuple[str, str]:
    """Split a parameter on its default, which `=>` in a function type is not."""
    depth = 0
    for index, character in enumerate(entry):
        if character in "([{<":
            depth += 1
        elif character in ")]}>":
            depth -= 1
        elif character == "=" and depth == 0:
            following = entry[index + 1 : index + 2]
            previous = entry[index - 1 : index]
            if following not in {">", "="} and previous not in {"=", "!", "<", ">"}:
                return entry[:index], entry[index + 1 :]
    return entry, ""


def matching_bracket(text: str, start: int) -> int:
    """Index just past the bracket group opened at `start`."""
    openers = "([{<"
    closers = ")]}>"
    pair = dict(zip(openers, closers))
    opener = text[start]
    closer = pair[opener]
    depth = 0
    quote: str | None = None
    for index in range(start, len(text)):
        character = text[index]
        if quote is not None:
            if character == quote:
                quote = None
            continue
        if character in "\"'`":
            quote = character
            continue
        if character == opener:
            depth += 1
        elif character == closer:
            depth -= 1
            if depth == 0:
                return index + 1
    raise SurfaceFailure(f"unbalanced {opener!r} in a TypeScript declaration")


class TypeScriptLanguage(SurfaceLanguage):
    """Reads argument shape out of the TypeScript frontend's declarations."""

    id = "typescript"
    generated_capabilities = (
        "crates/compiler/frontend/typescript/src/generated/capabilities.ts"
    )

    @staticmethod
    def _strip_comments(text: str) -> str:
        text = re.sub(r"/\*.*?\*/", "", text, flags=re.S)
        return re.sub(r"^\s*//.*$", "", text, flags=re.M)

    def _source(self, path: Path) -> str:
        return self._strip_comments(path.read_text(encoding="utf-8"))

    def root_exports(self) -> set[str]:
        text = self._source(self.authoring_root)
        names: set[str] = set()
        for group in re.findall(r"export\s*\{(.*?)\}\s*from", text, flags=re.S):
            for entry in split_top_level(group):
                name = re.sub(r"^\s*type\s+", "", entry.strip())
                if name:
                    names.add(name.split(" as ")[-1].strip())
        return names

    def root_bound_names(self) -> set[str]:
        return self.root_exports()

    def vocabulary_drift(self, projection: dict) -> Optional[str]:
        # Unlike Python, a TypeScript import path is not a file path: the package
        # decides which subpaths exist, so a module can be checked in and still
        # be unreachable by the path the manifest promises.
        package_json = self.authoring_root.parent.parent / "package.json"
        exports = json.loads(package_json.read_text(encoding="utf-8")).get("exports", {})
        scope = "@apxm/frontend"
        path = projection["import_path"]
        subpath = "." if path == scope else f".{path[len(scope):]}"
        if not path.startswith(scope) or subpath not in exports:
            return (
                f"declares import path {path!r}, which "
                f"{package_json.relative_to(REPO_ROOT)} does not publish; its "
                f"subpaths are {sorted(exports)}"
            )
        return None

    def shape(self, projection: dict) -> Optional[ProjectionShape]:
        text = self._source(REPO_ROOT / projection["module"])
        symbol = projection["symbol"]
        member = projection.get("member")
        shape = ProjectionShape()

        body = self._object_body(text, symbol)
        if body is not None:
            shape.members = set(re.findall(r"^\s*(\w+)\s*[<(]", body, flags=re.M))
            if member is None:
                return shape
            signature = self._member_signature(body, member)
        else:
            signature = self._function_signature(text, symbol)
            if signature is not None and member is not None:
                signature = None
        if signature is None:
            return None

        type_parameters, parameters = signature
        shape.type_parameters = [
            Argument(canonical(name), not has_default)
            for name, has_default in type_parameters
        ]
        for name, optional, annotation in parameters:
            expanded = self._expand_object_type(text, annotation)
            if expanded is not None:
                shape.option_fields.extend(expanded)
                continue
            shape.parameters.append(Argument(canonical(name), not optional))
        shape.returns_decorator = False
        return shape

    def _object_body(self, text: str, symbol: str) -> Optional[str]:
        """The body of `export const X = { ... }` or `declare const X: Readonly<{...}>`."""
        for pattern in (
            rf"export\s+(?:declare\s+)?const\s+{re.escape(symbol)}\s*(?::[^=]*?)?=\s*(?:Object\.freeze\()?\s*",
            rf"export\s+declare\s+const\s+{re.escape(symbol)}\s*:\s*Readonly\s*<\s*",
        ):
            match = re.search(pattern, text)
            if match is None:
                continue
            index = text.find("{", match.end() - 1)
            if index == -1:
                continue
            return text[index + 1 : matching_bracket(text, index) - 1]
        return None

    def _function_signature(self, text: str, symbol: str):
        match = re.search(
            rf"export\s+(?:declare\s+)?function\s+{re.escape(symbol)}\s*",
            text,
        )
        if match is None:
            return None
        return self._read_signature(text, match.end())

    def _member_signature(self, body: str, member: str):
        match = re.search(rf"(?:^|[;,{{\s]){re.escape(member)}\s*(?=[<(])", body)
        if match is None:
            return None
        return self._read_signature(body, match.end())

    def _read_signature(self, text: str, cursor: int):
        """Read `<type params>(params)` starting at `cursor`."""
        while cursor < len(text) and text[cursor].isspace():
            cursor += 1
        type_parameters: list[tuple[str, bool]] = []
        if cursor < len(text) and text[cursor] == "<":
            end = matching_bracket(text, cursor)
            for entry in split_top_level(text[cursor + 1 : end - 1]):
                name, _, default = entry.partition("=")
                name = name.split(" extends ")[0].strip()
                if name:
                    type_parameters.append((name, bool(default.strip())))
            cursor = end
        while cursor < len(text) and text[cursor].isspace():
            cursor += 1
        if cursor >= len(text) or text[cursor] != "(":
            return None
        end = matching_bracket(text, cursor)
        parameters: list[tuple[str, bool, str]] = []
        for entry in split_top_level(text[cursor + 1 : end - 1]):
            entry = entry.strip()
            declaration, default = split_default(entry)
            name, _, annotation = declaration.partition(":")
            name = name.strip()
            optional = name.endswith("?") or bool(default.strip())
            parameters.append((name.rstrip("?"), optional, annotation.strip()))
        return type_parameters, parameters

    def _expand_object_type(self, text: str, annotation: str) -> Optional[list[Argument]]:
        """Expand an options object — inline or named in the same module — into fields."""
        annotation = annotation.strip()
        if annotation.startswith("{"):
            return self._object_fields(annotation[1 : matching_bracket(annotation, 0) - 1])
        name = annotation.split("<")[0].strip()
        if not re.fullmatch(r"\w+", name):
            return None
        match = re.search(
            rf"(?:export\s+)?(?:type|interface)\s+{re.escape(name)}\s*", text
        )
        if match is None:
            return None
        cursor = match.end()
        if cursor < len(text) and text[cursor] == "<":
            cursor = matching_bracket(text, cursor)
        while cursor < len(text) and text[cursor] in " \t\r\n=":
            cursor += 1
        if cursor >= len(text) or text[cursor] != "{":
            return None
        return self._object_fields(text[cursor + 1 : matching_bracket(text, cursor) - 1])

    @staticmethod
    def _object_fields(body: str) -> list[Argument]:
        fields: list[Argument] = []
        for entry in split_top_level(body, ";"):
            for member in split_top_level(entry):
                member = member.strip()
                if not member:
                    continue
                match = re.match(r"(?:readonly\s+)?(\w+)(\??)\s*[<(:]", member)
                if match is None:
                    continue
                fields.append(Argument(canonical(match.group(1)), match.group(2) != "?"))
        return fields

    def diagnostic_codes(self) -> list[str]:
        text = self.generated_diagnostics.read_text(encoding="utf-8")
        block = re.search(r"export type DiagnosticCode =(.*?);", text, re.S)
        if block is None:
            return []
        return re.findall(r'"([^"]+)"', block.group(1))

    def import_patterns(self) -> tuple[str, ...]:
        return (r"import\s*\{([^}]+)\}\s*from\s*[\"']@apxm/frontend[\"']",)

    def catalogue_symbols(self) -> set[str]:
        text = self._source(self.capability_catalogue)
        names = set(re.findall(r"^export\s+const\s+(\w+)", text, flags=re.M))
        names.update(re.findall(r"^export\s+type\s+(\w+)", text, flags=re.M))
        return names

    def catalogue_ids(self) -> set[str]:
        text = self._source(self.capability_catalogue)
        block = re.search(r"export type CapabilityId =(.*?);", text, re.S)
        return set(re.findall(r'"([^"]+)"', block.group(1))) if block else set()

    def catalogue_import_patterns(self) -> tuple[str, ...]:
        return (r"import\s*\{([^}]+)\}\s*from\s*[\"']@apxm/frontend/capabilities[\"']",)


LANGUAGE_LAYERS: dict[str, type[SurfaceLanguage]] = {
    PythonLanguage.id: PythonLanguage,
    TypeScriptLanguage.id: TypeScriptLanguage,
}


# ---------------------------------------------------------------------------
# The conformance check
# ---------------------------------------------------------------------------


def check_declaration(
    language: SurfaceLanguage, declaration: dict, failures: list[str]
) -> None:
    """Prove one language projects one declaration with the declared shape."""
    concept = declaration["concept"]
    projection = declaration["projections"].get(language.id)
    if projection is None:
        failures.append(
            incomplete(
                language.id,
                concept,
                "the manifest registers this language but the declaration names no projection for it",
            )
        )
        return

    module = REPO_ROOT / projection["module"]
    if not module.exists():
        failures.append(
            incomplete(
                language.id,
                concept,
                f"the projection names {projection['module']}, which does not exist",
            )
        )
        return

    shape = language.shape(projection)
    if shape is None:
        member = f".{projection['member']}" if "member" in projection else ""
        failures.append(
            incomplete(
                language.id,
                concept,
                f"{projection['module']} declares no `{projection['symbol']}{member}`",
            )
        )
        return

    declared: set[str] = set()
    for argument in declaration["arguments"]:
        name = argument["name"]
        kinds = argument["projected_as"].get(language.id)
        if kinds is None:
            failures.append(
                incomplete(
                    language.id,
                    concept,
                    f"argument {name!r} does not say how this language projects it",
                )
            )
            continue
        declared.add(name)
        for kind in kinds:
            check_argument(language, concept, projection, shape, argument, kind, failures)

    extra = sorted(shape.accepted_names() - declared)
    if extra:
        failures.append(
            f"{language.id} projection of {concept!r} accepts undeclared arguments {extra}; "
            "the manifest is the surface, so an argument the manifest does not state is not one"
        )


def check_argument(
    language: SurfaceLanguage,
    concept: str,
    projection: dict,
    shape: ProjectionShape,
    argument: dict,
    kind: str,
    failures: list[str],
) -> None:
    """Prove one argument takes the form the manifest says it takes here."""
    name = argument["name"]
    required = argument["required"]

    if kind == "inferred":
        if name in shape.accepted_names():
            failures.append(
                f"{language.id} projection of {concept!r} accepts {name!r}, which the "
                "manifest says this language infers rather than accepts"
            )
        return

    if kind == "method_name":
        member = projection.get("member")
        if member is None or member not in shape.members or len(shape.members) < 2:
            failures.append(
                incomplete(
                    language.id,
                    concept,
                    f"{name!r} is projected as a method name, so `{projection['symbol']}` "
                    f"must expose more than one member and include {member!r}; it exposes "
                    f"{sorted(shape.members)}",
                )
            )
        return

    if kind == "context_manager_block":
        if not shape.context_manager:
            failures.append(
                incomplete(
                    language.id,
                    concept,
                    f"{name!r} is projected as a context-manager block, so "
                    f"`{projection['symbol']}` must define __aenter__ and __aexit__",
                )
            )
        return

    if kind == "decorated_function":
        if not shape.returns_decorator:
            failures.append(
                incomplete(
                    language.id,
                    concept,
                    f"{name!r} is projected as a decorated function, so "
                    f"`{projection['symbol']}` must return something applied to a def",
                )
            )
        return

    group = shape.by_kind(kind)
    if group is None:
        failures.append(f"unknown projected_as kind {kind!r} on {concept!r}")
        return
    found = next((entry for entry in group if entry.name == name), None)
    if found is None:
        failures.append(
            incomplete(
                language.id,
                concept,
                f"no {kind} named {name!r}; this projection accepts "
                f"{[entry.name for entry in group]}",
            )
        )
        return
    if found.required != required:
        expected = "required" if required else "optional"
        actual = "required" if found.required else "optional"
        failures.append(
            f"{language.id} projection of {concept!r} makes {kind} {name!r} {actual}, "
            f"but the surface declares it {expected}"
        )


def check_generated_diagnostics(
    language: SurfaceLanguage, manifest: dict, failures: list[str]
) -> None:
    """Prove the generated diagnostic vocabulary still matches the manifest."""
    expected: list[str] = []
    for declaration in manifest["declarations"]:
        for code in declaration["diagnostics"]:
            if code not in expected:
                expected.append(code)
    artifact = language.registration["generated_diagnostics"]
    if not language.generated_diagnostics.exists():
        failures.append(unsynced(artifact, "the generated module is missing"))
        return
    actual = language.diagnostic_codes()
    if actual != expected:
        missing = [code for code in expected if code not in actual]
        stale = [code for code in actual if code not in expected]
        detail = (
            f"missing {missing}, stale {stale}"
            if missing or stale
            else "the codes are ordered differently than the manifest states them"
        )
        failures.append(
            unsynced(artifact, f"{detail}; run `dekk agents codegen-diagnostics`")
        )


def check_schema_alignment(manifest: dict, failures: list[str]) -> None:
    """Prove the manifest still says what its own schema allows it to say."""
    schema = json.loads(SCHEMA.read_text(encoding="utf-8"))
    definitions = schema["$defs"]
    allowed_languages = set(definitions["LanguageId"]["enum"])
    argument_schema = definitions["SurfaceDeclaration"]["properties"]["arguments"][
        "items"
    ]["properties"]
    allowed_accepts = set(argument_schema["accepts"]["enum"])
    allowed_kinds = set(definitions["ProjectedAs"]["items"]["enum"])
    allowed_nodes = set(
        definitions["SurfaceDeclaration"]["properties"]["bound_semantic_node"]["enum"]
    )

    for key in ("everyday", "advanced", "non_public"):
        expected = schema["properties"][key]["const"]
        if manifest[key] != expected:
            failures.append(
                f"manifest {key} differs from the schema's frozen list: "
                f"expected {expected}, got {manifest[key]}"
            )

    for registration in manifest["languages"]:
        if registration["id"] not in allowed_languages:
            failures.append(f"unregistered language id {registration['id']!r}")

    for declaration in manifest["declarations"]:
        if declaration["bound_semantic_node"] not in allowed_nodes:
            failures.append(
                f"{declaration['concept']!r} binds unknown semantic node "
                f"{declaration['bound_semantic_node']!r}"
            )
        for argument in declaration["arguments"]:
            if argument["accepts"] not in allowed_accepts:
                failures.append(
                    f"{declaration['concept']!r} argument {argument['name']!r} accepts "
                    f"unknown kind {argument['accepts']!r}"
                )
            for language_id, kinds in argument["projected_as"].items():
                if language_id not in allowed_languages:
                    failures.append(
                        f"{declaration['concept']!r} argument {argument['name']!r} names "
                        f"unregistered language {language_id!r}"
                    )
                for kind in kinds:
                    if kind not in allowed_kinds:
                        failures.append(
                            f"{declaration['concept']!r} argument {argument['name']!r} is "
                            f"projected as unknown form {kind!r}"
                        )


def check_tiers(manifest: dict, failures: list[str]) -> None:
    """Every projected symbol is a name the surface tiers actually publish."""
    tiered = set(manifest["everyday"]) | set(manifest["advanced"])
    for declaration in manifest["declarations"]:
        for language_id, projection in declaration["projections"].items():
            if projection["symbol"] not in tiered:
                failures.append(
                    f"{language_id} projects {declaration['concept']!r} as "
                    f"{projection['symbol']!r}, which no surface tier names"
                )


def check_vocabularies(
    languages: list[SurfaceLanguage], manifest: dict, failures: list[str]
) -> None:
    """Every registered language projects every generated vocabulary, by path.

    A vocabulary is a closed set generated into both frontends — capability ids,
    permission decisions, hook scopes. The manifest names the import path an
    author writes rather than the generated file behind it, because where a
    language keeps generated code is a language-local fact and the import path
    is the contract. A language missing a projection is the same
    `FrontendSurfaceIncomplete` failure a missing declaration is: the vocabulary
    exists, and one language cannot spell it.

    `exported_from_root` is pinned false for all of them, and `check_roots`
    already refuses a root that binds a non-declaration name, so the two checks
    together keep a generated module importable only by its own path.
    """
    for vocabulary in manifest.get("vocabularies", ()):
        concept = vocabulary["concept"]
        for language in languages:
            projection = vocabulary["projections"].get(language.id)
            if projection is None:
                failures.append(
                    incomplete(
                        language.id,
                        concept,
                        f"the surface declares the vocabulary, generated by "
                        f"{vocabulary['generated_by']!r}, and this language projects none",
                    )
                )
                continue
            module = REPO_ROOT / projection["module"]
            if not module.is_file():
                failures.append(
                    unsynced(
                        projection["module"],
                        f"{language.id} declares it as the {concept!r} projection, "
                        "but no such module is checked in",
                    )
                )
                continue
            detail = language.vocabulary_drift(projection)
            if detail is not None:
                failures.append(incomplete(language.id, concept, detail))


def check_roots(
    language: SurfaceLanguage, manifest: dict, failures: list[str]
) -> None:
    """The authoring root publishes exactly the declarations that claim it."""
    expected = {
        declaration["projections"][language.id]["symbol"]
        for declaration in manifest["declarations"]
        if language.id in declaration["projections"]
        and declaration["projections"][language.id]["exported_from_root"]
    }
    exports = language.root_exports()
    if exports != expected:
        failures.append(
            f"{language.id} authoring root exports {sorted(exports)}, but the surface "
            f"declares {sorted(expected)}"
        )
    leaked = language.root_bound_names() - expected
    if leaked:
        failures.append(
            f"{language.id} authoring root binds non-surface names {sorted(leaked)}"
        )


#: A `Tool`/`Capability` marker bound to a string literal rather than to an
#: imported catalogue symbol, in either language's spelling.
LITERAL_REFERENCE = re.compile(
    r"\b(?:Tool|Capability)(?:\[[^\]]*\]|<[^>]*>)?\(\s*[\"']([^\"']+)[\"']"
)

#: A Capability a package declares by shipping a handler for it, in the two
#: forms a document writes one: the definition object `Tool.define` (TypeScript)
#: and `capability` (Python) take, and the `capabilities/<id>/handler.<ext>`
#: layout in which the directory name *is* the declaration.
HANDLER_DECLARATION = (
    re.compile(
        r"(?:Tool\.define|capability)\s*\(\s*\{[^{}]*?[\"']?\bname[\"']?\s*[:=]\s*"
        r"[\"']([^\"']+)[\"']",
        re.S,
    ),
    re.compile(r"capabilities/([A-Za-z_][\w.\-]*)/handler\.\w+"),
)


def package_handler_ids() -> set[str]:
    """Every Capability id a package in this repository ships a handler for.

    `capabilities/<id>/handler.<ext>` is the whole declaration — the directory
    name is the id — so the tree states these ids with no registry to read.
    Built output is a copy of a declaration rather than a second one, and a
    vendored package's handlers are not this repository's.
    """
    ignored = {"node_modules", "dist", "target", ".apxm"}
    return {
        path.parent.name
        for root in ("examples", "crates", "tools")
        for path in (REPO_ROOT / root).glob("**/capabilities/*/handler.*")
        if not ignored & set(path.parts)
    }


def fold_reference(value: str) -> str:
    """Fold one written capability reference onto the id it is trying to name.

    A display name (`search-web`), a dotted-namespace spelling
    (`capability.search_web`), and the id itself all fold together, so a
    reference that *means* a catalogue id but is not spelled as one is
    recognisable as the near-miss it is rather than passing for an id of some
    package's own.
    """
    folded = value.strip().lower().replace("-", "_").replace(".", "_")
    prefix = "capability_"
    return folded[len(prefix) :] if folded.startswith(prefix) else folded


def quoted_exemptions(text: str, relative: Path, failures: list[str]) -> dict[str, str]:
    """The names this document declares it quotes rather than teaches.

    The exemption is per name and carries its reason, rather than being a path
    exclusion, because the two documents that need one here are not one kind of
    file. `docs/agents/first-agent.md` is a current-pattern tutorial that quotes
    the display name the marker *refuses*, and an accepted ADR is a frozen
    record of a signature a later ADR superseded — `docs/adr/` would exclude the
    second and miss the first, while excluding thirteen ADRs that are checkable
    and should stay checked. Naming the exemption also keeps the rest of the
    document under the gate, which a whole-file exclusion would not.

    A declared name that nothing in the document would have failed on is itself
    a failure: an exemption list is a generated artifact's twin and rots the
    same way.
    """
    declared: dict[str, str] = {}
    for match in QUOTED.finditer(text):
        name, reason = match.group(1), " ".join(match.group(2).split())
        if not reason:
            failures.append(
                f"{relative} exempts {name!r} without saying why it is quoted "
                "rather than taught"
            )
        declared[name] = reason
    return declared


#: The nouns that make a negated sentence a claim about the *surface* rather
#: than about behaviour. "`Tool` does not execute a runtime tool" is a true
#: statement about what the marker does; "there is no `Tool` marker" is a claim
#: about what the surface publishes, and only the second can contradict the
#: manifest.
SURFACE_NOUN = (
    r"(?:markers?|declarations?|symbols?|exports?|bindings?|forms?|concepts?"
    r"|primitives?|nodes?|types?|decorators?|constructors?|entry\s+points?)"
)

#: The ways a document says a published name is not published. Each names the
#: symbol and denies its existence, declarability, or publication — never its
#: behaviour, which is the surface's business to describe and not this gate's.
DENIAL_TEMPLATES = (
    r"there\s+(?:is|are)\s+no\s+{S}\s+" + SURFACE_NOUN + r"\b",
    r"there\s+(?:is|are)\s+no\s+{S}\s*(?=[,.;:)]|$)",
    r"\bno\s+{S}\s+" + SURFACE_NOUN + r"\b",
    r"\b(?:neither|none\s+of|no)\s+(?:\w+\s+){0,2}?"
    r"(?:frontend|language|package|README|guide|catalogue|manifest)s?\s+"
    r"(?:\w+\s+){0,3}?"
    r"(?:declares?|exports?|publishes?|projects?|has|have|implements?|mints?"
    r"|names?|defines?)\s+(?:an?\s+|the\s+)?{S}\b",
    r"\b(?:does\s+not|doesn't|do\s+not|don't|never)\s+"
    r"(?:publish|export|declare|project|mint|expose|include|name|define)s?\s+"
    r"(?:an?\s+|the\s+)?{S}\b",
    r"{S}\s+is\s+not\s+(?:declarable|authorable|exported|published|declared"
    r"|projected|minted|available|a\s+(?:published|declared|surface|manifest)\b"
    r"|(?:in|part\s+of)\s+the\s+surface)",
    r"{S}\s+(?:does\s+not|doesn't)\s+exist\b",
    r"{S}\s+(?:is|was)\s+(?:not|never)\s+(?:an?\s+)?" + SURFACE_NOUN + r"\b",
)

#: What the denial has to be *about* for this gate to own it. A sentence that
#: denies a name somewhere other than the authoring surface — in a package's
#: topology, in a runtime, in some other product — is not this gate's to judge,
#: and requiring the scope keeps the denial scan quiet enough to stay on.
SURFACE_SCOPE = re.compile(
    r"surface|manifest|frontend|package|catalogue|authoring\s+root|apxm_program"
    r"|@apxm/frontend|either\s+language|both\s+languages|public\s+api",
    re.I,
)


def prose(text: str) -> str:
    """The document with its fenced code blocks blanked out.

    A denial is prose. Code in a sample is held to the surface by the import and
    reference checks, which read what it *does*; running the sentence patterns
    over it would only find English in comments.
    """
    lines: list[str] = []
    fenced = False
    for line in text.split("\n"):
        if line.lstrip().startswith("```"):
            fenced = not fenced
            lines.append("")
            continue
        lines.append("" if fenced else line)
    return "\n".join(lines)


def sentence_around(text: str, start: int, end: int) -> str:
    """The sentence a match sits in, for quoting back in the failure."""
    opening = max(text.rfind(". ", 0, start), text.rfind("\n\n", 0, start))
    closing = text.find(". ", end)
    closing = len(text) if closing == -1 else closing + 1
    return " ".join(text[opening + 1 : closing].split())


def denials(text: str, published: set[str]) -> list[tuple[str, str]]:
    """Every sentence in this document that denies a name the manifest publishes.

    Returned as `(name, sentence)`, at most one per name: a document denies a
    name once as far as the reader is concerned, and the exemption that answers
    the denial is per name too.
    """
    found: dict[str, str] = {}
    for name in sorted(published):
        symbol = r"`?" + re.escape(name) + r"`?"
        for template in DENIAL_TEMPLATES:
            for match in re.finditer(
                template.replace("{S}", symbol), text, flags=re.I
            ):
                sentence = sentence_around(text, match.start(), match.end())
                if SURFACE_SCOPE.search(sentence):
                    found.setdefault(name, sentence)
    return sorted(found.items())


def check_samples(
    languages: list[SurfaceLanguage], allowed: set[str], failures: list[str]
) -> None:
    """No authoring document teaches a name the surface does not publish, and
    none denies one it does.

    A document stops being true in four ways. It imports from the retired
    package. It imports a name no frontend publishes. It binds a capability
    reference that resolves to nothing — neither a minted catalogue id nor a
    Capability some package ships a handler for — which is what turned the
    guides stale before: an unresolvable id looks like authoring code and
    compiles into nothing, whether it is a near-miss of a real id or an
    invention that misses everything.

    And it denies a name the manifest publishes. That failure runs the other
    way from the first three and no positive check can see it: nothing is
    imported and nothing is misspelled, the document simply tells the reader a
    published marker does not exist. This branch shipped exactly that about
    `Skill` in two documents while both frontends exported it. A document with
    standing to record a position a later record superseded says so with the
    same `<!-- frontend-surface:quoted NAME reason -->` marker that answers the
    other three, because amending by a later record is the convention here and
    the gate must not make the amended record unwritable.
    """
    surface_patterns = tuple(
        pattern for language in languages for pattern in language.import_patterns()
    )
    catalogue_patterns = tuple(
        (pattern, language.catalogue_symbols())
        for language in languages
        for pattern in language.catalogue_import_patterns()
    )
    catalogue_ids = {
        identifier for language in languages for identifier in language.catalogue_ids()
    }
    folded_ids = {fold_reference(identifier): identifier for identifier in catalogue_ids}
    shipped = package_handler_ids()

    for sample in AUTHORING_SAMPLES:
        if not sample.exists():
            continue
        text = sample.read_text(encoding="utf-8")
        relative = sample.relative_to(REPO_ROOT)
        exempt = quoted_exemptions(text, relative, failures)
        used: set[str] = set()

        def report(name: str, detail: str) -> None:
            if name in exempt:
                used.add(name)
                return
            failures.append(f"{relative} {detail}")

        for match in re.finditer(
            r"^\s*(?:from|import)\s+apxm(?:\.|\s|$)", text, flags=re.MULTILINE
        ):
            report(
                "apxm",
                f"imports the retired `apxm` package: {match.group(0).strip()!r}",
            )
        for pattern in surface_patterns:
            for match in re.finditer(pattern, text):
                for name in imported_names(match.group(1)):
                    if name not in allowed:
                        report(name, f"imports non-manifest name {name!r}")
        for pattern, symbols in catalogue_patterns:
            for match in re.finditer(pattern, text):
                for name in imported_names(match.group(1)):
                    if name not in symbols:
                        report(
                            name,
                            f"imports {name!r} from the capability catalogue, which "
                            "the generated catalogue does not publish",
                        )
        declared = shipped | {
            identifier
            for pattern in HANDLER_DECLARATION
            for identifier in pattern.findall(text)
        }
        for match in LITERAL_REFERENCE.finditer(text):
            written = match.group(1)
            minted = folded_ids.get(fold_reference(written))
            if minted is not None and written != minted:
                report(
                    written,
                    f"binds the capability reference {written!r}, which no catalogue "
                    f"mints; the id is {minted!r} and the symbol naming it is what an "
                    "Agent Program imports",
                )
            elif minted is None and written not in declared:
                report(
                    written,
                    f"binds the capability reference {written!r}, which resolves to "
                    "nothing: no catalogue mints it and no package declares a handler "
                    "for it. A reference is a catalogue symbol imported from "
                    "`apxm_program.capabilities` or `@apxm/frontend/capabilities`, or "
                    "the declaration a shipped handler returns",
                )

        for name, sentence in denials(prose(text), allowed):
            report(
                name,
                f"denies {name!r}, which the surface manifest publishes and both "
                f"frontends project: {sentence!r}. A record whose position a later "
                "record superseded keeps it by saying so",
            )

        for name, reason in exempt.items():
            if name not in used:
                failures.append(
                    f"{relative} declares {name!r} quoted ({reason}), but nothing in "
                    "the document quotes it; remove the stale exemption"
                )


def imported_names(clause: str) -> list[str]:
    """The names one import clause binds, with any `as` alias resolved away."""
    return [
        name
        for entry in clause.split(",")
        if (name := entry.strip().split(" as ")[0].strip())
    ]


def check() -> list[str]:
    """Return every surface-conformance failure, in reporting order."""
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    failures: list[str] = []
    check_schema_alignment(manifest, failures)
    check_tiers(manifest, failures)

    languages: list[SurfaceLanguage] = []
    for registration in manifest["languages"]:
        layer = LANGUAGE_LAYERS.get(registration["id"])
        if layer is None:
            failures.append(
                f"the manifest registers language {registration['id']!r}, which has no "
                "extraction layer in this gate"
            )
            continue
        languages.append(layer(registration))

    for language in languages:
        for declaration in manifest["declarations"]:
            check_declaration(language, declaration, failures)
        check_generated_diagnostics(language, manifest, failures)
        check_roots(language, manifest, failures)
    check_vocabularies(languages, manifest, failures)

    allowed = {
        projection["symbol"]
        for declaration in manifest["declarations"]
        for projection in declaration["projections"].values()
        if projection["exported_from_root"]
    }
    check_samples(languages, allowed, failures)
    return failures


def main() -> int:
    """Print a compact surface-conformance report."""
    try:
        failures = check()
    except SurfaceFailure as error:
        print(f"Frontend surface conformance failed:\n  {error}", file=sys.stderr)
        return 1
    if failures:
        print("Frontend surface conformance failed:", file=sys.stderr)
        for failure in failures:
            print(f"  {failure}", file=sys.stderr)
        return 1
    print(
        "OK: every registered frontend projects every surface declaration with the "
        "declared argument shape."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
