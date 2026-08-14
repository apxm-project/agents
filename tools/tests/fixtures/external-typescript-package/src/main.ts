import { Agent, Model, TaskGroup, Tool } from "@apxm/frontend";
import { source } from "@apxm/frontend/node";

source(import.meta.url);

type Input = { request: string };
type Output = { answer: string };

const Lookup = Tool<Input, { facts: string }>("fixture.lookup");
const Answer = Model<{ request: string }, Output>("fixture.answer");
type FixtureProgram = ReturnType<typeof Agent<Input, Output>>;

export const ExternalFixture: FixtureProgram = Agent<Input, Output>({
  name: "ExternalFixture",
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
