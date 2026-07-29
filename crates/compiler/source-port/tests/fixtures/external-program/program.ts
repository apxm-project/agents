import { Agent, Context, Model, Tool } from "@apxm/frontend";
import "@apxm/frontend/node";

import { staticSource } from "apxm:source";

type TypedRef = { ref_type: string; ref: string };
type GaoWorkflowContext = {
  workflow_ref: TypedRef;
  draft_ref: TypedRef;
  expected_revision: number;
};
type GaoInput = {
  operator_request: string;
  surface: "studio_global" | "studio_workflow" | "embed_gao_assistant";
  workflow_context?: GaoWorkflowContext;
};
type GaoOutput = {
  status: "answered" | "approval_required" | "committed" | "rejected" | "outcome_unknown";
  message: string;
  committed_revision?: number;
  evidence_ref?: TypedRef;
};
type GaoState = { completed_requests: number };
type GaoProgram = ReturnType<typeof Agent<GaoInput, GaoOutput, GaoState>>;
type KnowledgeSummary = { document_ref: TypedRef; document_digest: string; title: string };
type KnowledgeSearchResult = { documents: readonly KnowledgeSummary[] };
type KnowledgeDocument = KnowledgeSummary & { body: string };
type WorkflowSummary = { workflow_ref: TypedRef; draft_ref?: TypedRef; revision?: number; title: string };
type WorkflowListResult = { workflows: readonly WorkflowSummary[] };
type WorkflowReadResult = WorkflowSummary & { source: string; frontend: "python" | "typescript"; entrypoint: string };
type WorkflowActionsResult = { actions: readonly string[] };
type MutationReceipt = {
  outcome: "committed" | "rejected" | "outcome_unknown";
  message: string;
  committed_revision?: number;
  evidence_ref?: TypedRef;
};

type GaoDecision =
  | { action: "answer"; message: string }
  | { action: "create"; frontend: "python" | "typescript"; entrypoint: string; source: string }
  | { action: "source_update"; workflow: GaoWorkflowContext; replacement: string }
  | { action: "node_insert"; workflow: GaoWorkflowContext; source_annotation_id: string; replacement: string }
  | { action: "node_edit"; workflow: GaoWorkflowContext; source_annotation_id: string; replacement: string }
  | { action: "node_delete"; workflow: GaoWorkflowContext; source_annotation_id: string }
  | { action: "node_move"; workflow: GaoWorkflowContext; source_annotation_id: string; x: number; y: number }
  | { action: "edge_connect"; workflow: GaoWorkflowContext; source_annotation_id: string; target_annotation_id: string; replacement: string }
  | { action: "edge_disconnect"; workflow: GaoWorkflowContext; source_annotation_id: string; target_annotation_id: string; replacement: string };

const KnowledgeSearch = Tool<{ query: string; limit: number }, KnowledgeSearchResult>("studio.apxm.knowledge.search");
const KnowledgeRead = Tool<{ document_ref: TypedRef; document_digest: string }, KnowledgeDocument>("studio.apxm.knowledge.read");
const WorkflowList = Tool<{ limit: number }, WorkflowListResult>("studio.workflow.list");
const WorkflowRead = Tool<GaoWorkflowContext, WorkflowReadResult>("studio.workflow.read");
const WorkflowActions = Tool<GaoWorkflowContext & { surface: GaoInput["surface"] }, WorkflowActionsResult>("studio.workflow.actions.list");
const WorkflowCreate = Tool<Extract<GaoDecision, { action: "create" }>, MutationReceipt>("studio.workflow.create");
const WorkflowSourceUpdate = Tool<Extract<GaoDecision, { action: "source_update" }>, MutationReceipt>("studio.workflow.source.update");
const WorkflowNodeInsert = Tool<Extract<GaoDecision, { action: "node_insert" }>, MutationReceipt>("studio.workflow.node.insert");
const WorkflowNodeEdit = Tool<Extract<GaoDecision, { action: "node_edit" }>, MutationReceipt>("studio.workflow.node.edit");
const WorkflowNodeDelete = Tool<Extract<GaoDecision, { action: "node_delete" }>, MutationReceipt>("studio.workflow.node.delete");
const WorkflowNodeMove = Tool<Extract<GaoDecision, { action: "node_move" }>, MutationReceipt>("studio.workflow.node.move");
const WorkflowEdgeConnect = Tool<Extract<GaoDecision, { action: "edge_connect" }>, MutationReceipt>("studio.workflow.edge.connect");
const WorkflowEdgeDisconnect = Tool<Extract<GaoDecision, { action: "edge_disconnect" }>, MutationReceipt>("studio.workflow.edge.disconnect");

