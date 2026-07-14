# Observed workflow optimization evaluation

This bundle preregisters the live backend evaluation for the workflow
optimization roadmap. The optimization and held-out case ids are disjoint, and
the baseline and candidate prompt arms are fixed before execution.

The task is contract classification. Each held-out input describes one compiler
or runtime situation and has one exact contract label. The baseline asks for a
brief classification without an output protocol. The candidate supplies the
typed workflow taxonomy and requires exactly one registered label. The exact
match metric therefore measures whether the prompt reliably produces the
machine-consumable contract result.

Run the bundle from `workspace/agents` through the registered backend surface:

```bash
dekk agents observed-prompt-evaluation \
  --preregistration evaluation/workflow-optimization-observed/preregistration.json \
  --optimization evaluation/workflow-optimization-observed/optimization.jsonl \
  --held-out evaluation/workflow-optimization-observed/held-out.jsonl \
  --baseline-arm evaluation/workflow-optimization-observed/baseline-arm.json \
  --candidate-arm evaluation/workflow-optimization-observed/candidate-arm.json \
  --backend amd
```

Generated requests, provider responses, execution records, receipts, and the
decision summary are written under
`.apxm/evaluation/workflow-optimization-observed/`. No credentials, configured
endpoint, or machine-local path is recorded.
