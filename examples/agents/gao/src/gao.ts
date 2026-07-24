// Gao is the APXM-capability extension of the conversational reference. It
// uses only installed generic authoring APIs: discover capabilities, plan a
// workflow, prepare validation, ask a Model, and yield the reply.

import { Agent, Context, Hook, Model, Tool } from "@apxm/frontend";
import "@apxm/frontend/node";
import { staticSource } from "./static-source.js";

type GaoInput = { request: string };
type GaoReply = { message: string };
type GaoState = { history: readonly string[] };
type GaoProgram = ReturnType<typeof Agent<GaoInput, GaoReply, GaoState>>;
type CapabilityCatalog = { summary: string };
type WorkflowPlan = { plan: string; next_action: "review_plan" };
type ValidationRequest = {
  validation_request: string;
  next_action: "submit_for_validation";
};

const DiscoverCapabilities = Tool<GaoInput, CapabilityCatalog>("capability_discovery");
const PlanWorkflow = Tool<{ request: string; catalog: string }, WorkflowPlan>("plan_workflow");
const PrepareValidation = Tool<{ request: string; plan: string }, ValidationRequest>("prepare_validation");
const GaoModel = Model<
  { incoming: GaoInput; catalog: CapabilityCatalog; plan: WorkflowPlan; validation: ValidationRequest },
  GaoReply
>("model.default");
const source = staticSource(import.meta.url);

const GaoContext = Context<GaoState>({ history: [] });

export const Gao: GaoProgram = Agent<GaoInput, GaoReply, GaoState>({
  name: "Gao",
  source,
  context: GaoContext,
  use: { DiscoverCapabilities, PlanWorkflow, PrepareValidation, GaoModel },
  async run(agent, incoming) {
    while (true) {
      const catalog = await DiscoverCapabilities(incoming);
      const plan = await PlanWorkflow({ request: incoming.request, catalog: catalog.summary });
      const validation = await PrepareValidation({ request: incoming.request, plan: plan.plan });
      const reply = await GaoModel({ incoming, catalog, plan, validation });
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
