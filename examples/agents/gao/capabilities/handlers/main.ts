import { GraphBuilder } from "@apxm/frontend";

import {
  compact_conversation,
  gate_compose_workflow,
  redact_tool_results,
} from "./hooks.js";

export {
  compact_conversation,
  gate_compose_workflow,
  redact_tool_results,
};

const graph = new GraphBuilder("main", { metadata: { is_entry: true } });
const input = graph.awaitInput({
  name: "await_turn",
  waitKey: "gao.session",
  rearm: true,
});

graph.done(input, "return_turn");
