// An Workflow Program that invokes the two Capabilities this package ships.
//
// `src/main.ts` is the agent: it reads a file, asks a model for a draft, and
// turns that draft into proposals. Every one of its Capability arguments comes
// from the program's own input or from a model response, so running it proves
// nothing about the handlers unless a model is bound and an input is supplied.
//
// This program isolates the half that is about the package's own
// implementations. It binds the same two handler declarations `src/main.ts`
// binds, with authored literal arguments, so executing it exercises exactly one
// thing: whether a `capability.invoke` for a Capability this package ships
// reaches the handler this package ships.
import { Workflow, Tool } from "@apxm/frontend";
import { source } from "@apxm/frontend/node";

import { proposeEdit } from "../capabilities/edit/handler.js";
import { prepareTest } from "../capabilities/test/handler.js";

source(import.meta.url);

type EditProposal = { file_path: string; before: string; after: string };
type PreparedEdit = EditProposal & { mutates: false };
type PreparedTest = { command: string; executes: false; mutates: false };
type NoInput = { unused?: never };

const ProposeEdit = Tool<EditProposal, PreparedEdit>(proposeEdit);
const PrepareTest = Tool<{ command: string }, PreparedTest>(prepareTest);

export const Proposals = Workflow<NoInput, PreparedTest>({
  name: "Proposals",
  async run() {
    await ProposeEdit({
      file_path: "src/main.ts",
      before: "const answer = 1;",
      after: "const answer = 2;",
    });
    return await PrepareTest({ command: "npm test" });
  },
});
