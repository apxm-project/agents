from __future__ import annotations

from dataclasses import asdict, dataclass, field
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
    search_depth: str = "basic"
    endpoint: str = "https://api.tavily.com/search"
    include_answer: bool = True


@dataclass(slots=True)
class ToolsConfig:
    bash: BashConfig | None = None
    read: ReadConfig | None = None
    write: WriteConfig | None = None
    search_web: SearchWebConfig | None = None

    def to_dict(self) -> dict[str, Any]:
        return _drop_none(asdict(self))


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
    tools_config: ToolsConfig | None = None
    token_budget: int | None = None
    output_schema: dict[str, Any] | None = None
    max_schema_retries: int = 0
    max_iterations: int = 100

    def __post_init__(self) -> None:
        """Validate that the provider is registered with the APXM runtime."""
        resolve_provider(self.provider)  # raises ValueError if invalid

    def to_node_attributes(self) -> dict[str, Any]:
        values: dict[str, Any] = {
            graph_keys.AGENT_NAME: self.name,
            graph_keys.MODEL: self.model,
            graph_keys.PROVIDER: self.provider,
            graph_keys.API_KEY: self.api_key,
            graph_keys.BASE_URL: self.base_url,
            graph_keys.TEMPERATURE: self.temperature,
            graph_keys.SYSTEM_PROMPT: self.system_prompt,
            graph_keys.TOOLS: self.tools,
            graph_keys.TOKEN_BUDGET: self.token_budget,
            graph_keys.OUTPUT_SCHEMA: self.output_schema,
            graph_keys.MAX_SCHEMA_RETRIES: self.max_schema_retries,
            graph_keys.MAX_ITERATIONS: self.max_iterations,
        }

        if self.tools is not None:
            values[graph_keys.TOOLS_ENABLED] = len(self.tools) > 0

        if self.tools_config is not None:
            values[graph_keys.TOOLS_CONFIG] = self.tools_config.to_dict()

        return _drop_none(values)
