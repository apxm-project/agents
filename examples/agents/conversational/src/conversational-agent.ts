// A model-directed conversational Agent Program on the typed frontend.

import { Agent, Context, Hook, Model, Tool } from "@apxm/frontend";
import "@apxm/frontend/node";
import { staticSource } from "./static-source.js";

type ConversationInput = { message: string };
type ConversationOutput = { message: string };
type ToolKind = "search_web" | "read_page";
type ToolRequest = { kind: ToolKind; arguments: string };
type ModelRequest = {
  messages: readonly unknown[];
  policy: string;
  availableTools: readonly ToolKind[];
};
type ModelResponse = {
  message: string;
  toolRequest: ToolRequest | null;
};
type SearchRequest = { query: string };
type SearchResult = { content: string };
type ReadPageRequest = { uri: string };
type ReadPageResult = { content: string };
type ConversationState = {
  messages: readonly unknown[];
  modelWindow: readonly unknown[];
  activeInput: ConversationInput | null;
  policy: string;
  modelCalls: number;
  toolCalls: number;
};
type ConversationalProgram = ReturnType<
  typeof Agent<ConversationInput, ConversationOutput, ConversationState>
>;

const SearchWeb = Tool<SearchRequest, SearchResult>("cap.search");
const ReadPage = Tool<ReadPageRequest, ReadPageResult>("cap.read");
const SupportModel = Model<ModelRequest, ModelResponse>("model.target.v1");
const ConversationContext = Context<ConversationState>(
  {
    messages: [],
    modelWindow: [],
    activeInput: null,
    policy: "grounded-support-v1",
    modelCalls: 0,
    toolCalls: 0,
  },
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
  use: { ReadPage, SearchWeb, SupportModel },
  async run(agent, incoming) {
    while (true) {
      if (agent.context.activeInput === null) {
        agent.context = {
          ...agent.context,
          messages: [...agent.context.messages, incoming],
          modelWindow: [...agent.context.modelWindow, incoming],
          activeInput: incoming,
        };
      }

      const response = await SupportModel({
        messages: agent.context.modelWindow,
        policy: agent.context.policy,
        availableTools: ["search_web", "read_page"],
      });

      if (response.toolRequest !== null) {
        let toolResult: SearchResult | ReadPageResult;
        if (response.toolRequest.kind === "search_web") {
          toolResult = await SearchWeb({ query: response.toolRequest.arguments });
        } else {
          toolResult = await ReadPage({ uri: response.toolRequest.arguments });
        }
        agent.context = {
          ...agent.context,
          messages: [...agent.context.messages, response, toolResult],
          modelWindow: [...agent.context.modelWindow, response, toolResult],
        };
      } else {
        agent.context = {
          ...agent.context,
          messages: [...agent.context.messages, response],
          modelWindow: [...agent.context.modelWindow, response],
          activeInput: null,
        };
        incoming = await agent.yield_({ message: response.message });
      }
    }
  },
});

const PrepareModelContext = Hook.before({
  agent: ConversationalExample,
  target: SupportModel,
  scope: "model",
  async run(agent) {
    agent.context = {
      ...agent.context,
      modelWindow: agent.context.modelWindow.slice(-24),
      policy: "grounded-support-v1",
    };
  },
});

const CountModelCall = Hook.after({
  agent: ConversationalExample,
  target: SupportModel,
  scope: "model",
  async run(agent) {
    agent.context = {
      ...agent.context,
      modelCalls: agent.context.modelCalls + 1,
    };
  },
});

const CountWebSearch = Hook.after({
  agent: ConversationalExample,
  target: SearchWeb,
  scope: "capability",
  async run(agent) {
    agent.context = {
      ...agent.context,
      toolCalls: agent.context.toolCalls + 1,
    };
  },
});

const CountPageRead = Hook.after({
  agent: ConversationalExample,
  target: ReadPage,
  scope: "capability",
  async run(agent) {
    agent.context = {
      ...agent.context,
      toolCalls: agent.context.toolCalls + 1,
    };
  },
});

void PrepareModelContext;
void CountModelCall;
void CountWebSearch;
void CountPageRead;

export function buildConversational(): ConversationalProgram {
  return ConversationalExample;
}
