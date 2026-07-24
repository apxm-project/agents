// Gao is a repository example: an ordinary Agent that calls a Model and a Tool
// and replies in a loop. It uses only the installed typed authoring surface — no
// conversation-specific runtime, turn type, or hidden loop.

import { Agent, Context, Hook, Model, Tool } from "@apxm/frontend";
import "@apxm/frontend/node";
import { staticSource } from "./static-source.js";

type GaoInput = { request: string };
type GaoReply = { message: string };
type GaoState = { history: readonly string[] };
type DiscoveryState = { requests: number };
type GaoProgram = ReturnType<typeof Agent<GaoInput, GaoReply, GaoState>>;

const GaoModel = Model<{ incoming: GaoInput; research: string }, GaoReply>("model.default");
const Search = Tool<GaoInput, string>("cap.search");
const DiscoveryContext = Context<DiscoveryState>({ requests: 0 });
const source = staticSource(import.meta.url);

const WorkflowDiscovery = Agent<GaoInput, string, DiscoveryState>({
  name: "WorkflowDiscovery",
  source,
  context: DiscoveryContext,
  use: { Search },
  async run(agent, incoming) {
    const research = await Search(incoming);
    agent.context = { requests: agent.context.requests + 1 };
    return research;
  },
});

const GaoContext = Context<GaoState>({ history: [] });

export const Gao: GaoProgram = Agent<GaoInput, GaoReply, GaoState>({
  name: "Gao",
  source,
  context: GaoContext,
  use: { GaoModel, WorkflowDiscovery },
  async run(agent, incoming) {
    while (true) {
      const specialist = WorkflowDiscovery.new({ context: { requests: 0 } });
      const research = await specialist.invoke(incoming);
      const reply = await GaoModel({ incoming, research });
      agent.context = {
        history: [...agent.context.history, incoming.request, reply.message],
      };
      incoming = await agent.yield_(reply);
    }
  },
});

export const ObserveGaoModel: ReturnType<typeof Hook.after> = Hook.after({
  agent: Gao,
  target: GaoModel,
  scope: "model",
  async run(agent) {
    agent.context = agent.context;
  },
});

export function buildGao(): GaoProgram {
  return Gao;
}
