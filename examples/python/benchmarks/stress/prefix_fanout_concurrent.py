#!/usr/bin/env python3
"""prefix_fanout_concurrent.py - Concurrent multi-tenant prefix-fanout source

Concurrent matrix workload. Same structural shape as ``prefix_fanout_large`` (one ~4K
shared context with 8-way fan-out reviewers, then a merge + print), but
parameterised over a tenant index so multiple instances launched in parallel
present *different* prefixes to the backend. The aggregate prefix mass across
the four tenants is large enough to put the vLLM KV cache into the regime
where ``_APXM_PIN_ALLOW_USAGE`` (0.85) gates the pin handle on, which is the
precondition the readiness notes identified as missing.

Tenant index source: ``APXM_MATRIX_VARIANT`` env var (default 0). Valid
values are 0..3. Each tenant's context is byte-distinct from byte 0 so the
backend's auto-prefix-cache treats them as disjoint blocks; *within* a single
tenant the 8-way fan-out still shares the full 4K prefix.

Usage:
  APXM_MATRIX_VARIANT=0 dekk apxm execute prefix_fanout_concurrent.py -O2
  APXM_MATRIX_VARIANT=1 dekk apxm execute prefix_fanout_concurrent.py -O2
  ...
"""

import os

from apxm import compile, GraphRecorder

from _config import VLLM, VLLM_ROUTE


_VARIANT_PROFILES = [
    {
        "service": "AuthenticationService",
        "domain": "authentication",
        "primary_file": "src/services/authentication.py",
        "tagline": "session/JWT/MFA login flow",
    },
    {
        "service": "BillingService",
        "domain": "billing",
        "primary_file": "src/services/billing_engine.py",
        "tagline": "invoice generation and payment reconciliation",
    },
    {
        "service": "InventoryService",
        "domain": "inventory",
        "primary_file": "src/services/inventory_ledger.py",
        "tagline": "warehouse stock-level reconciliation",
    },
    {
        "service": "NotificationService",
        "domain": "notifications",
        "primary_file": "src/services/notification_dispatcher.py",
        "tagline": "multi-channel notification fan-out",
    },
]


def _variant_index() -> int:
    raw = os.environ.get("APXM_MATRIX_VARIANT", "0")
    try:
        idx = int(raw)
    except ValueError:
        idx = 0
    if idx < 0:
        idx = 0
    return idx


