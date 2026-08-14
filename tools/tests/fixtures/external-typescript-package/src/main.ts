import { Agent, Model, TaskGroup, Tool } from "@apxm/frontend";
import "@apxm/frontend/node";
import { staticSource } from "./static-source.js";

type Input = { request: string };
type Output = { answer: string };

const Lookup = Tool<Input, { facts: string }>("fixture.lookup");
const Answer = Model<{ request: string }, Output>("fixture.answer");
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
      // `facts` is bound inside the TaskGroup scope, so it does not dominate
      // this call; passing it as an authored argument is rejected by the
      // forward-SSA-dominance rule. Mirrors the parity example's shape.
      const response = await Answer({ request: input.request });
      input = await agent.yield_(response);
    }
  },
});
