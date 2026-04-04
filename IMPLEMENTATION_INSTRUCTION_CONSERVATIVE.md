# Conservative implementation instruction

Do not implement anything until the concrete task and repository context are provided.

Required inputs before making any code change:
- The exact value of `{{TASK}}`
- The repository tree or relevant source file list
- The current interfaces and locations for any referenced components, including:
  - `ContextStack`
  - `MemoCache`
  - `Scheduler`
- Expected behavior and acceptance criteria
- Ideally, the three actual analyses referenced by the synthesis

Constraints:
- Do not invent APIs, file paths, module names, or tests
- Do not create speculative implementations
- Do not modify existing Rust files without grounded task details
- Limit future scope to the smallest safe change set once the missing inputs are available

Current action:
- Stop here and request the missing task and repository context from the user
