import { GraphBuilder } from "@apxm/frontend";

import {
  compact_conversation,
  gate_compose_workflow,
  inject_apxm_context,
  inject_context,
  redact_tool_results,
} from "./hooks.js";

export {
  compact_conversation,
  gate_compose_workflow,
  inject_apxm_context,
  inject_context,
  redact_tool_results,
};

const graph = new GraphBuilder("main", { metadata: { is_entry: true } });
graph.param("user_message", "str");

const loop = graph.autonomous({
  name: "turn_loop",
  prompt: "",
  mode: "recv",
  recv_once: "false",
  turn_agent: "gao",
  turn_flow: "turn",
  turn_param: "user_message",
  input_names: ["user_message"],
});

graph.done(loop, "return_turn");

console.log(graph.toAir());
