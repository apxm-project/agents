// The TypeScript conversational reference over generic Agent Program APIs.

import { Agent, Context, Hook, Model, Tool } from "@apxm/frontend";
import "@apxm/frontend/node";
import { staticSource } from "./static-source.js";

type ConversationInput = { message: string };
type ConversationOutput = { message: string };
type ConversationMessage = {
  role: "user" | "assistant" | "tool";
  content: string;
};
type ConversationState = {
  messages: readonly ConversationMessage[];
  tool_calls: number;
  last_reply: string;
};
type SearchWebRequest = { query: string };
type SearchWebResult = { content: string };
type SearchWebToolRequest = {
  kind: "search_web";
  arguments: SearchWebRequest;
};
type InitialModelRequest = {
  messages: readonly ConversationMessage[];
  incoming: ConversationInput;
};
type ToolResultModelRequest = InitialModelRequest & {
  tool_result: SearchWebResult;
};
type ModelRequest = InitialModelRequest | ToolResultModelRequest;
type ModelResponse =
  | { kind: "final"; reply: ConversationOutput }
  | { kind: "tool_request"; tool_request: SearchWebToolRequest };
type ConversationalProgram = ReturnType<
  typeof Agent<ConversationInput, ConversationOutput, ConversationState>
>;

const SearchWeb = Tool<SearchWebRequest, SearchWebResult>("cap.search");
const SupportModel = Model<ModelRequest, ModelResponse>("model.target.v1");
const ConversationContext = Context<ConversationState>(
  { messages: [], tool_calls: 0, last_reply: "" },
  "ConversationContext",
);
const source = staticSource(import.meta.url);

export const ConversationalExample: ConversationalProgram = Agent<
  ConversationInput,
  ConversationOutput,
  ConversationState
>({
  name: "ConversationalExample",
  input: "ConversationInput",
  output: "ConversationOutput",
  source,
  context: ConversationContext,
  use: { SearchWeb, SupportModel },
  async run(agent, incoming) {
    while (incoming.message !== "") {
      let response = await SupportModel({
        messages: agent.context.messages,
        incoming,
      });

      while (response.kind === "tool_request") {
        if (response.tool_request.kind === "search_web") {
          const toolResult = await SearchWeb(response.tool_request.arguments);
          response = await SupportModel({
            messages: agent.context.messages,
            incoming,
            tool_result: toolResult,
          });
        } else {
          throw new Error("undeclared tool request");
        }
      }

      if (response.kind !== "final") {
        throw new Error("undeclared model response");
      }

      agent.context = {
        messages: agent.context.messages,
        tool_calls: agent.context.tool_calls,
        last_reply: response.reply.message,
      };
      incoming = await agent.yield_(response.reply);
    }
    throw new Error("missing conversation input");
  },
});

const PrepareSearchContext = Hook.before({
  agent: ConversationalExample,
  target: SearchWeb,
  scope: "capability",
  async run(agent) {
    const context = agent.context as ConversationState;
    agent.context = {
      messages: context.messages.slice(-24),
      tool_calls: context.tool_calls,
      last_reply: context.last_reply,
    };
  },
});

const RecordSearchContext = Hook.after({
  agent: ConversationalExample,
  target: SearchWeb,
  scope: "capability",
  async run(agent) {
    const context = agent.context as ConversationState;
    agent.context = {
      messages: context.messages,
      tool_calls: context.tool_calls + 1,
      last_reply: context.last_reply,
    };
  },
});
void PrepareSearchContext;
void RecordSearchContext;

export function buildConversational(): ConversationalProgram {
  return ConversationalExample;
}
