import { Agent, Model, Tool } from "@apxm/frontend";
import "@apxm/frontend/node";

import { staticSource } from "apxm:source";

type Input = { request: string };
type Output = { answer: string };

const Lookup = Tool<Input, { facts: string }>("fixture.lookup.v1");
const Answer = Model<{ request: string; facts: string }, Output>("fixture.answer.v1");
const source = staticSource();
type FixtureProgram = ReturnType<typeof Agent<Input, Output>>;

export const ExternalFixture: FixtureProgram = Agent<Input, Output>({
  name: "ExternalFixture",
  source,
  use: { Lookup, Answer },
  async run(agent, input) {
    while (true) {
      const facts = await Lookup(input);
      const response = await Answer({ request: input.request, facts: facts.facts });
      input = await agent.yield_(response);
    }
  },
});
