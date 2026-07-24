import { Agent } from "@apxm/frontend";

type Input = { value: string };
type Output = { value: string };

export const echoInput = Agent<Input, Output>({
  async run(agent, input) {
    return { value: input.value };
  },
});
