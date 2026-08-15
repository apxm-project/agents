// Paired TypeScript source covering the complete canonical frontend seam.

import {
  Agent,
  Capability,
  Context,
  Event,
  Hook,
  Model,
  TaskGroup,
  Tool,
} from "@apxm/frontend";
import { COUNT_TOKENS, SEARCH_WEB } from "@apxm/frontend/capabilities";
import { source } from "@apxm/frontend/node";

source(import.meta.url);

type Input = unknown;
type Output = unknown;
type ParityContext = { iterations: number };
type ParityProgram = ReturnType<typeof Agent<Input, Output, ParityContext>>;

const ParityModel = Model<Input, Output>("parity.model");
const ParityTool = Tool<Input, Output>(SEARCH_WEB);
const ParityCapability = Capability<Input, Output>(COUNT_TOKENS);
const ParityEvent = Event<Output>("parity.event");
const ParityContext: ReturnType<typeof Context> = Context<ParityContext>({ iterations: 0 });

const ParityChild = Agent<Input, Output, ParityContext>({
  name: "ParityChild",
  context: ParityContext,
  async run(_agent, request) {
    return await ParityModel(request);
  },
});

export const ParityCorpus: ParityProgram = Agent<Input, Output, ParityContext>({
  name: "ParityCorpus",
  context: ParityContext,
  async run(agent, request) {
    while (true) {
      let toolResult: unknown = null;
      try {
        if (request !== null) {
          await TaskGroup.run(async () => {
            toolResult = await ParityTool(null);
          });
        } else {
          toolResult = await ParityCapability(null);
        }
      } catch {
        toolResult = await ParityCapability(null);
      }
      const child = ParityChild.new({ context: { iterations: 0 } });
      const childResult = await child.invoke(request);
      const eventResult = await ParityEvent.wait();
      const response = await ParityModel(request);
      agent.context = { iterations: agent.context.iterations };
      request = await agent.yield_(response);
    }
  },
});

const RecordParityModelStart = Hook.before<ParityContext>({
  agent: ParityCorpus,
  target: ParityModel,
  scope: "model",
  async run(agent) {
    void agent.context;
  },
});

export function buildParityCorpus(): ParityProgram {
  return ParityCorpus;
}