def _build_context(idx: int) -> str:
    p = _VARIANT_PROFILES[idx % len(_VARIANT_PROFILES)]
    suffix = f"_v{idx}"
    service = p["service"] + suffix
    domain = p["domain"] + suffix
    primary_file = p["primary_file"].replace(".py", f"{suffix}.py")
    tagline = p["tagline"] + f" (tenant {idx})"
    body = (
        "You are reviewing a large pull request for the "
        + tagline
        + ". Here is the complete diff:\n\n"
        + "=== FILE: "
        + primary_file
        + " ===\n"
        + "CHANGES: 247 lines added, 89 lines removed\n\n"
        + "+ class "
        + service
        + ":\n"
        + "+     '''Enhanced "
        + domain
        + " service with multi-tenant support.'''\n"
        + "+\n"
        + "+     def __init__(self, db_pool, redis_client, config):\n"
        + "+         self.db = db_pool\n"
        + "+         self.cache = redis_client\n"
        + "+         self.config = config\n"
        + "+         self.session_manager = SessionManager(redis_client)\n"
        + "+         self.token_service = TokenService(config.jwt_secret)\n"
        + "+         self.password_hasher = PasswordHasher(cost_factor=12)\n"
        + "+         self.rate_limiter = RateLimiter(redis_client, max_attempts=5, window_seconds=900)\n"
        + "+         self.mfa_service = MFAService(config.mfa_settings)\n"
        + "+\n"
        + "+     async def perform_"
        + domain
        + "_operation(self, principal, payload, mfa_token=None):\n"
        + "+         '''Perform "
        + domain
        + " operation for the given principal with optional MFA.'''\n"
        + "+         if await self.rate_limiter.is_limited(principal):\n"
        + "+             raise RateLimitExceeded(f'Too many "
        + domain
        + " attempts for {principal}')\n"
        + "+         entity = await self.db.fetch_one(\n"
        + "+             'SELECT id, principal, secret_hash, is_verified, mfa_enabled FROM "
        + domain
        + "_entities WHERE principal = $1',\n"
        + "+             principal\n"
        + "+         )\n"
        + "+         if not entity:\n"
        + "+             await self.rate_limiter.increment(principal)\n"
        + "+             raise OperationFailed('Invalid credentials')\n"
        + "+         if not self.password_hasher.verify(payload, entity['secret_hash']):\n"
        + "+             await self.rate_limiter.increment(principal)\n"
        + "+             raise OperationFailed('Invalid credentials')\n"
        + "+         if entity['mfa_enabled']:\n"
        + "+             if not mfa_token:\n"
        + "+                 return { 'status': 'mfa_required', 'entity_id': entity['id']}\n"
        + "+             if not await self.mfa_service.verify_token(entity['id'], mfa_token):\n"
        + "+                 await self.rate_limiter.increment(principal)\n"
        + "+                 raise MFAVerificationFailed('Invalid MFA token')\n"
        + "+         session = await self.session_manager.create_session(\n"
        + "+             entity_id=entity['id'],\n"
        + "+             ttl_seconds=self.config.session_ttl\n"
        + "+         )\n"
        + "+         token = self.token_service.generate_token(\n"
        + "+             entity_id=entity['id'],\n"
        + "+             session_id=session.id,\n"
        + "+             expires_in=self.config.jwt_expiry\n"
        + "+         )\n"
        + "+         await self.rate_limiter.reset(principal)\n"
        + "+         return {\n"
        + "+             'status': 'completed',\n"
        + "+             'entity_id': entity['id'],\n"
        + "+             'token': token,\n"
        + "+             'session_id': session.id,\n"
        + "+             'expires_at': session.expires_at\n"
        + "+         }\n\n"
        + "=== FILE: src/services/session_manager.py ===\n"
        + "CHANGES: 156 lines added, 43 lines removed\n\n"
        + "+ class SessionManager:\n"
        + "+     '''Manages "
        + domain
        + " sessions with Redis backing and automatic cleanup.'''\n"
        + "+\n"
        + "+     def __init__(self, redis_client, default_ttl=1800):\n"
        + "+         self.redis = redis_client\n"
        + "+         self.default_ttl = default_ttl\n"
        + "+\n"
        + "+     async def create_session(self, entity_id, ttl_seconds=None):\n"
        + "+         '''Create a new "
        + domain
        + " session for entity.'''\n"
        + "+         session_id = self._generate_session_id()\n"
        + "+         ttl = ttl_seconds or self.default_ttl\n"
        + "+         expires_at = datetime.utcnow() + timedelta(seconds=ttl)\n"
        + "+         session_data = {\n"
        + "+             'id': session_id,\n"
        + "+             'entity_id': entity_id,\n"
        + "+             'created_at': datetime.utcnow().isoformat(),\n"
        + "+             'expires_at': expires_at.isoformat(),\n"
        + "+             'last_activity': datetime.utcnow().isoformat()\n"
        + "+         }\n"
        + "+         await self.redis.setex(\n"
        + "+             f'"
        + domain
        + "-session: { session_id}',\n"
        + "+             ttl,\n"
        + "+             json.dumps(session_data)\n"
        + "+         )\n"
        + "+         return SessionRecord(**session_data)\n"
        + "+\n"
        + "+     async def get_session(self, session_id):\n"
        + "+         '''Retrieve "
        + domain
        + " session by ID.'''\n"
        + "+         data = await self.redis.get(f'"
        + domain
        + "-session: { session_id}')\n"
        + "+         if not data:\n"
        + "+             raise SessionNotFound(session_id)\n"
        + "+         session = SessionRecord(**json.loads(data))\n"
        + "+         if datetime.fromisoformat(session.expires_at) < datetime.utcnow():\n"
        + "+             await self.delete_session(session_id)\n"
        + "+             raise SessionExpired(session_id)\n"
        + "+         return session\n"
        + "+\n"
        + "+     async def refresh_session(self, session_id, extend_by=None):\n"
        + "+         '''Extend "
        + domain
        + " session TTL.'''\n"
        + "+         session = await self.get_session(session_id)\n"
        + "+         extend_seconds = extend_by or self.default_ttl\n"
        + "+         new_expires_at = datetime.utcnow() + timedelta(seconds=extend_seconds)\n"
        + "+         session.expires_at = new_expires_at.isoformat()\n"
        + "+         session.last_activity = datetime.utcnow().isoformat()\n"
        + "+         await self.redis.setex(\n"
        + "+             f'"
        + domain
        + "-session: { session_id}',\n"
        + "+             extend_seconds,\n"
        + "+             json.dumps(asdict(session))\n"
        + "+         )\n"
        + "+         return session\n\n"
        + "=== FILE: src/services/token_service.py ===\n"
        + "CHANGES: 98 lines added, 34 lines removed\n\n"
        + "+ class TokenService:\n"
        + "+     '''JWT token generation and validation with RSA-2048 signing for "
        + domain
        + ".'''\n"
        + "+\n"
        + "+     def __init__(self, secret_key, algorithm='RS256'):\n"
        + "+         self.secret_key = secret_key\n"
        + "+         self.algorithm = algorithm\n"
        + "+         self.public_key = self._load_public_key()\n"
        + "+\n"
        + "+     def generate_token(self, entity_id, session_id, expires_in=86400):\n"
        + "+         '''Generate JWT token with entity and session claims for "
        + domain
        + ".'''\n"
        + "+         now = datetime.utcnow()\n"
        + "+         payload = {\n"
        + "+             'entity_id': entity_id,\n"
        + "+             'session_id': session_id,\n"
        + "+             'iat': now,\n"
        + "+             'exp': now + timedelta(seconds=expires_in),\n"
        + "+             'iss': '"
        + domain
        + ".example.com',\n"
        + "+             'aud': 'api.example.com'\n"
        + "+         }\n"
        + "+         token = jwt.encode(payload, self.secret_key, algorithm=self.algorithm)\n"
        + "+         return token\n"
        + "+\n"
        + "+     def verify_token(self, token):\n"
        + "+         '''Verify JWT signature and extract claims for "
        + domain
        + ".'''\n"
        + "+         try:\n"
        + "+             payload = jwt.decode(\n"
        + "+                 token,\n"
        + "+                 self.public_key,\n"
        + "+                 algorithms=[self.algorithm],\n"
        + "+                 audience='api.example.com',\n"
        + "+                 issuer='"
        + domain
        + ".example.com'\n"
        + "+             )\n"
        + "+             return payload\n"
        + "+         except jwt.ExpiredSignatureError:\n"
        + "+             raise TokenExpired('JWT token has expired')\n"
        + "+         except jwt.InvalidTokenError as e:\n"
        + "+             raise TokenInvalid(f'JWT validation failed: { e}')\n\n"
        + "=== FILE: tests/test_"
        + domain
        + ".py ===\n"
        + "CHANGES: 312 lines added, 0 lines removed\n\n"
        + "+ @pytest.mark.asyncio\n"
        + "+ async def test_successful_"
        + domain
        + "_operation(db, redis, config):\n"
        + "+     '''Test successful "
        + domain
        + " flow.'''\n"
        + "+     service = "
        + service
        + "(db, redis, config)\n"
        + "+     entity_id = await db.execute(\n"
        + "+         'INSERT INTO "
        + domain
        + "_entities (principal, secret_hash, is_verified) VALUES ($1, $2, $3) RETURNING id',\n"
        + "+         'principal-"
        + domain
        + "@example.com',\n"
        + "+         '$2b$12$hash...',\n"
        + "+         True\n"
        + "+     )\n"
        + "+     result = await service.perform_"
        + domain
        + "_operation('principal-"
        + domain
        + "@example.com', 'payload123')\n"
        + "+     assert result['status'] == 'completed'\n"
        + "+     assert result['entity_id'] == entity_id\n"
        + "+     assert 'token' in result\n"
        + "+     assert 'session_id' in result\n\n"
        + "SUMMARY:\n"
        + "- 813 lines changed across 4 files in the "
        + domain
        + " subsystem\n"
        + "- 3 new service classes: "
        + service
        + ", SessionManager, TokenService\n"
        + "- 1 new test file with 8 test cases targeting "
        + domain
        + " flows\n"
        + "- Dependencies: PyJWT 2.8, bcrypt 5.1, redis 7.0\n"
        + "- Breaking changes: None\n"
        + "- Migration required: No\n"
        + "- Performance impact: +80ms avg "
        + domain
        + " operation latency (bcrypt cost)\n"
    )
    return body


