import { Agent, Workflow, Model, type Program } from "../src/index.ts";
import type { AgentAuthoring, WorkflowAuthoring, ProgramAuthoring, SsaExpression, TruthyPredicate } from "../src/generated/frontend-records.ts";

type Input = { reference: string };
type Output = { accepted: boolean };
type Context = { count: number };

const definition: Program<Input, Output, Context> = Workflow<Input, Output, Context>({
  name: "TypedWorkflow",
  async run(_agent, _input) { return { accepted: true }; },
});

const result: Promise<Output> = definition.invoke({ reference: "R-42" });
const instance = definition.new({ context: { count: 1 } });
const instanceResult: Promise<Output> = instance.invoke({ reference: "R-42" });
// @ts-expect-error The executable contract preserves its input type.
definition.invoke({ reference: 42 });
// @ts-expect-error Instance invocation preserves the same input type.
instance.invoke({ unknown: true });
// @ts-expect-error Context is not an untyped configuration bag.
definition.new({ context: { count: "one" } });
// @ts-expect-error The declared output cannot be widened into another shape.
const wrongOutput: Promise<{ accepted: string }> = result;
// @ts-expect-error The shared program type is not a raw graph factory.
Program<Input, Output>({});
// @ts-expect-error A Workflow requires its authored run callback.
Workflow<Input, Output>({});
// @ts-expect-error An Agent additionally requires an explicit typed Model.
Agent<Input, Output>({ async run() { return { accepted: true }; } });
const Primary = Model<Input, Output>("example.primary");
const modelBacked: Program<Input, Output> = Agent<Input, Output>({ model: Primary, async run(_agent, input) { return await Primary(input); } });
void modelBacked;
void instanceResult;
void wrongOutput;

// @ts-expect-error A branch's discriminator is its schema const, not the union.
const wrongWorkflow: WorkflowAuthoring = { kind: "agent" };
// @ts-expect-error Agent authoring requires its exact primary model reference.
const missingPrimary: ProgramAuthoring = { kind: "agent" };
// @ts-expect-error This record cannot carry another branch's discriminator.
const wrongAgent: AgentAuthoring = { kind: "workflow", primary_model_ref: "primary" };
// @ts-expect-error Const precision applies to all generated unions, not just authoring.
const wrongValue: SsaExpression = { kind: "string", value_id: "value.input" };
// @ts-expect-error A truthiness predicate cannot impersonate equality.
const wrongPredicate: TruthyPredicate = { comparator: "equals", root_value_id: "input", property_path: [] };
function primaryModel(authoring: ProgramAuthoring): string | undefined {
  return authoring.kind === "agent" ? authoring.primary_model_ref : undefined;
}
void [wrongWorkflow, missingPrimary, wrongAgent, wrongValue, wrongPredicate, primaryModel];
