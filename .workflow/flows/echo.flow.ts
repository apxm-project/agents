/**
 * Echo Flow — workflow-test
 *
 * Simple test to verify agent spawning works.
 * Two sequential ACP nodes: one with codex, one with claude,
 * both answering the same question to compare outputs.
 *
 * Usage:
 *   acpx flow run ./echo.flow.ts \
 *     --input-json '{"question":"What is 2+2?"}' \
 *     --approve-all
 */
import { defineFlow, acp, compute } from "acpx/flows";

type EchoInput = {
  question: string;
};

export default defineFlow({
  name: "echo-test",
  startAt: "load_input",

  permissions: {
    requiredMode: "approve-all",
  },

  nodes: {
    load_input: compute({
      run: ({ input }) => {
        const { question } = input as EchoInput;
        return { question };
      },
    }),

    ask_codex: acp({
      profile: "codex",
      session: { handle: "codex-echo", isolated: true },
      statusDetail: "Asking Codex...",
      timeoutMs: 2 * 60_000,

      prompt: ({ outputs }) => {
        const { question } = outputs.load_input as { question: string };
        return `Answer this concisely in one line: ${question}`;
      },

      parse: (text) => ({ answer: text.trim() }),
    }),

    ask_claude: acp({
      profile: "claude",
      session: { handle: "claude-echo", isolated: true },
      statusDetail: "Asking Claude...",
      timeoutMs: 2 * 60_000,

      prompt: ({ outputs }) => {
        const { question } = outputs.load_input as { question: string };
        return `Answer this concisely in one line: ${question}`;
      },

      parse: (text) => ({ answer: text.trim() }),
    }),

    compare: compute({
      run: ({ outputs }) => {
        const codex = outputs.ask_codex as { answer: string };
        const claude = outputs.ask_claude as { answer: string };
        return {
          codex_answer: codex.answer,
          claude_answer: claude.answer,
          match: codex.answer.toLowerCase() === claude.answer.toLowerCase(),
        };
      },
    }),
  },

  edges: [
    { from: "load_input", to: "ask_codex" },
    { from: "ask_codex", to: "ask_claude" },
    { from: "ask_claude", to: "compare" },
  ],
});
