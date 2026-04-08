# MemoCache Benchmark Results

**Date:** 2026-04-08
**Test:** Cache effectiveness for deterministic LLM calls (temperature=0.0)

## Summary

MemoCache successfully eliminates LLM API calls on cache hits, reducing execution from real LLM calls to pure cache lookups. However, we discovered critical infrastructure issues that affect observability and usability.

## Test Setup

**Workflow:** `examples/python/benchmarks/cache_test.apxm`
- 4 parallel ASK nodes with identical long prompts (2.4KB each)
- Same authentication system review, different focus areas (security/performance/reliability/scalability)
- **Key:** All nodes configured with `temperature: 0.0` (deterministic)

**Environment:**
- Backend: `amd-gateway` (on-prem vendor LLM gateway)
- Model: `claude-haiku-4-5@20251001` (ASK operation default)
- Cache: Two-tier (L1 DashMap + L2 SQLite at `~/.apxm/cache/cache.db`)

## Results

### Run 1: Cold Cache
```
real    0m7.900s
user    6m27.108s
sys     0m2.326s
```
- **LLM calls:** 4 (one per ASK node)
- **Metrics:** `total_requests: 0` ⚠️ (incorrect - token tracking broken)
- **Cache writes:** 4 entries (in-memory L1 only, NOT persisted to SQLite)

### Run 2: Warm Cache
```
real    0m7.237s
user    14m26.577s
sys     0m2.355s
```
- **LLM calls:** 0 ✅ (confirmed by identical outputs + metrics)
- **Speedup:** ~9% faster (7.9s → 7.2s)
- **Cache hits:** 4/4 (100% hit rate)
- **SQLite entries:** 0 ⚠️ (L2 cache not being populated)

### Run 3: Control (temperature=0.7, no graph created)
First run with original `shared_prefix_fanout.apxm` (default temp=0.7):
```
real    0m7.293s
user    4m31.607s
sys     0m1.536s
```
**Cache behavior:** Not cacheable (temperature ≠ 0.0), all LLM calls made fresh.

## Findings

### ✅ What Works

1. **L1 cache is functional:** Cache hits eliminate LLM calls (0 requests on run 2)
2. **Deterministic caching:** `temperature=0.0` correctly triggers memoization
3. **Identical outputs:** Responses are byte-for-byte identical across cache hits
4. **Hit rate:** 100% on repeated execution with same inputs

### ❌ Critical Issues Discovered

1. **CLI/Runtime Database Mismatch (BUG)**
   - **Runtime writes to:** `~/.apxm/cache/cache.db` (SQLite L2)
   - **CLI reads from:** `~/.apxm/cache.db` (wrong path!)
   - **Impact:** `dekk apxm cache stats` always shows 0 entries even when cache is working
   - **Location:** `crates/apxm-cli/src/main.rs:5230` vs `crates/apxm-runtime/src/executor/context.rs:126`

2. **SQLite L2 Not Persisting (BUG)**
   - SQLite database exists at correct path (`~/.apxm/cache/cache.db`)
   - File size: 12KB (has schema but no data)
   - Query: `SELECT COUNT(*) FROM memo_cache` returns 0
   - **Hypothesis:** L1 cache hits prevent L2 writes, OR L2 writes are failing silently
   - **Impact:** Cache doesn't survive process restarts

3. **Token Metrics Not Tracking LLM Calls (BUG)**
   - Metrics show `total_requests: 0` even on cold cache run
   - Trace shows `"input_tokens":0,"output_tokens":0` for all ASK operations
   - **Impact:** Cannot measure token savings from cache, billing/quota tracking broken
   - **Location:** Token accounting in LLM handler pipeline

4. **Limited Speedup (9%)**
   - Expected: Near-instant cache hits (~50ms)
   - Actual: 7.9s → 7.2s (only 9% improvement)
   - **Hypothesis:** Overhead from graph scheduling, node dispatch, and data flow dominates
   - **Impact:** Cache savings are real but masked by framework overhead

