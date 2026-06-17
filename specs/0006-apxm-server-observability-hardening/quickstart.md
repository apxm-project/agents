# Quickstart — 0006 server observability hardening

## OTLP + metrics

```bash
export OTEL_EXPORTER_OTLP_ENDPOINT=http://127.0.0.1:4318
dekk apxm build-server
dekk apxm server --port 18800 &
curl -s http://127.0.0.1:18800/health
curl -s http://127.0.0.1:18800/metrics | head
```

## Rate limit + body cap (fixture)

```bash
export APXM_SERVER_RATE_LIMIT_RPS=2
export APXM_SERVER_MAX_BODY_BYTES=1024
dekk apxm test -p apxm-server hardening_contract
```

## Non-loopback auth (manual)

```bash
export APXM_SERVER_ADDR=0.0.0.0:18801
export APXM_SERVER_BEARER=dev-token
dekk apxm server
# curl without Authorization → 401 on /v1/execute
# curl -H "Authorization: Bearer dev-token" → 200/admission path
```

## Graceful drain

```bash
dekk apxm server --port 18800 &
kill -TERM $!
# logs: "shutdown signal received" → "rollout flush complete"
```