@compile(
    default_provider=VLLM,
    default_route=VLLM_ROUTE,
)
def prefix_fanout_concurrent(g: GraphRecorder):
    """Tenant-parameterised 8-way fan-out over a ~4K shared context."""

    idx = _variant_index()
    large_context = _build_context(idx)
    # Same escape trick as prefix_fanout_large: APXM template parser walks `{`
    # then matches `[a-zA-Z0-9_]+`. A trailing space breaks the match without
    # changing the model-readable content.
    large_context = large_context.replace("{", "{ ")

    review_security = g.ask(
        name="review_security",
        prompt=large_context + "\n\n=== SECURITY REVIEW ===\n"
        "Analyze this PR for security vulnerabilities:\n"
        "1. Are there any authentication/authorization issues?\n"
        "2. Is sensitive data properly protected?\n"
        "3. Are there any injection vulnerabilities?\n"
        "Provide 3-4 sentences.",
    )

    review_performance = g.ask(
        name="review_performance",
        prompt=large_context + "\n\n=== PERFORMANCE REVIEW ===\n"
        "Analyze this PR for performance concerns:\n"
        "1. Are there any performance bottlenecks?\n"
        "2. Is caching used effectively?\n"
        "3. Are database queries optimized?\n"
        "Provide 3-4 sentences.",
    )

    review_reliability = g.ask(
        name="review_reliability",
        prompt=large_context + "\n\n=== RELIABILITY REVIEW ===\n"
        "Analyze this PR for reliability concerns:\n"
        "1. Is error handling comprehensive?\n"
        "2. Are there race conditions or edge cases?\n"
        "3. Is the code fault-tolerant?\n"
        "Provide 3-4 sentences.",
    )

    review_scalability = g.ask(
        name="review_scalability",
        prompt=large_context + "\n\n=== SCALABILITY REVIEW ===\n"
        "Analyze this PR for scalability:\n"
        "1. Will this work with multiple instances?\n"
        "2. Are there any single points of contention?\n"
        "3. Can load be distributed effectively?\n"
        "Provide 3-4 sentences.",
    )

    review_maintainability = g.ask(
        name="review_maintainability",
        prompt=large_context + "\n\n=== MAINTAINABILITY REVIEW ===\n"
        "Analyze this PR for maintainability:\n"
        "1. Is the code well-structured and readable?\n"
        "2. Are there sufficient tests?\n"
        "3. Is documentation adequate?\n"
        "Provide 3-4 sentences.",
    )

    review_api_design = g.ask(
        name="review_api_design",
        prompt=large_context + "\n\n=== API DESIGN REVIEW ===\n"
        "Analyze this PR for API design:\n"
        "1. Is the API interface intuitive?\n"
        "2. Are there any breaking changes?\n"
        "3. Is error handling consistent?\n"
        "Provide 3-4 sentences.",
    )

    review_testing = g.ask(
        name="review_testing",
        prompt=large_context + "\n\n=== TESTING REVIEW ===\n"
        "Analyze this PR for test coverage:\n"
        "1. Are all critical paths tested?\n"
        "2. Are edge cases covered?\n"
        "3. Are tests isolated and repeatable?\n"
        "Provide 3-4 sentences.",
    )

    review_accessibility = g.ask(
        name="review_accessibility",
        prompt=large_context + "\n\n=== OBSERVABILITY REVIEW ===\n"
        "Analyze this PR for observability:\n"
        "1. Are error messages user-friendly?\n"
        "2. Is logging comprehensive for debugging?\n"
        "3. Are configuration options well-documented?\n"
        "Provide 3-4 sentences.",
    )

    final_report = g.merge(
        "final_report",
        review_security,
        review_performance,
        review_reliability,
        review_scalability,
        review_maintainability,
        review_api_design,
        review_testing,
        review_accessibility,
    )
    g.done(final_report)


if __name__ == "__main__":
    print(prefix_fanout_concurrent._graph.to_air())
