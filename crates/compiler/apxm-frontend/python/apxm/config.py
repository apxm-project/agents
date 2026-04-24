from __future__ import annotations

from dataclasses import asdict, dataclass, field
from enum import Enum
import os
from typing import Any

from apxm._generated import constants as graph_keys
from apxm.providers import resolve_provider


def _drop_none(value: Any) -> Any:
    if isinstance(value, dict):
        return {
            k: _drop_none(v)
            for k, v in value.items()
            if v is not None
        }
    if isinstance(value, list):
        return [_drop_none(v) for v in value]
    return value


@dataclass(slots=True)
class BashConfig:
    enabled: bool = True
    blocked_commands: list[str] = field(default_factory=list)
    allowed_commands: list[str] | None = None
    working_directory: str | None = None
    timeout_secs: int = 120
    max_output_bytes: int = 1_000_000


@dataclass(slots=True)
class ReadConfig:
    enabled: bool = True
    blocked_paths: list[str] = field(default_factory=list)
    allowed_paths: list[str] | None = None
    allowed_extensions: list[str] | None = None
    max_file_size: int = 1_000_000
    base_directory: str | None = None
    max_default_lines: int = 2000


@dataclass(slots=True)
class WriteConfig:
    enabled: bool = True
    blocked_paths: list[str] = field(default_factory=list)
    allowed_paths: list[str] | None = None
    allowed_extensions: list[str] | None = None
    blocked_extensions: list[str] = field(default_factory=list)
    create_directories: bool = True
    overwrite_existing: bool = True
    max_file_size: int | None = None
    base_directory: str | None = None


@dataclass(slots=True)
class SearchWebConfig:
    enabled: bool = True
    allowed_domains: list[str] | None = None
    blocked_domains: list[str] = field(default_factory=list)
    blocked_queries: list[str] = field(default_factory=list)
    max_results: int = 5
    safe_search: bool = False
    search_depth: "SearchDepth" = None  # type: ignore[assignment]
    endpoint: str = "https://api.tavily.com/search"
    include_answer: bool = True

    def __post_init__(self) -> None:
        if self.search_depth is None:
            self.search_depth = SearchDepth.BASIC
        elif not isinstance(self.search_depth, SearchDepth):
            raise TypeError("SearchWebConfig.search_depth must be a SearchDepth")


@dataclass(slots=True)
class ToolsConfig:
    bash: BashConfig | None = None
    read: ReadConfig | None = None
    write: WriteConfig | None = None
    search_web: SearchWebConfig | None = None

    def to_dict(self) -> dict[str, Any]:
        return _drop_none(asdict(self))


@dataclass(slots=True)
class NodePolicy:
    """Typed node-level execution defaults lowered to stable graph attrs."""

    model: str | None = None
    provider: str | None = None
    api_key: str | None = None
    base_url: str | None = None
    temperature: float | None = None
    system_prompt: str | None = None
    backend: str | None = None
    tools: list[str] | None = None
    tool_groups: list[str] | None = None
    tools_enabled: bool | None = None
    tools_config: ToolsConfig | None = None
    token_budget: int | None = None
    output_schema: dict[str, Any] | None = None
    max_schema_retries: int | None = None
    max_iterations: int | None = None
    timeout_ms: int | None = None

    def overlay(self, override: "NodePolicy | None") -> "NodePolicy":
        merged: dict[str, Any] = {}
        for field_name in self.__dataclass_fields__:
            if override is None:
                merged[field_name] = getattr(self, field_name)
                continue
            value = getattr(override, field_name)
            merged[field_name] = getattr(self, field_name) if value is None else value
        return NodePolicy(**merged)

    def to_node_attributes(self) -> dict[str, Any]:
        values: dict[str, Any] = {
            graph_keys.MODEL: self.model,
            graph_keys.PROVIDER: self.provider,
            graph_keys.API_KEY: self.api_key,
            graph_keys.BASE_URL: self.base_url,
            graph_keys.TEMPERATURE: self.temperature,
            graph_keys.SYSTEM_PROMPT: self.system_prompt,
            graph_keys.BACKEND: self.backend,
            graph_keys.TOOLS: self.tools,
            graph_keys.TOOL_GROUPS: self.tool_groups,
            graph_keys.TOKEN_BUDGET: self.token_budget,
            graph_keys.OUTPUT_SCHEMA: self.output_schema,
            graph_keys.MAX_SCHEMA_RETRIES: self.max_schema_retries,
            graph_keys.MAX_ITERATIONS: self.max_iterations,
            graph_keys.TIMEOUT_MS: self.timeout_ms,
        }

        if self.tools_enabled is not None:
            values[graph_keys.TOOLS_ENABLED] = self.tools_enabled
        elif self.tools is not None:
            values[graph_keys.TOOLS_ENABLED] = len(self.tools) > 0
        elif self.tool_groups is not None:
            values[graph_keys.TOOLS_ENABLED] = len(self.tool_groups) > 0

        if self.tools_config is not None:
            values[graph_keys.TOOLS_CONFIG] = self.tools_config.to_dict()

        return _drop_none(values)


class HookEvent(str, Enum):
    GRAPH_START = "graph_start"
    GRAPH_END = "graph_end"
    NODE_START = "node_start"
    NODE_COMPLETE = "node_complete"
    NODE_ERROR = "node_error"
    TOOL_START = "tool_start"
    TOOL_END = "tool_end"


