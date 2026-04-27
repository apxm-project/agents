#!/bin/bash
# Mock ACP agent for integration testing.
# Reads NDJson on stdin, responds on stdout with ACP protocol messages.
set -e

while IFS= read -r line; do
  method=$(echo "$line" | jq -r '.method // empty')
  id=$(echo "$line" | jq -r '.id // empty')

  case "$method" in
    initialize)
      echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"protocolVersion\":\"2025-11-05\",\"agentCapabilities\":{}}}"
      ;;
    session/new)
      echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"sessionId\":\"mock-session-1\"}}"
      ;;
    session/prompt)
      # Send a streaming chunk notification
      echo "{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"text\":\"Hello from mock\"}}}}"
      # Send the final response
      echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"stopReason\":\"end_turn\"}}"
      ;;
    session/set_mode)
      echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{}}"
      ;;
    session/cancel)
      echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{}}"
      ;;
    *)
      echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32601,\"message\":\"Unknown method: $method\"}}"
      ;;
  esac
done
