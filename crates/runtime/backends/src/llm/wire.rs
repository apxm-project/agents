//! Backend-owned wire constants.
//!
//! These names describe provider HTTP payloads and backend registration JSON.
//! They are intentionally kept outside `apxm-core` so core remains backend
//! agnostic.

pub mod headers {
    pub const CONTENT_TYPE: &str = "content-type";
    pub const CONTENT_TYPE_JSON: &str = "application/json";
    pub const AUTHORIZATION: &str = "authorization";
    pub const X_API_KEY: &str = "x-api-key";
    pub const ANTHROPIC_VERSION: &str = "anthropic-version";
    pub const X_GOOG_API_KEY: &str = "x-goog-api-key";
}

pub mod message_keys {
    pub const ROLE: &str = "role";
    pub const CONTENT: &str = "content";
    pub const TYPE: &str = "type";
    pub const TEXT: &str = "text";
    pub const IMAGE: &str = "image";
    pub const IMAGE_URL: &str = "image_url";
    pub const SOURCE: &str = "source";
    pub const URL: &str = "url";
    pub const MAX_TOKENS: &str = "max_tokens";
    pub const STREAM: &str = "stream";
}

pub mod tool_keys {
    pub const TOOLS: &str = "tools";
    pub const TOOL_CHOICE: &str = "tool_choice";
    pub const TOOL_USE: &str = "tool_use";
    pub const TOOL_RESULT: &str = "tool_result";
    pub const TOOL_CALLS: &str = "tool_calls";
    pub const FUNCTION: &str = "function";
    pub const NAME: &str = "name";
    pub const ID: &str = "id";
    pub const INPUT: &str = "input";
    pub const INPUT_SCHEMA: &str = "input_schema";
    pub const ARGUMENTS: &str = "arguments";
}

pub mod roles {
    pub const USER: &str = "user";
    pub const ASSISTANT: &str = "assistant";
    pub const SYSTEM: &str = "system";
    pub const TOOL: &str = "tool";
}

pub mod api_paths {
    pub const VERSION_PREFIX: &str = "/v1";
    pub const CHAT_COMPLETIONS: &str = "/chat/completions";
    pub const MODELS: &str = "/models";
    pub const MESSAGES: &str = "/messages";
    pub const API_CHAT: &str = "/api/chat";
    pub const API_TAGS: &str = "/api/tags";
    pub const APXM_GRAPHS: &str = "/apxm/graphs";
    pub const APXM_GRAPHS_REGISTER: &str = "/apxm/graphs/register";
    /// `GET /v1/apxm/scheduler` — APXM-fork endpoint reporting the live
    /// scheduler policy so the backend can verify priority hints will be
    /// honored (FCFS silently ignores per-request priority).
    pub const APXM_SCHEDULER: &str = "/apxm/scheduler";
    /// `POST /v1/apxm/admin/reset_prefix_cache` — operator endpoint used
    /// by benchmark harnesses to flush the prefix cache between cells.
    pub const APXM_ADMIN_RESET_PREFIX_CACHE: &str = "/apxm/admin/reset_prefix_cache";
}

pub mod anthropic_events {
    pub const MESSAGE_START: &str = "message_start";
    pub const CONTENT_BLOCK_START: &str = "content_block_start";
    pub const CONTENT_BLOCK_DELTA: &str = "content_block_delta";
    pub const CONTENT_BLOCK_STOP: &str = "content_block_stop";
    pub const MESSAGE_DELTA: &str = "message_delta";
    pub const MESSAGE_STOP: &str = "message_stop";
    pub const ERROR: &str = "error";
}

pub mod sse {
    pub const DATA_PREFIX: &str = "data: ";
    pub const EVENT_PREFIX: &str = "event: ";
    pub const DONE_MARKER: &str = "[DONE]";
}

pub mod config_keys {
    pub const ID: &str = "id";
    pub const CONTEXT_WINDOW: &str = "context_window";
    pub const EXTRA_HEADERS: &str = "extra_headers";
    pub const ENV_PREFIX: &str = "env:";
    pub const AUTO_TOOL_CHOICE: &str = "auto_tool_choice";
    pub const MODELS: &str = "models";
    pub const SUPPORTS_VISION: &str = "supports_vision";
    pub const SUPPORTS_FUNCTIONS: &str = "supports_functions";
    pub const SUPPORTS_FINE_TUNING: &str = "supports_fine_tuning";
    pub const SUPPORTS_THINKING: &str = "supports_thinking";
    pub const SUPPORTS_CUSTOM_TEMPERATURE: &str = "supports_custom_temperature";
    pub const SUPPORTS_STRUCTURED_OUTPUTS: &str = "supports_structured_outputs";
    /// Exact profile admission for llama.cpp's `cache_prompt` lowering.
    pub const GRAPH_HINT_CACHE_PROMPT_ADMITTED: &str = "graph_hint_cache_prompt_admitted";
    pub const USES_REASONING_TOKEN_FIELDS: &str = "uses_reasoning_token_fields";
    pub const CHAT_TEMPLATE_KWARGS: &str = "chat_template_kwargs";
    pub const ENABLE_THINKING: &str = "enable_thinking";
}

pub mod openai {
    pub const MODEL: &str = "model";
    pub const MESSAGES: &str = "messages";
    pub const ROLE: &str = "role";
    pub const CONTENT: &str = "content";
    pub const STREAM_OPTIONS: &str = "stream_options";
    pub const INCLUDE_USAGE: &str = "include_usage";
    pub const TEMPERATURE: &str = "temperature";
    pub const EXTRA_BODY: &str = "extra_body";
    pub const TOP_P: &str = "top_p";
    pub const FREQUENCY_PENALTY: &str = "frequency_penalty";
    pub const PRESENCE_PENALTY: &str = "presence_penalty";
    pub const STOP: &str = "stop";
    pub const RESPONSE_FORMAT: &str = "response_format";
    pub const RESPONSE_FORMAT_TYPE: &str = "type";
    pub const RESPONSE_FORMAT_JSON_SCHEMA: &str = "json_schema";
    pub const JSON_SCHEMA: &str = "json_schema";
    pub const JSON_SCHEMA_NAME: &str = "name";
    pub const JSON_SCHEMA_SCHEMA: &str = "schema";
    pub const JSON_SCHEMA_STRICT: &str = "strict";
    pub const APXM_OUTPUT_SCHEMA_NAME: &str = "apxm_output";
}

pub mod backend_metadata {
    pub const BACKEND_TYPE: &str = "backend_type";
}

pub mod google {
    pub const USAGE_METADATA: &str = "usageMetadata";
    pub const PROMPT_TOKEN_COUNT: &str = "promptTokenCount";
    pub const CANDIDATES_TOKEN_COUNT: &str = "candidatesTokenCount";
}

pub mod ollama {
    pub const NUM_PREDICT: &str = "num_predict";
    pub const THINK: &str = "think";
}

pub mod response_metadata {
    pub const APXM_BACKEND_NAME: &str = "apxm_backend_name";
    pub const APXM_BACKEND_MODEL: &str = "apxm_backend_model";
    pub const REASONING: &str = "reasoning";
}

pub mod defaults {
    pub const ANTHROPIC_MAX_TOKENS: usize = 4096;
    pub const GOOGLE_MAX_OUTPUT_TOKENS: usize = 2048;
    pub const GOOGLE_TOP_P: f64 = 0.95;
}
