import { Agent, Model, Tool } from "@apxm/frontend";
import "@apxm/frontend/node";
import { staticSource } from "./static-source.js";

type CoderInput = { file_path: string; task: string };
type EditProposal = { file_path: string; before: string; after: string };
type PreparedEdit = EditProposal & { mutates: false };
type EditDraft = { after: string; test_command: string };
type CoderOutput = { summary: string };

const source = staticSource(import.meta.url);
const ReadSource = Tool<{ file_path: string }, string>("read");
const ProposeEdit = Tool<EditProposal, PreparedEdit>("edit");
const PrepareTest = Tool<{ command: string }, { command: string; executes: false; mutates: false }>("test");
const DraftEdit = Model<{ request: CoderInput; source: string }, EditDraft>("model.target.v1");
const ReviewModel = Model<
  { request: CoderInput; source: string; proposal: PreparedEdit; test: { command: string } },
  CoderOutput
>("model.target.v1");

export const Coder = Agent<CoderInput, CoderOutput>({
  name: "Coder",
  source,
  use: { ReadSource, ProposeEdit, PrepareTest, DraftEdit, ReviewModel },
  async run(agent, input) {
    const currentSource = await ReadSource({ file_path: input.file_path });
    const draft = await DraftEdit({ request: input, source: currentSource });
    const proposal = await ProposeEdit({
      file_path: input.file_path,
      before: currentSource,
      after: draft.after,
    });
    const test = await PrepareTest({ command: draft.test_command });
    return await ReviewModel({ request: input, source: currentSource, proposal, test });
  },
});