const GaoModel = Model<
  {
    request: GaoInput;
    knowledge: KnowledgeDocument;
    workflows: WorkflowListResult;
    workflow?: WorkflowReadResult;
    actions?: WorkflowActionsResult;
  },
  GaoDecision
>("model-target:sha256:1111111111111111111111111111111111111111111111111111111111111111");

const GaoContext = Context<GaoState>({ completed_requests: 0 }, "StudioGaoContext");
const source = staticSource();

function outputFromReceipt(receipt: MutationReceipt): GaoOutput {
  return {
    status: receipt.outcome,
    message: receipt.message,
    ...(receipt.committed_revision === undefined ? {} : { committed_revision: receipt.committed_revision }),
    ...(receipt.evidence_ref === undefined ? {} : { evidence_ref: receipt.evidence_ref }),
  };
}

export const Gao: GaoProgram = Agent<GaoInput, GaoOutput, GaoState>({
  name: "Gao",
  source,
  context: GaoContext,
  use: {
    KnowledgeSearch,
    KnowledgeRead,
    WorkflowList,
    WorkflowRead,
    WorkflowActions,
    WorkflowCreate,
    WorkflowSourceUpdate,
    WorkflowNodeInsert,
    WorkflowNodeEdit,
    WorkflowNodeDelete,
    WorkflowNodeMove,
    WorkflowEdgeConnect,
    WorkflowEdgeDisconnect,
    GaoModel,
  },
  async run(agent, incoming) {
    while (true) {
      const matches = await KnowledgeSearch({ query: incoming.operator_request, limit: 8 });
      const first = matches.documents[0];
      const knowledge = await KnowledgeRead({
        document_ref: first.document_ref,
        document_digest: first.document_digest,
      });
      const workflows = await WorkflowList({ limit: 100 });
      const workflow = incoming.workflow_context
        ? await WorkflowRead(incoming.workflow_context)
        : undefined;
      const actions = incoming.workflow_context
        ? await WorkflowActions({ ...incoming.workflow_context, surface: incoming.surface })
        : undefined;
      const decision = await GaoModel({ request: incoming, knowledge, workflows, workflow, actions });

      let output: GaoOutput;
      switch (decision.action) {
        case "answer":
          output = { status: "answered", message: decision.message };
          break;
        case "create":
          output = outputFromReceipt(await WorkflowCreate(decision));
          break;
        case "source_update":
          output = outputFromReceipt(await WorkflowSourceUpdate(decision));
          break;
        case "node_insert":
          output = outputFromReceipt(await WorkflowNodeInsert(decision));
          break;
        case "node_edit":
          output = outputFromReceipt(await WorkflowNodeEdit(decision));
          break;
        case "node_delete":
          output = outputFromReceipt(await WorkflowNodeDelete(decision));
          break;
        case "node_move":
          output = outputFromReceipt(await WorkflowNodeMove(decision));
          break;
        case "edge_connect":
          output = outputFromReceipt(await WorkflowEdgeConnect(decision));
          break;
        case "edge_disconnect":
          output = outputFromReceipt(await WorkflowEdgeDisconnect(decision));
          break;
      }
      agent.context = { completed_requests: agent.context.completed_requests + 1 };
      incoming = await agent.yield_(output);
    }
  },
});