class MiddlewareKind(str, Enum):
    TIMEOUT = "timeout"
    LOOP_GUARD = "loop_guard"


class WorkflowTargetKind(str, Enum):
    REGISTERED_FLOW = graph_keys.WORKFLOW_TARGET_KIND_REGISTERED_FLOW
    GRAPH_PATH = graph_keys.WORKFLOW_TARGET_KIND_GRAPH_PATH
    ARTIFACT_PATH = graph_keys.WORKFLOW_TARGET_KIND_ARTIFACT_PATH
    WORKFLOW_PATH = graph_keys.WORKFLOW_TARGET_KIND_WORKFLOW_PATH

    @classmethod
    def spawn_path_kinds(cls) -> tuple["WorkflowTargetKind", ...]:
        return (
            cls.GRAPH_PATH,
            cls.ARTIFACT_PATH,
            cls.WORKFLOW_PATH,
        )


class SearchDepth(str, Enum):
    BASIC = "basic"
    ADVANCED = "advanced"


@dataclass(slots=True)
class HookConfig:
    event: HookEvent
    command: str
    shell: str | None = None

    def __post_init__(self) -> None:
        if not isinstance(self.event, HookEvent):
            raise TypeError("HookConfig.event must be a HookEvent")

    def to_toml_table(self) -> dict[str, Any]:
        return _drop_none(
            {
                "event": self.event.value,
                "command": self.command,
                "shell": self.shell,
            }
        )


@dataclass(slots=True)
class TimeoutMiddlewareConfig:
    default_timeout_ms: int | None = None

    def to_toml_table(self) -> dict[str, Any]:
        return _drop_none(
            {
                "kind": MiddlewareKind.TIMEOUT.value,
                "default_timeout_ms": self.default_timeout_ms,
            }
        )


@dataclass(slots=True)
class LoopGuardMiddlewareConfig:
    max_repeats: int = 1

    def to_toml_table(self) -> dict[str, Any]:
        return {
            "kind": MiddlewareKind.LOOP_GUARD.value,
            "max_repeats": self.max_repeats,
        }


@dataclass(slots=True)
class ExecutionOptions:
    """Execution-time controls for APXM frontend runs.

    `session_id`, `session_root`, `token_budget`, `output_schema`, and
    `max_schema_retries` are supported over the HTTP server path today.
    Hooks and middlewares require local CLI execution.
    """

    session_id: str | None = None
    session_root: str | os.PathLike[str] | None = None
    token_budget: int | None = None
    output_schema: dict[str, Any] | None = None
    max_schema_retries: int | None = None
    hooks: list[HookConfig] = field(default_factory=list)
    middlewares: list[TimeoutMiddlewareConfig | LoopGuardMiddlewareConfig] = field(
        default_factory=list
    )

    def requires_local_cli(self) -> bool:
        return bool(self.hooks or self.middlewares)

    def server_request_fields(self) -> dict[str, Any]:
        return _drop_none(
            {
                "session_id": self.session_id,
                "session_root": os.fspath(self.session_root) if self.session_root is not None else None,
                "token_budget": self.token_budget,
                "output_schema": self.output_schema,
                "max_schema_retries": self.max_schema_retries,
            }
        )

    def config_toml(self) -> str:
        lines: list[str] = []

        for hook in self.hooks:
            lines.append("[[hooks]]")
            for key, value in hook.to_toml_table().items():
                lines.append(f"{key} = {_toml_value(value)}")
            lines.append("")

        for middleware in self.middlewares:
            lines.append("[[middlewares]]")
            for key, value in middleware.to_toml_table().items():
                lines.append(f"{key} = {_toml_value(value)}")
            lines.append("")

        return "\n".join(lines).strip() + ("\n" if lines else "")


@dataclass(slots=True)
class AgentConfig:
    name: str
    model: str = "deepseek-r1:14b"
    provider: str = "ollama"
    api_key: str | None = None
    base_url: str | None = None
    temperature: float | None = None
    system_prompt: str | None = None
    operation_instructions: dict[str, str] | None = None
    tools: list[str] | None = None
    tool_groups: list[str] | None = None
    tools_enabled: bool | None = None
    tools_config: ToolsConfig | None = None
    token_budget: int | None = None
    output_schema: dict[str, Any] | None = None
    max_schema_retries: int = 0
    max_iterations: int = 100

    def __post_init__(self) -> None:
        """Validate that the provider is registered with the APXM runtime."""
        resolve_provider(self.provider)  # raises ValueError if invalid

    def to_node_attributes(self) -> dict[str, Any]:
        values = NodePolicy(
            model=self.model,
            provider=self.provider,
            api_key=self.api_key,
            base_url=self.base_url,
            temperature=self.temperature,
            system_prompt=self.system_prompt,
            tools=self.tools,
            tool_groups=self.tool_groups,
            tools_enabled=self.tools_enabled,
            tools_config=self.tools_config,
            token_budget=self.token_budget,
            output_schema=self.output_schema,
            max_schema_retries=self.max_schema_retries,
            max_iterations=self.max_iterations,
        ).to_node_attributes()
        values[graph_keys.AGENT_NAME] = self.name
        return values


def _toml_value(value: Any) -> str:
    if isinstance(value, str):
        import json

        return json.dumps(value)
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, list):
        return "[" + ", ".join(_toml_value(item) for item in value) + "]"
    if value is None:
        raise ValueError("None is not a valid TOML scalar")
    return str(value)
