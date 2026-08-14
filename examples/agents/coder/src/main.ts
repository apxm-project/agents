import { Agent, Model, Tool } from "@apxm/frontend";
import { source } from "@apxm/frontend/node";

import { proposeEdit } from "../capabilities/edit/handler.js";
import { prepareTest } from "../capabilities/test/handler.js";

source(import.meta.url);

type CoderInput = { file_path: string; task: string };
type EditProposal = { file_path: string; before: string; after: string };
type PreparedEdit = EditProposal & { mutates: false };
type EditDraft = { after: string; test_command: string };
type CoderOutput = { summary: string };

const ReadSource = Tool<{ file_path: string }, string>("read");
// The package-local capabilities are bound to the handlers that implement them,
// so the reference and the implementation are one object rather than two
// spellings of a name that have to agree.
const ProposeEdit = Tool<EditProposal, PreparedEdit>(proposeEdit);
const PrepareTest = Tool<{ command: string }, { command: string; executes: false; mutates: false }>(prepareTest);
const DraftEdit = Model<{ request: CoderInput; source: string }, EditDraft>("model.target");
const ReviewModel = Model<
  { request: CoderInput; source: string; proposal: PreparedEdit; test: { command: string } },
  CoderOutput
>("model.target");

export const Coder = Agent<CoderInput, CoderOutput>({
  name: "Coder",
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