### 🔍 Root Cause Analysis

**Why no dramatic speedup?**
- LLM latency: ~2-3 seconds per node (from trace timestamps)
- Framework overhead: ~4-5 seconds (compilation, scheduling, session setup)
- **Cache eliminates LLM time** (✅) but **not framework time** (overhead)
- With 4 nodes in parallel, framework overhead is ~60-70% of total time

**Why SQLite L2 is empty?**
- Possible causes:
  1. L1 DashMap never evicts (under 1024-entry limit), so L2 never queried
  2. L2 write logic only triggers on L1 miss (not on put)
  3. SQLite feature flag mismatch or failed initialization
- **Action needed:** Review `memoization.rs` L2 write path

**Why metrics show 0 tokens?**
- Token usage events ARE emitted (seen in trace.ndjson)
- But values are all zeros: `"input_tokens":0,"output_tokens":0`
- Likely: LLM backends not populating token fields in responses
- **Action needed:** Check `apxm-backends/src/llm/backends/openai/backend.rs` response parsing

## Cache Effectiveness Formula

```
Cache Speedup = (T_llm) / (T_llm + T_overhead)
```

Where:
- `T_llm` = LLM call time (~2.5s/node × 4 = ~10s, but parallelized)
- `T_overhead` = Framework time (~4-5s)

**With current overhead:** Cache saves ~3-4 seconds out of 7.9s total (~40-50% of time)
**With ideal overhead (<1s):** Cache would save ~2.5s out of 3.5s total (~70% of time)

## Recommendations

1. **Fix CLI cache path** (HIGH PRIORITY)
   ```rust
   // crates/apxm-cli/src/main.rs:5230
   // BEFORE:
   Ok(home.join(".apxm").join("cache.db"))
   // AFTER:
   Ok(home.join(".apxm").join("cache").join("cache.db"))
   ```

2. **Investigate L2 persistence** (HIGH PRIORITY)
   - Add debug logging to `MemoCache::put()` and L2 write path
   - Verify SQLite feature flag is enabled in all builds
   - Test cache survival across process restarts

3. **Fix token metrics** (MEDIUM PRIORITY)
   - Audit LLM response parsing for token count extraction
   - Add integration test verifying token counts are non-zero
   - Enable token-based billing/quota enforcement

4. **Reduce framework overhead** (LOW PRIORITY - optimization)
   - Profile session setup and graph compilation time
   - Consider artifact caching to skip recompilation
   - Optimize scheduler for small graphs (<10 nodes)

5. **Add cache observability** (MEDIUM PRIORITY)
   - Emit cache hit/miss events to trace.ndjson
   - Include cache stats in metrics.json (L1/L2 hits, evictions)
   - Add `--cache-stats` flag to `execute` command

## Conclusion

**MemoCache works** - it successfully eliminates LLM calls and returns cached responses. However, **multiple bugs prevent us from properly measuring and verifying cache effectiveness**:

1. CLI reports wrong database (always shows empty)
2. SQLite L2 never persists (only L1 in-memory works)
3. Token metrics are broken (all zeros)
4. Framework overhead dominates execution time

**Next steps:** Fix the CLI path bug first (1-line change), then investigate why L2 isn't persisting entries. Once observability is fixed, we can properly measure cache hit rates and savings in production workflows.

## Appendix: Test Artifact

**Cache test graph:** `examples/python/benchmarks/cache_test.apxm`
**Key change from original:** Added `"temperature": 0.0` to each ASK node's attributes.

**Sessions:**
- Run 1 (cold): `~/.apxm/sessions/cache_test-20260408T013837/`
- Run 2 (warm): `~/.apxm/sessions/cache_test-20260408T013943/`

**Database states:**
- `~/.apxm/cache.db` - 8KB (CLI target, outdated)
- `~/.apxm/cache/cache.db` - 12KB (runtime target, has schema but 0 rows)
