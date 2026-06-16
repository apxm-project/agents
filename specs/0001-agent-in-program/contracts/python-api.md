# Contract: Python authoring API

The author-facing surface in `apxm` (frontend package). Stable names + shapes the
implementation MUST provide. Behavior is normative; signatures are indicative.

## `@tool` (existing — unchanged)
Decorates a Python function into a callable capability; produces a `handler_id`
resolved by the runtime bridge. Reused verbatim as the hook backing mechanism.

## `@hook` (new)
```
@hook(on: HookEvent, match: str = "*", mode: "observe" | "gate" = "observe")
def fn(ctx, ...): ...
```
- `on ∈ {session_start, pre_turn, post_turn, pre_ask, post_ask, pre_tool, post_tool}`.
- `mode="gate"` permitted only on `pre_*`; enables flow control.
- Handler receives a `ctx` with: `log`, `remaining_budget`, memory access
  (`umem`/`recall_window(n)`), `read_agents_md`, `summarize`, and event payload
  (`call` for tool events with `.args`; `reply` for `post_turn`).
- Return contract: `pre_tool` → `ctx.allow()` | `ctx.deny(reason)` |
  `ctx.edit_args(new_args)`; `post_tool` → `ctx.replace_result(x)` or None;
  `pre_ask` → `ctx.prepend_system(text)` / `ctx.set_system(text)`; others → None.
- MUST run on both hosts (FR-006). MUST surface handler errors (FR-014).

## `ConversationalAgent` (new)
```
ConversationalAgent(
  persona: str,
  memory_space: str = "stm",
  tools: list = [],
  tool_groups: list[str] = [],
  skills: bool = False,
  sub_agents: list[Agent] = [],
  compaction: CompactionPolicy | None = None,
  hooks: list[HookFn] = [],
  loop: "host" | "in_graph" = "in_graph",
).compile() -> Artifact
```
- Emits ONE multi-flow artifact: entry loop flow + author turn flow + one flow per
  sub-agent + the hooks/tools sidecars.
- Declares the reserved turn/user-message parameter by name (no positional bind).
- `skills=True` enables real description-based discovery (the `skills` group).
- `loop="in_graph"` puts the conversation loop in the artifact (target);
  `loop="host"` keeps a thin host loop (compatibility default until the native
  loop lands).

## `CompactionPolicy` (new)
```
CompactionPolicy(keep_recent: int = 4, compact_at_tokens: int = 20_000,
                 strategy: "summarize" = "summarize")
```
Authored in-program (FR-008). The builder uses `keep_recent` for the recent
recall window and pins `summary_key`; the author's `post_turn` hook owns the
threshold, summarization prompt, and fold via `ctx.count_tokens`, `ctx.ask`, and
`ctx.umem`.

## `skill_search(query, imports=None)` (new helper)
First-class wrapper over the real `search_skills` capability so authors need not
know the capability name. Returns ranked `{id, description}` matches.

## Removed
- `AgentHooks(on_start/on_tool_call/on_end)` — deleted (decorative, never
  lowered). Replaced by `@hook`.
