import { Agent } from "@apxm/frontend";

type CoderInput = { task: string };
type CoderOutput = { task: string };

export const Coder = Agent<CoderInput, CoderOutput>({
  async run(agent, input) {
    return { task: input.task };
  },
});
