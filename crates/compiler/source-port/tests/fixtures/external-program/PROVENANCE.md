# External program fixture

`program.ts` is an immutable copy of a first-party Agent Program authored and
owned by another repository. It is conformance input, not source this repository
owns.

| | |
| --- | --- |
| Origin | `studio`, `agent-programs/gao/src/gao.ts` |
| Origin revision | `8eb435ec930e317418f8bd528f072ffe4b2bcf49` |
| Content digest | `sha256:56bc2101daae748510c47f4d2f7b86fc33f9c3157c37cd134c5a1041b407d7a9` |

Per `agents` ADR-0017, conformance "may compile the exact Studio-owned Gao
source bundle or an immutable fixture derived from it. Such a fixture proves
generic frontend and runtime behavior; it does not transfer source ownership to
Agents." A copy is used rather than a path into a sibling checkout so this
repository's tests depend on no checkout but their own.

The fixture is deliberately unmodified from its origin bytes. It is what an
external author actually committed, so compiling it proves the generic boundary
accepts real external source rather than source shaped to pass. If the origin
changes in a way that matters here, re-copy it and update the revision and
digest above — do not edit it in place, and do not adjust it to make a test
pass. A fixture edited to pass is a fixture that has stopped being evidence.

Nothing in this repository may branch on this program's name or product
semantics. The tests that consume it assert generic properties only.
