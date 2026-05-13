#!/usr/bin/env python3
"""prefix_fanout_large.py - Large shared-prefix benchmark source

Tests: shared-prefix analysis with a large shared context.
Measures: emitted graph hints, backend cache telemetry, and wall-clock results.

Graph structure: 1 large context (4000 tokens), 8-way fan-out for different reviews
- O0: each review sends the full context independently.
- O2: compiler emits shared-prefix hints when it can prove the common prefix.

Metrics:
- Prefill and cached input tokens reported by the backend
- Shared-prefix graph telemetry
- Total execution time

Usage:
  dekk apxm execute examples/python/benchmarks/stress/prefix_fanout_large.py -O0
  dekk apxm execute examples/python/benchmarks/stress/prefix_fanout_large.py -O2
"""

from apxm import compile, GraphRecorder

from _config import VLLM, VLLM_ROUTE


@compile(
    default_provider=VLLM,
    default_route=VLLM_ROUTE,
)
def prefix_fanout_large(g: GraphRecorder):
    """One large 4000-token context shared by 8 parallel review nodes."""

    # Large context simulating a code review diff (~4000 tokens)
    large_context = """You are reviewing a large pull request. Here is the complete diff:

=== FILE: src/services/authentication.py ===
CHANGES: 247 lines added, 89 lines removed

+ class AuthenticationService:
+     '''Enhanced authentication service with multi-factor support.'''
+
+     def __init__(self, db_pool, redis_client, config):
+         self.db = db_pool
+         self.cache = redis_client
+         self.config = config
+         self.session_manager = SessionManager(redis_client)
+         self.token_service = TokenService(config.jwt_secret)
+         self.password_hasher = PasswordHasher(cost_factor=12)
+         self.rate_limiter = RateLimiter(redis_client, max_attempts=5, window_seconds=900)
+         self.mfa_service = MFAService(config.mfa_settings)
+
+     async def authenticate_user(self, email, password, mfa_token=None):
+         '''Authenticate user with email/password and optional MFA.'''
+         # Rate limit check
+         if await self.rate_limiter.is_limited(email):
+             raise RateLimitExceeded(f'Too many login attempts for {email}')
+
+         # Fetch user from database
+         user = await self.db.fetch_one(
+             'SELECT id, email, password_hash, is_verified, mfa_enabled FROM users WHERE email = $1',
+             email
+         )
+
+         if not user:
+             await self.rate_limiter.increment(email)
+             raise AuthenticationFailed('Invalid credentials')
+
+         # Verify password
+         if not self.password_hasher.verify(password, user['password_hash']):
+             await self.rate_limiter.increment(email)
+             raise AuthenticationFailed('Invalid credentials')
+
+         # Check MFA if enabled
+         if user['mfa_enabled']:
+             if not mfa_token:
+                 return {'status': 'mfa_required', 'user_id': user['id']}
+             if not await self.mfa_service.verify_token(user['id'], mfa_token):
+                 await self.rate_limiter.increment(email)
+                 raise MFAVerificationFailed('Invalid MFA token')
+
+         # Create session
+         session = await self.session_manager.create_session(
+             user_id=user['id'],
+             ttl_seconds=self.config.session_ttl
+         )
+
+         # Generate JWT
+         token = self.token_service.generate_token(
+             user_id=user['id'],
+             session_id=session.id,
+             expires_in=self.config.jwt_expiry
+         )
+
+         # Reset rate limit on success
+         await self.rate_limiter.reset(email)
+
+         return {
+             'status': 'authenticated',
+             'user_id': user['id'],
+             'token': token,
+             'session_id': session.id,
+             'expires_at': session.expires_at
+         }

=== FILE: src/services/session_manager.py ===
CHANGES: 156 lines added, 43 lines removed

+ class SessionManager:
+     '''Manages user sessions with Redis backing and automatic cleanup.'''
+
+     def __init__(self, redis_client, default_ttl=1800):
+         self.redis = redis_client
+         self.default_ttl = default_ttl
+
+     async def create_session(self, user_id, ttl_seconds=None):
+         '''Create a new session for user.'''
+         session_id = self._generate_session_id()
+         ttl = ttl_seconds or self.default_ttl
+         expires_at = datetime.utcnow() + timedelta(seconds=ttl)
+
+         session_data = {
+             'id': session_id,
+             'user_id': user_id,
+             'created_at': datetime.utcnow().isoformat(),
+             'expires_at': expires_at.isoformat(),
+             'last_activity': datetime.utcnow().isoformat()
+         }
+
+         # Store in Redis with TTL
+         await self.redis.setex(
+             f'session:{session_id}',
+             ttl,
+             json.dumps(session_data)
+         )
+
+         return SessionRecord(**session_data)
+
+     async def get_session(self, session_id):
+         '''Retrieve session by ID.'''
+         data = await self.redis.get(f'session:{session_id}')
+         if not data:
+             raise SessionNotFound(session_id)
+
+         session = SessionRecord(**json.loads(data))
+
+         # Check if expired
+         if datetime.fromisoformat(session.expires_at) < datetime.utcnow():
+             await self.delete_session(session_id)
+             raise SessionExpired(session_id)
+
+         return session
+
+     async def refresh_session(self, session_id, extend_by=None):
+         '''Extend session TTL.'''
+         session = await self.get_session(session_id)
+         extend_seconds = extend_by or self.default_ttl
+
+         new_expires_at = datetime.utcnow() + timedelta(seconds=extend_seconds)
+         session.expires_at = new_expires_at.isoformat()
+         session.last_activity = datetime.utcnow().isoformat()
+
+         await self.redis.setex(
+             f'session:{session_id}',
+             extend_seconds,
+             json.dumps(asdict(session))
+         )
+
+         return session

=== FILE: src/services/token_service.py ===
CHANGES: 98 lines added, 34 lines removed

+ class TokenService:
+     '''JWT token generation and validation with RSA-2048 signing.'''
+
+     def __init__(self, secret_key, algorithm='RS256'):
+         self.secret_key = secret_key
+         self.algorithm = algorithm
+         self.public_key = self._load_public_key()
+
+     def generate_token(self, user_id, session_id, expires_in=86400):
+         '''Generate JWT token with user and session claims.'''
+         now = datetime.utcnow()
+         payload = {
+             'user_id': user_id,
+             'session_id': session_id,
+             'iat': now,
+             'exp': now + timedelta(seconds=expires_in),
+             'iss': 'app.example.com',
+             'aud': 'api.example.com'
+         }
+
+         token = jwt.encode(payload, self.secret_key, algorithm=self.algorithm)
+         return token
+
+     def verify_token(self, token):
+         '''Verify JWT signature and extract claims.'''
+         try:
+             payload = jwt.decode(
+                 token,
+                 self.public_key,
+                 algorithms=[self.algorithm],
+                 audience='api.example.com',
+                 issuer='app.example.com'
+             )
+             return payload
+         except jwt.ExpiredSignatureError:
+             raise TokenExpired('JWT token has expired')
+         except jwt.InvalidTokenError as e:
+             raise TokenInvalid(f'JWT validation failed: {e}')

=== FILE: tests/test_authentication.py ===
CHANGES: 312 lines added, 0 lines removed

+ @pytest.mark.asyncio
+ async def test_successful_authentication(db, redis, config):
+     '''Test successful login flow.'''
+     auth_service = AuthenticationService(db, redis, config)
+
+     # Create test user
+     user_id = await db.execute(
+         'INSERT INTO users (email, password_hash, is_verified) VALUES ($1, $2, $3) RETURNING id',
+         'test@example.com',
+         '$2b$12$hash...',
+         True
+     )
+
+     # Authenticate
+     result = await auth_service.authenticate_user('test@example.com', 'password123')
+
+     assert result['status'] == 'authenticated'
+     assert result['user_id'] == user_id
+     assert 'token' in result
+     assert 'session_id' in result

SUMMARY:
- 813 lines changed across 4 files
- 3 new service classes: AuthenticationService, SessionManager, TokenService
- 1 new test file with 8 test cases
- Dependencies: PyJWT 2.8, bcrypt 5.1, redis 7.0
- Breaking changes: None
- Migration required: No
- Performance impact: +80ms avg login latency (bcrypt cost)
"""

    # The literal review text contains Python f-string fragments like
    # {email} and {session_id} that the APXM template parser would otherwise
    # interpret as graph placeholders. Insert a space after each '{' to break
    # the {name} pattern; the model still reads the code as intended.
    large_context = large_context.replace("{", "{ ")

    # 8-way fan-out: different review aspects
    # All share the SAME large context prefix (4000 tokens)
    # Each adds a small unique suffix (50-100 tokens)

    review_security = g.ask(
        name="review_security",
        prompt=large_context + "\n\n=== SECURITY REVIEW ===\n"
        "Analyze this PR for security vulnerabilities:\n"
        "1. Are there any authentication/authorization issues?\n"
        "2. Is sensitive data properly protected?\n"
        "3. Are there any injection vulnerabilities?\n"
        "Provide 3-4 sentences."
    )

    review_performance = g.ask(
        name="review_performance",
        prompt=large_context + "\n\n=== PERFORMANCE REVIEW ===\n"
        "Analyze this PR for performance concerns:\n"
        "1. Are there any performance bottlenecks?\n"
        "2. Is caching used effectively?\n"
        "3. Are database queries optimized?\n"
        "Provide 3-4 sentences."
    )

    review_reliability = g.ask(
        name="review_reliability",
        prompt=large_context + "\n\n=== RELIABILITY REVIEW ===\n"
        "Analyze this PR for reliability concerns:\n"
        "1. Is error handling comprehensive?\n"
        "2. Are there race conditions or edge cases?\n"
        "3. Is the code fault-tolerant?\n"
        "Provide 3-4 sentences."
    )

    review_scalability = g.ask(
        name="review_scalability",
        prompt=large_context + "\n\n=== SCALABILITY REVIEW ===\n"
        "Analyze this PR for scalability:\n"
        "1. Will this work with multiple instances?\n"
        "2. Are there any single points of contention?\n"
        "3. Can load be distributed effectively?\n"
        "Provide 3-4 sentences."
    )

    review_maintainability = g.ask(
        name="review_maintainability",
        prompt=large_context + "\n\n=== MAINTAINABILITY REVIEW ===\n"
        "Analyze this PR for maintainability:\n"
        "1. Is the code well-structured and readable?\n"
        "2. Are there sufficient tests?\n"
        "3. Is documentation adequate?\n"
        "Provide 3-4 sentences."
    )

    review_api_design = g.ask(
        name="review_api_design",
        prompt=large_context + "\n\n=== API DESIGN REVIEW ===\n"
        "Analyze this PR for API design:\n"
        "1. Is the API interface intuitive?\n"
        "2. Are there any breaking changes?\n"
        "3. Is error handling consistent?\n"
        "Provide 3-4 sentences."
    )

    review_testing = g.ask(
        name="review_testing",
        prompt=large_context + "\n\n=== TESTING REVIEW ===\n"
        "Analyze this PR for test coverage:\n"
        "1. Are all critical paths tested?\n"
        "2. Are edge cases covered?\n"
        "3. Are tests isolated and repeatable?\n"
        "Provide 3-4 sentences."
    )

    review_accessibility = g.ask(
        name="review_accessibility",
        prompt=large_context + "\n\n=== ACCESSIBILITY REVIEW ===\n"
        "Analyze this PR for accessibility:\n"
        "1. Are error messages user-friendly?\n"
        "2. Is logging comprehensive for debugging?\n"
        "3. Are configuration options well-documented?\n"
        "Provide 3-4 sentences."
    )

    # Merge all reviews
    final_report = g.merge(
        "final_report",
        review_security,
        review_performance,
        review_reliability,
        review_scalability,
        review_maintainability,
        review_api_design,
        review_testing,
        review_accessibility
    )

    # Output (auto-wires from each {review_*} placeholder).
    output = g.print(
        message="=== PREFIX FANOUT LARGE STRESS TEST ===\n\n"
        "Security: {review_security}\n\n"
        "Performance: {review_performance}\n\n"
        "Reliability: {review_reliability}\n\n"
        "Scalability: {review_scalability}\n\n"
        "Maintainability: {review_maintainability}\n\n"
        "API Design: {review_api_design}\n\n"
        "Testing: {review_testing}\n\n"
        "Accessibility: {review_accessibility}\n\n"
        "---\n"
        "This workflow sent a 4000-token context to 8 parallel review nodes.\n"
        "O0: each review sends the full context independently\n"
        "O2: shared-prefix hints and backend cache telemetry must be measured"
    )
    # Control edge keeps the merge as a synchronization barrier.
    g.add_edge(final_report, output, dependency="Control")

    g.done(output)


if __name__ == "__main__":
    # Output AIR.
    print(prefix_fanout_large._graph.to_air())
    # To execute directly:
    # import apxm
    # result = apxm.run(prefix_fanout_large())
    # print(result.content)
