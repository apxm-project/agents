import { Agent } from "@apxm/frontend";
import "@apxm/frontend/node";
import { staticSource } from "./static-source.js";

type CoderInput = { task: string };
type CoderOutput = { task: string };

const source = staticSource(import.meta.url);

export const Coder = Agent<CoderInput, CoderOutput>({
  name: "Coder",
  source,
  async run(agent, input) {
    return { task: input.task };
  },
});
