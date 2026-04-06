# apxm-designer

AI-first commercial app builder on APXM. You describe an app. It builds all screens × all states, verified in HTML, before a single line of implementation code is written.

## The Gap This Fills

Every existing tool (Replit, Firebase Studio, Bolt, v0) skips the design phase — goes straight to code, discovers missing states in QA. Figma has design but it's disconnected from code.

**apxm-designer enforces a structured design phase:**
All screens × all states → verified HTML → any framework code.
The HTML IS the design, the spec, and the conversion source.

## Session Isolation

Every workflow run creates an isolated session directory:

```
~/.apxm-designer/sessions/{project_name}-{YYYYMMDD-HHMMSS}/
├── state.json          ← tracks completion status per phase
├── specs/              ← Phase 1 outputs (entity_map.json, etc.)
├── components/         ← Phase 2A component HTML
├── screens/            ← Phase 2B/2C screen HTML
│   ├── shared/         ← common screens (profile, messaging, settings)
│   └── {capability}/   ← feature screens by capability
└── code/               ← Phase 3 framework code
    ├── widgets/        ← framework components
    ├── screens/        ← converted screen code
    ├── backend/        ← API endpoints
    ├── routing/        ← route definitions
    ├── state/          ← state management
    ├── di/             ← dependency injection
    ├── auth/           ← auth guards
    └── i18n/           ← internationalization
```

**Why session isolation?**
- **Reproducibility**: Every artifact is in one directory, timestamped
- **Debuggability**: Inspect any phase's outputs
- **Traceability**: state.json shows what completed, what failed
- **Parallelism**: Multiple runs don't collide
- **Resumability**: Can continue from any phase

**How it works:**
- `create.ais` generates session_dir path with timestamp
- All phase agents receive session_dir as parameter
- Agents read inputs from session_dir using bash tool
- Agents write outputs to session_dir using bash tool
- state.json updated after each phase completion

## Usage

```bash
# Full app from scratch
dekk apxm execute examples/workflows/appbuilder/create.ais

# Find your session directory in the output:
# SESSION CREATED: ~/.apxm-designer/sessions/marketplace-20260406-143022/

# Inspect session state
cat ~/.apxm-designer/sessions/marketplace-20260406-143022/state.json

# View generated specs
ls ~/.apxm-designer/sessions/marketplace-20260406-143022/specs/

# View generated components
ls ~/.apxm-designer/sessions/marketplace-20260406-143022/components/

# View generated code
ls ~/.apxm-designer/sessions/marketplace-20260406-143022/code/
```

## File Structure

```
appbuilder/
├── create.ais              ← entry: full app from scratch
├── add-feature.ais         ← entry: delta on existing project
├── edit-screen.ais         ← entry: regen one screen + verify
├── add-part.ais            ← entry: one new component
├── phases/
│   ├── discover.ais        ← Phase 1: 4 specialists + Q&A + 6 spec artifacts
│   ├── common-parts.ais    ← Phase 2A: component library (codex+claude+verify)
│   ├── common-screens.ais  ← Phase 2B: shared screens (profile/messaging/settings)
│   ├── feature-screens.ais ← Phase 2C: N×V screens in parallel + 2-stage verify
│   ├── implement.ais       ← Phase 3C: backend + routing + state + DI + auth + i18n
│   └── test.ais            ← Phase 4: stack tests + coverage + quality gate
└── sub/
    ├── component-widget.ais ← atomic: 1 component HTML → framework widget
    ├── screen.ais          ← atomic: 1 file, conservative+bold+pick winner
    ├── verify-structural.ais ← Stage 1: structural validation
    ├── verify-visual.ais   ← Stage 2: Playwright→PNG→visual review
    ├── convert.ais         ← HTML→framework (deterministic mapping)
    ├── export-to-figma.ais ← verified HTML→Figma frames+components+tokens
    └── sync-from-figma.ais ← round-trip: read Figma changes, update HTML
```

## The Pipeline

