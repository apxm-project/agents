// The TypeScript conversational reference over generic Agent Program APIs.

import { Agent, Capability, Context, Hook, Model, Tool } from "@apxm/frontend";
import { COUNT_TOKENS, SEARCH_WEB } from "@apxm/frontend/capabilities";
import "@apxm/frontend/node";
import { staticSource } from "./static-source.js";

type ConversationInput = { message: string };
type ConversationOutput = { message: string };
type ConversationMessage = {
  role: "user" | "assistant" | "tool";
  content: string;
};
type CountTokensRequest = { messages: readonly ConversationMessage[] };
type CountTokensResult = { total: number };
type ConversationState = {
  messages: readonly ConversationMessage[];
  last_reply: string;
  context_budget: CountTokensResult | null;
  last_tool: string;
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

// Measuring the model-visible conversation is a Capability, not host code, so
// the measurement is an ordinary `capability.invoke` the compiler can see.
const CountTokens = Capability<CountTokensRequest, CountTokensResult>(
  COUNT_TOKENS,
);
const SearchWeb = Tool<SearchWebRequest, SearchWebResult>(SEARCH_WEB);
const SupportModel = Model<ModelRequest, ModelResponse>("model.target");
const ConversationContext = Context<ConversationState>(
  { messages: [], last_reply: "", context_budget: null, last_tool: "" },
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
  use: { CountTokens, SearchWeb, SupportModel },
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
        last_reply: response.reply.message,
        context_budget: agent.context.context_budget,
        last_tool: agent.context.last_tool,
      };
      incoming = await agent.yield_(response.reply);
    }
    throw new Error("missing conversation input");
  },
});

// Measure the model-visible conversation before the Tool runs.
const PrepareSearchContext = Hook.before<ConversationState>({
  agent: ConversationalExample,
  target: SearchWeb,
  scope: "capability",
  async run(agent) {
    const budget = await CountTokens({ messages: agent.context.messages });
    agent.context = {
      messages: agent.context.messages,
      last_reply: agent.context.last_reply,
      context_budget: budget,
      last_tool: agent.context.last_tool,
    };
  },
});

// Record which Capability the conversation last dispatched.
const RecordSearchContext = Hook.after<ConversationState>({
  agent: ConversationalExample,
  target: SearchWeb,
  scope: "capability",
  async run(agent) {
    agent.context = {
      messages: agent.context.messages,
      last_reply: agent.context.last_reply,
      context_budget: agent.context.context_budget,
      last_tool: "search_web",
    };
  },
});
void PrepareSearchContext;
void RecordSearchContext;

export function buildConversational(): ConversationalProgram {
  return ConversationalExample;
}
