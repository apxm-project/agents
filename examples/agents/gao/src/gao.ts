// Gao is a repository example: an ordinary Agent that calls a Model and a Tool
// and replies in a loop. It uses only the installed typed authoring surface — no
// conversation-specific runtime, turn type, or hidden loop.

import { Agent, Context, Event, Model, Tool } from "@apxm/frontend";

const GaoModel = Model("model.default");
const Search = Tool("cap.search");
const NextInput = Event("GaoInput");

type GaoState = { history: readonly string[] };

const GaoContext = Context<GaoState>({ history: [] });

export const Gao = Agent<unknown, unknown, GaoState>({
  name: "Gao",
  context: GaoContext,
  use: { GaoModel, Search, NextInput },
  async run(agent, incoming) {
    while (true) {
      const research = await Search(incoming);
      const reply = await GaoModel(incoming);
      agent.context = { history: [] };
      incoming = await NextInput.wait();
    }
  },
});

export function buildGao() {
  return Gao;
}
