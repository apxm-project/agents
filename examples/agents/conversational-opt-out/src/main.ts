import { Agent } from "@apxm/frontend";
import "@apxm/frontend/node";
import { staticSource } from "./static-source.js";

type Input = { value: string };
type Output = { value: string };

const source = staticSource(import.meta.url);

export const echoInput = Agent<Input, Output>({
  name: "EchoInput",
  source,
  async run(agent, input) {
    return { value: input.value };
  },
});
