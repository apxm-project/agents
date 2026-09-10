// Agent requires a primary Model; execution uses the shared typed Program.

import type { ModelBinding } from "./markers.js";
import { AGENT_DYNAMIC_ARGUMENT } from "./generated/diagnostics.js";
import { defineProgram, type ProgramConfig, type Program } from "./workflow.js";

/**
 * Everything an Agent states about itself.
 *
 * The typed interface is not here: it is `Agent<Input, Output, Context>`, read
 * back from the authored source. Neither are the declarations the body uses —
 * they are the ones the module declared — nor the source text, which the module
 * states once with `source(import.meta.url)`.
 */
type AgentConfig<Input, Output, Context> = ProgramConfig<Input, Output, Context> & {
  readonly model: ModelBinding<never, unknown>;
};

export function Agent<Input, Output, Context = undefined>(
  config: AgentConfig<Input, Output, Context>,
): Program<Input, Output, Context> {
  if (config.model?.kind !== "model_binding") {
    throw new Error(`${AGENT_DYNAMIC_ARGUMENT}: Agent requires an explicit typed Model binding`);
  }
  return defineProgram(config, "agent", config.model);
}
