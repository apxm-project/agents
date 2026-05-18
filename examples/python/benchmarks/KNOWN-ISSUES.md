# Benchmark harness — known issues

## `stress/prefix_fanout_concurrent.py` reliably crashes vLLM engines

**Symptom.** Running `concurrent_matrix.py` (which dispatches the
above as tenants) against any live vLLM service produces, after the
first batch:

- per-tenant `returncode=1`, `failed_tenants` = `concurrency`
- container log shows `vllm.v1.engine.exceptions.EngineDeadError`
  followed by `Application shutdown complete`
- `docker ps -a` on the node shows the container `Exited (0) <N>
  seconds ago`
- subsequent requests to the endpoint get `HTTP=000` /
  `Connection refused`

**Reproduced on.**
- `gpt-oss-120b` (TP=8) on `apxm-vllm-runtime:a0e42ad3-e4e3a9af-post-rebase`
  at c=4 i=2 → engine dead after iter 1
- `Qwen3-32B` (TP=2 replica, port 8920) same image at single-tenant
  invocation → engine dead after one ASK request

**What this rules out.**
- Not model-specific (two different models, same crash)
- Not concurrency-driven (c=1 also kills it)
- Not the reasoning-parser config (Qwen3-32B is not a reasoning model)
- Not the APXM-fork patches (`/v1/apxm/*` routes work; direct
  `/v1/chat/completions` curl works fine for a single ad-hoc request)

**What this implicates.**
- Something specific to the request shape `dekk apxm execute` sends
  when invoking `prefix_fanout_concurrent.py`. The workload generates
  long shared-prefix prompts (~thousands of tokens) per tenant. The
  failure may be `max_model_len` overflow, KV cache thrashing, or a
  vLLM-side regression triggered by very long context.

**Next-session investigation path.**
1. Compare the request body `dekk apxm execute` sends vs the working
   ad-hoc `curl` (capture on the host with `tcpdump -i lo port
   <PORT>` where `<PORT>` is the service's manifest port).
2. Try `examples/python/benchmarks/workloads/pin_demo.py` instead —
   INT-03 shipped successfully with that workload on the same image.
3. If `pin_demo` also crashes, the bug is in the harness's tenant
   spawn (env, args, or `dekk apxm execute` runtime), not the
   workload itself.
4. Bisect by reducing the prompt-generation length in
   `prefix_fanout_concurrent.py` until the engine survives.

**Until fixed, don't use `concurrent_matrix.py` against any
production zoo service.** The crash kills the service, and the
`sleep infinity` at the end of `run-vllm.sh` keeps the Slurm job
in `R` state — so the only signal that the service is dead is
`docker ps -a` showing `Exited`. Cluster watchdog will not catch
this; benchmarks that follow a crashed cell will see opaque
connection errors.

**Workaround for the matrix-style claim.**
INT-03 (`docs/claims/dispatch-pin-latency-win.md`) shipped using
`pin_demo` — that is the known-working path for any near-term
matrix claim on this image. Use that workload until
`prefix_fanout_concurrent.py` is fixed.
