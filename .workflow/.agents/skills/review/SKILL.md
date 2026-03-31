---
name: review
description: Review code changes for quality, security, and correctness
user-invocable: true
---

# Code Review Guidelines

When reviewing code changes:

1. **Correctness** — Does the code do what it claims? Are there logic errors?
2. **Security** — Check for buffer overflows, injection, unsafe operations, hardcoded secrets
3. **Error handling** — Are errors caught and handled appropriately?
4. **Style** — Does the code follow project conventions and be consistent?
5. **Performance** — Any obvious performance regressions or unnecessary allocations?
6. **Tests** — Are there adequate tests for the changes?

Output structured JSON with your verdict, summary, and specific comments.
