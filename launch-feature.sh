#!/bin/bash
# launch-feature.sh — Spawn Claude Code to implement a feature
#
# Usage: ./launch-feature.sh "feature description"
#        ./launch-feature.sh  # Uses default: backend unification
#
# This script:
# 1. Reads the implementation spec from docs/architecture/backend-implementation-spec.md
# 2. Spawns Claude Code with full context
# 3. Runs in foreground so you can monitor progress

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

# Default feature if none provided
DEFAULT_FEATURE="Implement the Backend Unification feature as specified in docs/architecture/backend-implementation-spec.md. Follow the implementation plan phases 1-7 in order. After each phase, run 'dekk apxm build' and 'dekk apxm test' to verify. Fix any errors before proceeding to the next phase."

FEATURE="${1:-$DEFAULT_FEATURE}"

# Ensure we're in the right environment
if ! command -v dekk &>/dev/null; then
    echo "Error: dekk not found. Run from a shell with dekk available."
    exit 1
fi

# Verify we can build
echo "🔍 Verifying build environment..."
if ! dekk apxm build --help &>/dev/null; then
    echo "Error: 'dekk apxm build' not available. Check .dekk.toml"
    exit 1
fi

# Check Claude Code is available
if ! command -v claude &>/dev/null; then
    echo "Error: claude CLI not found. Install with: npm install -g @anthropic-ai/claude-code"
    exit 1
fi

echo "🚀 Launching Claude Code for feature implementation..."
echo "Feature: $FEATURE"
echo ""
echo "Working directory: $SCRIPT_DIR"
echo "Press Ctrl+C to interrupt"
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# Run Claude Code with full permissions (no interactive prompts)
# --print mode keeps tool access without interactive confirmation dialogs
claude --permission-mode bypassPermissions --print "
You are implementing a feature for the APXM project, a Rust-based MLIR compiler and runtime for agent workflows.

## Project Structure
- crates/apxm-core: Core types and constants
- crates/apxm-credentials: Credential storage (~/.apxm/credentials.toml)
- crates/apxm-driver: Driver that configures runtime from config
- crates/apxm-runtime: Execution runtime with ModelRouter
- crates/apxm-backends: LLM backend implementations
- crates/apxm-cli: CLI (main.rs is 3400+ lines)

## Build Commands
- Build: dekk apxm build
- Test: dekk apxm test
- Test all: dekk apxm test-all

## Your Task
$FEATURE

## Key Files to Read First
1. docs/architecture/backend-implementation-spec.md — The full implementation spec
2. docs/architecture/backends-and-models.md — The conceptual hierarchy
3. crates/apxm-core/src/types/provider_spec.rs — ProviderProtocol enum
4. crates/apxm-credentials/src/lib.rs — Current credential store
5. crates/apxm-driver/src/runtime/llm.rs — How registry is configured
6. crates/apxm-cli/src/main.rs — CLI structure (search for LlmAction, ModelsAction)

## Rules
1. Read each file before editing
2. After each phase, run 'dekk apxm build' and fix errors
3. Run 'dekk apxm test' after build succeeds
4. Commit after each working phase with descriptive message
5. Do NOT break existing functionality
6. Keep backward compatibility with credentials.toml

Start by reading the implementation spec, then proceed phase by phase.
"

echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ Claude Code session complete"
