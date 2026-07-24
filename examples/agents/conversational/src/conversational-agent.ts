// A conversational Agent authored on the installed typed frontend.
//
// A conversational Agent is an ordinary Agent: a typed input, an authored loop
// that calls a Model and an optional Tool, an explicit Context replacement, and a
// reply yielded before the next input. No conversation-specific runtime, turn
// type, or hidden loop is involved.

import { Agent, Context, Model, Tool } from "@apxm/frontend";
import "@apxm/frontend/node";
import { staticSource } from "./static-source.js";

type ConversationInput = { message: string; query: string };
type ConversationOutput = { message: string };
type ResearchContext = { requests: number };
type ConversationState = { messages: readonly string[] };
type ConversationalProgram = ReturnType<
  typeof Agent<ConversationInput, ConversationOutput, ConversationState>
>;

const SearchWeb = Tool<ConversationInput, string>("cap.search");
const SupportModel = Model<
  { incoming: ConversationInput; research: string },
  ConversationOutput
>("model.default");
const ResearchContext = Context<ResearchContext>({ requests: 0 });
const source = staticSource(import.meta.url);

const ResearchSpecialist = Agent<ConversationInput, string, ResearchContext>({
  name: "ResearchSpecialist",
  source,
  context: ResearchContext,
  use: { SearchWeb },
  async run(agent, incoming) {
    const research = await SearchWeb(incoming);
    agent.context = { requests: agent.context.requests + 1 };
    return research;
  },
});

const ConversationContext = Context<ConversationState>({ messages: [] });

export const ConversationalExample: ConversationalProgram = Agent<
  ConversationInput,
  ConversationOutput,
  ConversationState
>({
  name: "ConversationalExample",
  source,
  context: ConversationContext,
  use: { ResearchSpecialist, SupportModel },
  async run(agent, incoming) {
    while (true) {
      const specialist = ResearchSpecialist.new({ context: { requests: 0 } });
      const research = await specialist.invoke(incoming);
      const response = await SupportModel({ incoming, research });
      agent.context = {
        messages: [...agent.context.messages, incoming.message, response.message],
      };
      incoming = await agent.yield_(response);
    }
  },
});

export function buildConversational(): ConversationalProgram {
  return ConversationalExample;
}
