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
import "@apxm/frontend/node";
import { staticSource } from "./static-source.js";

type ParityContext = { iterations: number };
type ParityProgram = ReturnType<typeof Agent<unknown, unknown, ParityContext>>;

const ParityModel = Model<unknown, unknown>("parity.model.v1");
const ParityTool = Tool<unknown, unknown>("parity.tool.v1");
const ParityCapability = Capability<unknown, unknown>("parity.capability.v1");
const ParityEvent = Event<unknown>("parity.event.v1");
const ParityContext: ReturnType<typeof Context> = Context<ParityContext>(
  { iterations: 0 },
  "ParityContext",
);
const source = staticSource(import.meta.url);

const ParityChild = Agent<unknown, unknown, ParityContext>({
  name: "ParityChild",
  input: "Input",
  output: "Output",
  source,
  context: ParityContext,
  use: { ParityModel },
  async run(_agent, request) {
    return await ParityModel(request);
  },
});

export const ParityCorpus: ParityProgram = Agent<unknown, unknown, ParityContext>({
  name: "ParityCorpus",
  input: "Input",
  output: "Output",
  source,
  context: ParityContext,
  use: {
    ParityCapability,
    ParityChild,
    ParityEvent,
    ParityModel,
    ParityTool,
  },
  async run(agent, request) {
    while (true) {
      let toolResult: unknown = null;
      try {
        if (request !== null) {
          await TaskGroup.run(async () => {
            toolResult = await ParityTool(request);
          });
        } else {
          toolResult = await ParityCapability(request);
        }
      } catch {
        toolResult = await ParityCapability(request);
      }
      const child = ParityChild.new({ context: { iterations: 0 } });
      const childResult = await child.invoke(request);
      const eventResult = await ParityEvent.wait();
      const response = await ParityModel(request);
      agent.context = { iterations: agent.context.iterations + 1 };
      request = await agent.yield_(response);
    }
  },
});

const RecordParityModelStart = Hook.before({
  agent: ParityCorpus,
  target: ParityModel,
  scope: "model",
  async run(agent) {
    void agent.context;
  },
});
void RecordParityModelStart;

export function buildParityCorpus(): ParityProgram {
  return ParityCorpus;
}
