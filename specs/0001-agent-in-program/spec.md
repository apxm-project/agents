# Feature Specification: The Whole Conversational Agent in One Program

**Feature dir:** `specs/0001-agent-in-program/`
**Phase:** specify (next: clarify)
**Input:** "All the different things of a conversational agent — the loop, context
management, skills/tool discovery, pre/post/session-start hooks, sub-agents —
should live in the conversational-agent program itself. We should be able to do
it all in there; the host should be a dumb pipe."

## User Scenarios & Testing

### User Story 1 — Author the whole agent as one program (Priority: P1)

An author writes a single conversational-agent definition that contains the
entire behavior: the turn-by-turn loop, using tools, and remembering across
turns. Nothing essential about how the agent behaves lives in whatever runs it.

**Why this priority:** This is the core promise of the feature. Every other
capability (hooks, context policy, sub-agents) composes onto a program that can
already hold a multi-turn conversation by itself. Without this, there is no
"agent in one program."

**Independent Test:** An author writes one agent definition and runs it; it holds
a multi-turn conversation that uses a tool and recalls a fact from an earlier
turn, with no conversational logic supplied by the runner.

**Acceptance Scenarios:**

1. **Given** a single agent definition with one tool and memory, **When** the
   user sends three messages in sequence, **Then** each reply reflects tool
   results where relevant and correctly references information from earlier turns.
2. **Given** the same definition, **When** the user asks in turn 3 about
   something stated in turn 1, **Then** the agent answers correctly from recalled
   context.
3. **Given** the agent is running, **When** the user sends a new message,
   **Then** the reply is produced without re-deriving the earlier turns.

### User Story 2 — Run the same agent unchanged on any host (Priority: P1)

The identical agent definition behaves the same whether it runs in an interactive
terminal or as a hosted service. The runner only delivers user input and renders
output.

**Why this priority:** "All in the program" is only meaningful if the program is
the single source of behavior across every place it runs. A capability that works
in one host but not another is not delivered.

**Independent Test:** Run one unchanged agent definition in the interactive
terminal and as a hosted service; the same inputs produce equivalent replies and
the same author-defined behavior fires in both.

**Acceptance Scenarios:**

1. **Given** one agent definition, **When** it runs in an interactive terminal
   and as a hosted service with the same inputs, **Then** the replies are
   equivalent and every author-defined behavior fires identically in both.
2. **Given** a host with no agent-specific behavior configured, **When** the
   agent runs, **Then** the full behavior still appears, sourced entirely from
   the program.

### User Story 3 — Control every turn with lifecycle rules (Priority: P2)

The author defines rules that run at session start, before and after each turn,
and before and after each tool use. A before-tool rule can allow, block, or
modify the tool call; an after-tool rule can transform the result; a before-turn
rule can inject context into the reply.

**Why this priority:** Control over the lifecycle is the differentiator — "we
provide the basis so the author controls every detail." It depends on US1/US2
existing first.

**Independent Test:** An author attaches a before-tool rule that blocks one tool
call and edits another, plus a before-turn rule that injects context; all are
observed firing in a real session on both hosts.

**Acceptance Scenarios:**

1. **Given** a before-tool rule that blocks a specific argument, **When** the
   agent attempts that tool call, **Then** the call does not execute and the
   agent is told why.
2. **Given** a before-tool rule that rewrites an argument, **When** the tool is
   called, **Then** the tool runs with the rewritten argument.
3. **Given** an after-tool rule that redacts results, **When** a tool returns
   sensitive data, **Then** the agent only sees the redacted result.
4. **Given** a session-start rule, **When** a new session begins, **Then** the
   rule runs exactly once before the first turn.
5. **Given** any author-defined rule, **When** the agent runs on either host,
   **Then** the rule fires in both.

### User Story 4 — The agent manages its own context (Priority: P2)

The agent keeps the conversation within its working limit across long sessions
without losing information the author marked important, summarizing older content
as needed. The author sets the context policy (how many recent turns are kept
verbatim, when summarization triggers) within the program.

**Why this priority:** Long real conversations fail without this, and the mandate
requires the policy to live in the program, not the host.

**Independent Test:** Run a long session that exceeds the working limit; the agent
still answers a question whose answer appeared early, and changing the policy in
the program changes the observed retention behavior.

**Acceptance Scenarios:**

1. **Given** a session that grows past the working context limit, **When** the
   conversation continues, **Then** the agent keeps responding coherently and
   does not exceed the limit.
2. **Given** a fact stated in the first turn, **When** the session has since
   exceeded the limit and been summarized, **Then** the agent still answers a
   later question about that fact.
3. **Given** the author changes the retention policy in the program, **When** the
   session runs, **Then** the retained-vs-summarized boundary changes accordingly.

### User Story 5 — Discover skills and delegate to sub-agents, in the same program (Priority: P3)

The agent finds the most relevant installed capability for a request by its
description without the author hard-coding which one, and the author can define
specialist sub-agents within the same program and delegate a subtask to one,
folding the result back into the reply.

**Why this priority:** High-value composition, but the agent is already useful
with US1–US4. Builds on the single-program foundation.

**Independent Test:** In one program, the agent selects a relevant skill by
description for a request it was not pre-told about, and delegates a subtask to a
sub-agent defined in the same program, using the sub-agent's result in the reply.

**Acceptance Scenarios:**

