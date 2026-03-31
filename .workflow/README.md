# workflow-test

Agentic workflow orchestration with [acpx](https://github.com/openclaw/acpx) flows.
Multi-step agent pipelines that review PRs, triage bugs, and run coding agents
(Claude Code, Codex, Cline, Gemini) through deterministic workflows.

Built on [dekk](https://github.com/randreshg/dekk) for agent config generation
and [OpenClaw](https://github.com/openclaw/openclaw) for trigger orchestration.

## Architecture

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full system design.

```
  GitHub / Slack / Cron
         |
         v
  +--------------+     +------------------+     +----------------+
  |   OpenClaw   | --> |   acpx flows     | --> | Coding Agents  |
  |   (trigger)  |     |   (execution)    |     | (Claude, Codex)|
  +--------------+     +------------------+     +----------------+
                               |
                        dekk agents generate
                               |
                        .agents/ (SSOT)
```

## Quick Start

### Prerequisites

```bash
pip install dekk              # Agent config CLI
conda create -n workflow-test nodejs>=22
conda activate workflow-test
npm install -g pnpm

# Clone and build acpx from source
git clone https://github.com/openclaw/acpx.git ../acpx
cd ../acpx && pnpm install && pnpm run build && pnpm link --global
cd ../workflow-test

# Coding agents (install separately)
# codex: https://github.com/openai/codex
# claude: https://github.com/anthropics/claude-code
```

### Setup

```bash
# Generate agent configs from .agents/ source of truth
dekk agents generate

# Install flow dependencies
cd flows && npm install && cd ..
```

### Run Flows

```bash
cd flows

# Echo test — spawns both Codex and Claude to compare answers
acpx --approve-all flow run ./echo.flow.ts \
  --input-json '{"question":"What is 2+2?"}'

# PR code review — fetches diff, reviews, posts verdict
acpx --approve-all flow run ./review.flow.ts \
  --input-json '{"repo":"owner/repo","prNumber":42}'

# PR triage — multi-step: extract intent, test, review, route
acpx --approve-all flow run ./triage.flow.ts \
  --input-json '{"repo":"owner/repo","prNumber":42}'
```

### Replay Viewer

Visualize flow runs as interactive DAG graphs:

```bash
cd ../acpx
pnpm run viewer:dev
# Open http://127.0.0.1:4173
```

### Generate New Flows

```bash
dekk agents flow review    # flows/review.flow.ts (6 nodes)
dekk agents flow triage    # flows/triage.flow.ts (9 nodes)
dekk agents flow echo      # flows/echo.flow.ts  (4 nodes)
```

## Project Structure

```
workflow-test/
├── .agents/                  # Source of truth (committed)
│   ├── project.md            #   Project-level instructions
│   ├── skills/               #   Agent skill definitions
│   │   ├── build/SKILL.md    #     How to build the project
│   │   ├── test/SKILL.md     #     How to run tests
│   │   └── review/SKILL.md   #     Code review guidelines
│   └── rules/                #   Path-scoped rules
├── flows/                    # acpx flow definitions
│   ├── echo.flow.ts          #   Agent echo test (Codex + Claude)
│   ├── review.flow.ts        #   PR code review (6 nodes)
│   ├── triage.flow.ts        #   PR triage pipeline (9 nodes)
│   ├── lib/utils.ts          #   Shared skill embedding helpers
│   ├── package.json          #   acpx dependency (source-linked)
│   └── tsconfig.json
├── .dekk.toml                # dekk project config
├── ARCHITECTURE.md           # System design document
└── README.md
```

## Related Projects

| Project | Role |
|---------|------|
| [dekk](https://github.com/randreshg/dekk) | Agent config generation CLI |
| [acpx](https://github.com/openclaw/acpx) | Headless ACP CLI + flow runtime |
| [OpenClaw](https://github.com/openclaw/openclaw) | Messaging gateway + trigger orchestration |
