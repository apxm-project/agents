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
  toolCalls: number;
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
  { messages: [], toolCalls: 0 },
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
    while (true) {
      let turnMessages: readonly ConversationMessage[] = [
        { role: "user", content: incoming.message },
      ];
      let workingMessages: readonly ConversationMessage[] = [
        ...agent.context.messages,
        ...turnMessages,
      ];
      let response = await SupportModel({
        messages: workingMessages,
        incoming,
      });

      while (response.kind === "tool_request") {
        if (response.tool_request.kind === "search_web") {
          const toolResult = await SearchWeb(response.tool_request.arguments);
          turnMessages = [
            ...turnMessages,
            { role: "tool", content: toolResult.content },
          ];
          workingMessages = [...agent.context.messages, ...turnMessages];
          response = await SupportModel({
            messages: workingMessages,
            incoming,
            tool_result: toolResult,
          });
        } else {
          throw new Error("undeclared tool request");
        }
      }

      const finalReply = response.reply;
      agent.context = {
        messages: [
          ...workingMessages,
          { role: "assistant", content: finalReply.message },
        ],
        toolCalls: agent.context.toolCalls,
      };
      incoming = await agent.yield_(finalReply);
    }
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
      toolCalls: context.toolCalls,
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
      toolCalls: context.toolCalls + 1,
    };
  },
});
void PrepareSearchContext;
void RecordSearchContext;

export function buildConversational(): ConversationalProgram {
  return ConversationalExample;
}
