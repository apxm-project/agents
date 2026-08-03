import { Agent, Model, TaskGroup, Tool } from "@apxm/frontend";
import "@apxm/frontend/node";
import { staticSource } from "./static-source.js";

type Input = { request: string };
type Output = { answer: string };

const Lookup = Tool<Input, { facts: string }>("fixture.lookup.v1");
const Answer = Model<{ request: string; facts: string }, Output>("fixture.answer.v1");
const source = staticSource(import.meta.url);
type FixtureProgram = ReturnType<typeof Agent<Input, Output>>;

export const ExternalFixture: FixtureProgram = Agent<Input, Output>({
  name: "ExternalFixture",
  source,
  use: { Lookup, Answer },
  async run(agent, input) {
    while (true) {
      let facts = "";
      await TaskGroup.run(async () => {
        const lookup = await Lookup(input);
        facts = lookup.facts;
      });
      const response = await Answer({ request: input.request, facts });
      input = await agent.yield_(response);
    }
  },
});
