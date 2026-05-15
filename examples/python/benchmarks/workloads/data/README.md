# Vendored workload data

Small, tracked samples of external workload datasets used by the
comparable-workloads suite. Full datasets are downloaded on demand into
`.apxm/datasets/` (gitignored) via the corresponding fetch scripts.

## Mooncake conversation trace

**File:** `mooncake_sample.jsonl`
**Size:** 128,585 bytes (500 rows)
**SHA256:** `e29d6346a8bdc0148d180c94a9482fc5a8eec41e54be7623fbe25f463dbb5e42`

**Upstream:** `kvcache-ai/Mooncake`, file
`FAST25-release/traces/conversation_trace.jsonl`.
**Upstream commit at sample time:** `b0ae4a727fece46a6bbb80b03ef4478828214071`
(committed 2026-05-13).
**License:** Apache 2.0.

**Sampling strategy:** stride sample (every 24th row) over the first
12,031 complete rows of the upstream file. Stride was chosen as
`floor(population / 500)` to give 500 rows with even temporal coverage
across the workload's lifetime.

**Distribution preservation** (full first-12K population vs 500-row sample):

| metric | population p50 | sample p50 | drift |
|---|---|---|---|
| `input_length` (tokens) | 6909 | 6758 | -2.2 % |
| `output_length` (tokens) | 350 | 361 | +3.1 % |
| `len(hash_ids)` (blocks) | 14 | 14 | 0 |

**Row schema** (from upstream FAST'25 release):
```json
{"timestamp": <ms>, "input_length": <prefill_tokens>,
 "output_length": <decode_tokens>, "hash_ids": [<block_hash>, ...]}
```

`hash_ids` is the prefix-cache block sequence; rows in the same prefix
cohort share a `hash_ids` prefix. The trace does **not** carry prompt
text (privacy-redacted upstream); replay drivers synthesize prompts
from `hash_ids` via `_mooncake_hash.synthesize_prompt`.

**Regenerate:**

```bash
bash examples/python/benchmarks/workloads/data/fetch_mooncake.sh
# Then re-run the stride sampler (see below).
```

## Regeneration recipe (Mooncake)

```python
import json, statistics, hashlib
src = ".apxm/datasets/mooncake/conversation_trace.jsonl"
rows = [json.loads(line) for line in open(src) if line.strip()]
TARGET = 500
stride = max(1, len(rows) // TARGET)
sample = rows[::stride][:TARGET]
out = "examples/python/benchmarks/workloads/data/mooncake_sample.jsonl"
with open(out, "w") as f:
    for r in sample:
        f.write(json.dumps(r) + "\n")
print(hashlib.sha256(open(out, "rb").read()).hexdigest())
```

If the upstream trace changes, update the SHA256 and the upstream
commit reference above in the same commit.
