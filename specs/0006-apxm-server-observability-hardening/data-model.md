# Data model — 0006 server observability hardening

## Config (apxm-driver `ServerConfig` extensions)

| Field | Type | Default | Purpose |
|---|---|---|---|
| `observability.otlp_endpoint` | `Option<String>` | `None` | OTLP HTTP endpoint |
| `observability.metrics_enabled` | `bool` | `true` | Gate `/metrics` route |
| `safety.rate_limit_rps` | `Option<u32>` | `None` | Per-principal RPS cap |
| `safety.rate_limit_burst` | `Option<u32>` | `None` | Burst tokens (defaults to RPS) |
| `safety.max_body_bytes` | `Option<usize>` | `Some(10_485_760)` | HTTP body cap (10 MiB) |
| `shutdown.drain_timeout_secs` | `u64` | `30` | Rollout flush wait |

Env overrides: `OTEL_EXPORTER_OTLP_ENDPOINT`, `APXM_SERVER_RATE_LIMIT_RPS`,
`APXM_SERVER_MAX_BODY_BYTES`, `APXM_SERVER_DRAIN_TIMEOUT_SECS`.

## Runtime types (apxm-server)

| Type | Location | Notes |
|---|---|---|
| `PrincipalId` | `principal.rs` | Stable bearer fingerprint |
| `SafetyLimiter` | `safety.rs` | Per-key token bucket |
| `ShutdownCoordinator` | `shutdown.rs` | HTTP in-flight + drain signal |
| `MetricsSnapshot` | `metrics.rs` | Prometheus text builder |

## Request flow

```
 HTTP request
   → body limit layer (safety)
   → rate limit layer (safety, keyed by principal/IP)
   → bearer auth (effective_require_auth)
   → principal extension insert
   → handler
```

## Effective auth policy

| Bind | `require_auth` config | Effective |
|---|---|---|
| loopback | `false` (default) | off |
| loopback | `true` | on |
| non-loopback | any | **on** (fail-closed) |
