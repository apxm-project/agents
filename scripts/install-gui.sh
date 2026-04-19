#!/bin/bash
set -e

# Prerequisite: at least one backend with at least one model
BACKEND_COUNT=$(apxm backend list --format json 2>/dev/null | python3 -c \
  "import sys,json; bs=json.load(sys.stdin); print(sum(1 for b in bs if b.get('models')))" 2>/dev/null || echo 0)

if [ "$BACKEND_COUNT" = "0" ]; then
  echo ""
  echo "ERROR: No backends with models configured."
  echo ""
  echo "The GUI needs at least one backend with models for chat."
  echo "Add one with:"
  echo ""
  echo "  apxm backend add <name> --protocol openai --endpoint <url>"
  echo "  apxm backend add-model <name> <model-id>"
  echo ""
  echo "Example:"
  echo "  apxm backend add my-provider --protocol openai --endpoint https://api.example.com"
  echo "  apxm backend add-model my-provider gpt-4.1"
  echo ""
  echo "Then re-run: dekk apxm install"
  exit 1
fi

# 1. Build GUI binary
cargo build -p apxm-gui --release

echo "GUI installed. Run: dekk apxm gui"
