// The TypeScript conversational reference over generic Agent Program APIs.

import { Agent, Capability, Context, Hook, Model, Skill, Tool } from "@apxm/frontend";
import { COUNT_TOKENS, SEARCH_WEB } from "@apxm/frontend/capabilities";
import { Allow, Ask } from "@apxm/frontend/permissions";
import { CAPABILITY } from "@apxm/frontend/scopes";
import { source } from "@apxm/frontend/node";

source(import.meta.url);

type ConversationInput = { message: string };
type ConversationOutput = { message: string };
type ConversationMessage = {
  role: "user" | "assistant" | "tool";
  content: string;
};
type CountTokensRequest = { messages: readonly ConversationMessage[] };
type CountTokensResult = { total: number; over_budget: boolean };
type CompactionRequest = {
  messages: readonly ConversationMessage[];
  budget: CountTokensResult;
};
type CompactionResult = { messages: readonly ConversationMessage[] };
type ConversationContext = {
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
  persona: string;
  context_policy: string;
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
  typeof Agent<ConversationInput, ConversationOutput, ConversationContext>
>;

// Measuring the model-visible conversation is a Capability, not host code, so
// the measurement is an ordinary `capability.invoke` the compiler can see. The
// permission each binding carries is what the *program* asks for; a deployment
// may narrow it in `agent.toml [permissions]` and may never widen it.
const CountTokens = Capability<CountTokensRequest, CountTokensResult>(COUNT_TOKENS, {
  permission: Allow("Counts tokens in the conversation this program already holds."),
});
const SearchWeb = Tool<SearchWebRequest, SearchWebResult>(SEARCH_WEB, {
  permission: Ask("Sends a model-chosen query to a third-party search index."),
});
const SupportModel = Model<ModelRequest, ModelResponse>("model.target");

// The second declared Model. It is called from a Hook body, so compaction is
// workflow structure the compiler sees rather than host code behind a digest.
const CompactConversation = Model<CompactionRequest, CompactionResult>("model.compaction");

// The persona and the context policy are instructions, which is what a Skill
// is. Loading one is an ordinary `capability.invoke` on `read_skill`.
const PersonaSkill = Skill("persona", { entry: "skills/persona/SKILL.md" });
const ContextPolicySkill = Skill("context-policy", {
  entry: "skills/context-policy/SKILL.md",
});

const ConversationContext: ReturnType<typeof Context> = Context<ConversationContext>({
  messages: [],
  last_reply: "",
  context_budget: null,
  last_tool: "",
});

export const ConversationalExample: ConversationalProgram = Agent<
  ConversationInput,
  ConversationOutput,
  ConversationContext
>({
  name: "ConversationalExample",
  model: SupportModel,
  context: ConversationContext,
  async run(agent, incoming) {
    const persona = await PersonaSkill.load();
    const context_policy = await ContextPolicySkill.load();
    while (incoming.message !== "") {
      let response = await SupportModel({
        persona,
        context_policy,
        messages: agent.context.messages,
        incoming,
      });

      while (response.kind === "tool_request") {
        if (response.tool_request.kind === "search_web") {
          const toolResult = await SearchWeb(response.tool_request.arguments);
          response = await SupportModel({
            persona,
            context_policy,
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

// Measure the conversation and hand the measurement to the compactor.
const PrepareSearchContext = Hook.before<ConversationContext>({
  agent: ConversationalExample,
  target: SearchWeb,
  scope: CAPABILITY,
  async run(agent) {
    const budget = await CountTokens({ messages: agent.context.messages });
    const compacted = await CompactConversation({
      messages: agent.context.messages,
      budget,
    });
    agent.context = {
      messages: compacted.messages,
      last_reply: agent.context.last_reply,
      context_budget: budget,
      last_tool: agent.context.last_tool,
    };
  },
});

// Record which Capability the conversation last dispatched.
const RecordSearchContext = Hook.after<ConversationContext>({
  agent: ConversationalExample,
  target: SearchWeb,
  scope: CAPABILITY,
  async run(agent) {
    agent.context = {
      messages: agent.context.messages,
      last_reply: agent.context.last_reply,
      context_budget: agent.context.context_budget,
      last_tool: "search_web",
    };
  },
});

export function buildConversational(): ConversationalProgram {
  return ConversationalExample;
}
