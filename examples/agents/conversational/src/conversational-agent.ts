// A conversational Agent authored on the installed typed frontend.
//
// A conversational Agent is an ordinary Agent: a typed input, an authored loop
// that calls a Model and an optional Tool, an explicit Context replacement, and a
// reply yielded before the next input. No conversation-specific runtime, turn
// type, or hidden loop is involved.

import { Agent, Context, Event, Model, Tool } from "@apxm/frontend";

const SearchWeb = Tool("cap.search");
const SupportModel = Model("model.default");
const NextInput = Event("ConversationInput");

type ConversationState = { messages: readonly string[] };

const ConversationContext = Context<ConversationState>({ messages: [] });

export const ConversationalExample = Agent<unknown, unknown, ConversationState>({
  name: "ConversationalExample",
  context: ConversationContext,
  use: { SearchWeb, SupportModel, NextInput },
  async run(agent, incoming) {
    while (true) {
      const research = await SearchWeb(incoming);
      const response = await SupportModel(incoming);
      agent.context = { messages: [] };
      incoming = await NextInput.wait();
    }
  },
});

export function buildConversational() {
  return ConversationalExample;
}
