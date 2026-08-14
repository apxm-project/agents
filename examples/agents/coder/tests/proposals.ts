// An Agent Program that invokes the two Capabilities this package ships.
//
// `src/main.ts` is the agent: it reads a file, asks a model for a draft, and
// turns that draft into proposals. Every one of its Capability arguments comes
// from the program's own input or from a model response, so running it proves
// nothing about the handlers unless a model is bound and an input is supplied.
//
// This program isolates the half that is about the package's own
// implementations. It names the same two Capability references `src/main.ts`
// names, with authored literal arguments, so executing it exercises exactly one
// thing: whether a `capability.invoke` for a Capability this package ships
// reaches the handler this package ships.
import { Agent, Tool } from "@apxm/frontend";
import { source } from "@apxm/frontend/node";

source(import.meta.url);

type EditProposal = { file_path: string; before: string; after: string };
type PreparedEdit = EditProposal & { mutates: false };
type PreparedTest = { command: string; executes: false; mutates: false };
type NoInput = { unused?: never };

const ProposeEdit = Tool<EditProposal, PreparedEdit>("edit");
const PrepareTest = Tool<{ command: string }, PreparedTest>("test");

export const Proposals = Agent<NoInput, PreparedTest>({
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