```
Phase 1: DISCOVER
  4 specialists (entity, competitor, navigation, design) → 6 JSON spec artifacts
  Architect writes specs while Adversary challenges every assumption
  Outputs: session_dir/specs/*.json

Phase 2A: COMMON PARTS
  component_registry.json → each component: Codex + Claude → judge picks winner
  2-stage verification before any screen can use them
  Outputs: session_dir/components/*.html

Phase 2B: COMMON SCREENS
  Shared screens (profile, messaging, settings) using verified components
  Outputs: session_dir/screens/shared/*.html

Phase 2C: FEATURE SCREENS
  ALL role screens × ALL state variants in parallel
  Stage 1: structural verification (all HTML + all JSON in one pass)
  Stage 2: visual review (all screenshots simultaneously)
  Outputs: session_dir/screens/**/*.html

Phase 3A: COMPONENT WIDGETS
  Generate framework widgets for all components (must run BEFORE convert)
  Codex conservative + Claude bold → Claude picks winner
  Outputs: session_dir/code/widgets/*.dart|tsx

Phase 3B: CONVERT
  sub/convert.ais × N×V HTML files (deterministic HTML→framework)
  Outputs: session_dir/code/screens/*.dart|tsx

Phase 3C: IMPLEMENT
  Backend endpoints, routing, state management, DI, auth guards, i18n
  Outputs: session_dir/code/{backend,routing,state,di,auth,i18n}/

Phase 4: TEST
  Stack tests → coverage audit → quality gate
  Outputs: session_dir/{test_results,coverage,quality_report}.json
```

## Key Innovations

1. **Session isolation** — every run is reproducible, debuggable, traceable
2. **File handoffs via bash** — agents read/write session_dir files, not passing huge JSON strings through ask()
3. **HTML as universal intermediate** — browser/Playwright/converter all use same file
4. **Verification gate** — structural + visual BLOCK implementation if they fail
5. **Component registry** — one definition drives HTML mockup, Figma component, Code Connect, framework widget
6. **File-per-state = state machine** — dashboard.html → DashboardState.loaded, dashboard-empty.html → DashboardState.empty
7. **design_system.json** — single source: CSS vars → Figma variables → framework theme tokens

## Example: Tracing a Run

```bash
# Start a run
dekk apxm execute examples/workflows/appbuilder/create.ais

# Output shows:
# SESSION CREATED: ~/.apxm-designer/sessions/marketplace-20260406-143022/

# Check current phase
cat ~/.apxm-designer/sessions/marketplace-20260406-143022/state.json
# {"phase":"discover","status":"complete","timestamp":"20260406-143022","brief":"..."}

# View specs generated in Phase 1
ls ~/.apxm-designer/sessions/marketplace-20260406-143022/specs/
# entity_map.json  navigation_spec.json  competitor_analysis.json
# design_system.json  product_spec.json  screen_manifest.json

# View components generated in Phase 2A
ls ~/.apxm-designer/sessions/marketplace-20260406-143022/components/
# alert-card.html  empty-state.html  verification.json

# View screens generated in Phase 2C
find ~/.apxm-designer/sessions/marketplace-20260406-143022/screens/ -name '*.html'
# screens/shared/profile.html
# screens/seller/home/dashboard.html
# screens/seller/home/dashboard-empty.html
# screens/seller/home/dashboard-triage.html

# View converted code from Phase 3B
ls ~/.apxm-designer/sessions/marketplace-20260406-143022/code/screens/
# dashboard.html.dart  dashboard-empty.html.dart  dashboard-triage.html.dart

# View test results from Phase 4
cat ~/.apxm-designer/sessions/marketplace-20260406-143022/test_results.json
# {"total_tests":127,"passed":125,"failed":2,"skipped":0,...}
```

## Benefits of Session-Based Design

**For users:**
- Know exactly where your outputs are
- Can inspect intermediate artifacts at any time
- Can resume from a specific phase if something failed
- Easy to compare runs (different timestamps)

**For agents:**
- No massive JSON strings passed through ask()
- Simple bash reads/writes instead of complex data flow
- Can verify outputs before next phase reads them
- State management is explicit (state.json)

**For debugging:**
- All artifacts in one timestamped directory
- Can replay any phase with the same inputs
- Can diff two session directories to see what changed
- Verification results saved alongside artifacts