1. **Given** several installed capabilities, **When** the user makes a request
   that matches one by description, **Then** the agent selects and uses it
   without the author naming it.
2. **Given** a sub-agent defined in the same program, **When** the agent
   delegates a subtask, **Then** the sub-agent runs and its result is
   incorporated into the final reply.

### Edge Cases

- A very long session that repeatedly exceeds the working limit (sustained
  compaction, not a one-off).
- An author-defined rule that errors at runtime — must not silently corrupt the
  turn; the failure must surface.
- A tool blocked by a before-tool rule — the agent must continue the turn
  gracefully.
- A delegated sub-agent that fails or returns nothing.
- The runner disconnects mid-turn — partial output must not be reported as a
  complete reply.
- Empty, malformed, or hostile user input.
- No stable session identity available — cross-turn memory degrades predictably
  rather than crashing.

## Requirements

### Functional Requirements

- **FR-001:** The agent's full conversational behavior — the turn loop, tool use,
  and cross-turn memory — MUST be expressible within a single authored agent
  program.
- **FR-002:** A host running the agent MUST NOT be required to add conversational
  behavior; its role MUST be limited to delivering user input and rendering agent
  output.
- **FR-003:** The same agent program MUST produce equivalent behavior whether run
  in an interactive terminal or as a hosted service.
- **FR-004:** The author MUST be able to define rules that run before and after
  each tool use, and those rules MUST be able to allow, block, or modify the tool
  call and its result.
- **FR-005:** The author MUST be able to define rules that run at session start
  and before and after each turn, including injecting context before a reply.
- **FR-006:** Every author-defined lifecycle rule MUST run on every supported
  host, not only one.
- **FR-007:** The agent MUST keep the conversation within its working context
  limit across long sessions, summarizing older content while retaining
  information the author marked important.
- **FR-008:** The author MUST be able to set the context and compaction policy
  from within the agent program.
- **FR-009:** The agent MUST be able to recall relevant prior context, including
  earlier turns, when forming each reply.
- **FR-010:** The agent MUST be able to find the most relevant installed
  capability for a request by its description, without the author hard-coding the
  choice.
- **FR-011:** The author MUST be able to define one or more specialist sub-agents
  within the same agent program and delegate a subtask to one, receiving the
  result back into the turn.
- **FR-012:** A single new user message MUST be processed and its reply streamed
  back without re-running prior turns.
- **FR-013:** The agent MUST preserve memory and conversation state across turns
  within a session.
- **FR-014:** The system MUST surface a clear error when an author-defined rule,
  sub-agent, capability, or policy is misconfigured; it MUST NOT silently ignore
  it.

### Key Entities

- **Agent program** — the single authored definition that holds all behavior.
- **Turn** — one user message and the agent's resulting reply, including any tool
  use and delegation.
- **Lifecycle rule** — an author-defined behavior bound to a defined moment
  (session start, before/after turn, before/after tool).
- **Sub-agent** — a specialist the agent program defines and delegates subtasks
  to.
- **Session memory** — the cross-turn state (history, summary, deliberately
  stored facts) scoped to one conversation.
- **Skill / capability** — an installed unit of functionality the agent can
  discover by description and invoke.

## Success Criteria

### Measurable Outcomes

- **SC-001:** An author implements a working multi-turn agent (loop, one tool,
  cross-turn recall) entirely within one agent program, with zero conversational
  logic added by the host.
- **SC-002:** The identical program runs in both reference hosts and produces
  equivalent replies for the same inputs — 100% of US1–US5 acceptance scenarios
  pass on both hosts.
- **SC-003:** A session of at least 50 turns stays within the working context
  limit and still correctly answers a question whose answer was given in turn 1.
- **SC-004:** Each author-defined lifecycle rule (before/after tool, session
  start, before/after turn) is observed firing on both hosts; a block rule
  prevents a tool call and an edit rule changes it, observably.
- **SC-005:** For a representative set of requests, the agent selects the correct
  skill by description in at least 8 of 10 cases without the author naming it.
- **SC-006:** A delegated sub-agent's result is incorporated into the final reply
  in 100% of delegation scenarios.
- **SC-007:** Processing one additional message does not require recomputing prior
  turns: per-turn work grows only with retained context size, not with the count
  of already-completed turns.

## Clarifications

### Session 2026-06-15

- Q: For the first delivered milestone, where is the boundary — turn-in-program
  first with the loop as a later capstone, or the full in-program loop up front?
  → A: **Finish it all.** No reduced milestone. The complete feature, including
  the in-program conversation loop (and the runtime/scheduler work it requires),
  is in scope for delivery — not deferred.

## Assumptions

- The two reference hosts are the interactive terminal and the hosted service;
  any other runner (e.g. a visual builder) inherits the same dumb-pipe contract.
- "Information the author marked important" is whatever the program deliberately
  stores as memory; everything else is eligible for summarization.
- A session has a stable identifier so memory accrues across turns; without one,
  cross-turn memory degrades predictably and is surfaced, not crashed.
- Existing capability installation, skill catalogues, and credential handling are
  reused; this feature does not redefine them.
- The feature is delivered as one complete scope (per the 2026-06-15
  clarification): the in-program loop is included, not deferred. The current
  host-driven conversational agent remains available and unchanged during
  development for backward compatibility, but the delivered target is the full
  in-program agent.
- Whether a sub-agent runs in-process or as a separate worker is the author's
  choice; both satisfy FR-011.
