#!/usr/bin/env python3
"""dead_context_stress.py - Benchmark for Dead Context Elimination

Tests: DeadContextElimination optimization pass
Measures: Removal of unused context from LLM prompts to save tokens

Graph structure: 5 upstream nodes producing context, 1 downstream think node
referencing only the database context.
- O0: All 5 contexts passed to LLM (~5000 tokens wasted on unused context)
- O2 with DeadContextElimination: only referenced context is retained statically

Metrics:
- Total input tokens (should drop significantly from O0 to O2)
- Number of context inputs wired to downstream node
- Static context payload reduction

Usage:
  dekk apxm execute dead_context_stress.air -O0  # No elimination (all 5 contexts sent)
  dekk apxm execute dead_context_stress.air -O2  # With elimination (only context_1 sent)
"""

from apxm import compile, GraphRecorder
from apxm.constants import INPUT_NAMES


@compile()
def dead_context_stress(g: GraphRecorder):
    """5 upstream contexts, downstream node only uses first one."""

    # Generate 5 large context chunks (simulating database queries, file reads, etc.)
    # Each is ~1000 tokens of context

    context_database_text = """DATABASE SCHEMA CONTEXT (1000 tokens):

Tables: users, posts, comments, likes, followers, sessions, notifications
- users: id, username, email, password_hash, created_at, updated_at, is_verified, profile_data
- posts: id, user_id, title, content, created_at, updated_at, view_count, is_published, tags
- comments: id, post_id, user_id, content, created_at, is_edited, parent_comment_id
- likes: id, user_id, post_id, created_at (composite unique on user_id + post_id)
- followers: id, follower_id, following_id, created_at (composite unique)
- sessions: id, user_id, token, expires_at, created_at, last_activity
- notifications: id, user_id, type, content, is_read, created_at

Indexes: users.email (unique), posts.user_id, posts.created_at, comments.post_id,
likes.post_id, followers.follower_id, followers.following_id, sessions.token (unique)

Foreign keys: All user_id fields reference users.id, post_id references posts.id"""

    context_1 = g.ask(
        name="context_database",
        prompt=context_database_text
    )

    context_api_docs_text = """API DOCUMENTATION CONTEXT (1000 tokens):

Endpoints:
GET /api/users/:id - Fetch user profile (auth optional)
POST /api/users - Create new user (requires: username, email, password)
PUT /api/users/:id - Update user (requires auth, owner only)
DELETE /api/users/:id - Delete user (requires auth, owner only)

GET /api/posts - List posts (supports: limit, offset, user_id, tag filters)
GET /api/posts/:id - Fetch single post (includes: post data, author, comment count)
POST /api/posts - Create post (requires auth, params: title, content, tags)
PUT /api/posts/:id - Update post (requires auth, owner only)
DELETE /api/posts/:id - Delete post (requires auth, owner only)

GET /api/comments?post_id=:id - List comments for post (paginated)
POST /api/comments - Create comment (requires auth, params: post_id, content)
DELETE /api/comments/:id - Delete comment (requires auth, owner/admin)

POST /api/likes - Toggle like (requires auth, params: post_id, idempotent)
GET /api/likes?post_id=:id - Get like count and user likes

POST /api/auth/login - Login (params: email, password, returns: token)
POST /api/auth/logout - Logout (requires auth, invalidates token)
POST /api/auth/refresh - Refresh session token (requires valid token)"""

    context_2 = g.ask(
        name="context_api_docs",
        prompt=context_api_docs_text
    )

    context_environment_text = """ENVIRONMENT CONFIGURATION CONTEXT (1000 tokens):

Production:
- DATABASE_URL: postgres://prod-db.internal:5432/app_production
- REDIS_URL: redis://prod-cache.internal:6379/0
- API_BASE_URL: https://api.example.com
- FRONTEND_URL: https://www.example.com
- JWT_SECRET: <secret-from-vault>
- JWT_EXPIRY: 86400 (24 hours)
- SESSION_TIMEOUT: 1800 (30 minutes)
- BCRYPT_ROUNDS: 12
- RATE_LIMIT_WINDOW: 900 (15 minutes)
- RATE_LIMIT_MAX: 5
- FILE_UPLOAD_MAX_SIZE: 10485760 (10MB)
- CORS_ORIGINS: https://www.example.com,https://admin.example.com
- LOG_LEVEL: warn
- SENTRY_DSN: <configured>

Staging:
- DATABASE_URL: postgres://staging-db.internal:5432/app_staging
- REDIS_URL: redis://staging-cache.internal:6379/0
- API_BASE_URL: https://api-staging.example.com
- FRONTEND_URL: https://staging.example.com
- JWT_EXPIRY: 3600 (1 hour for testing)
- LOG_LEVEL: info

Development:
- DATABASE_URL: postgres://localhost:5432/app_dev
- REDIS_URL: redis://localhost:6379/0
- API_BASE_URL: http://localhost:8000
- FRONTEND_URL: http://localhost:3000
- JWT_SECRET: dev-secret-not-for-production
- LOG_LEVEL: debug"""

    context_3 = g.ask(
        name="context_environment",
        prompt=context_environment_text
    )

    context_metrics_text = """PERFORMANCE METRICS CONTEXT (1000 tokens):

Last 7 days (production):
- Total requests: 14,285,392
- Average response time: 145ms (p50), 320ms (p95), 890ms (p99)
- Error rate: 0.23% (32,876 errors)
- Cache hit rate: 87.3%
- Database query time: 12ms (p50), 45ms (p95), 120ms (p99)
- Redis latency: 1.2ms (p50), 3.5ms (p95), 8.2ms (p99)

Endpoint breakdown:
- GET /api/posts: 8,342,112 requests, 98ms p50, 245ms p95
- GET /api/posts/:id: 3,129,845 requests, 52ms p50, 134ms p95
- POST /api/auth/login: 412,334 requests, 185ms p50, 420ms p95 (bcrypt cost)
- GET /api/users/:id: 1,834,221 requests, 45ms p50, 112ms p95
- POST /api/posts: 234,112 requests, 210ms p50, 580ms p95

Error breakdown:
- 401 Unauthorized: 18,234 (invalid/expired tokens)
- 429 Rate Limited: 8,123 (exceeded 5 login attempts)
- 500 Internal Server: 4,892 (database connection pool exhaustion)
- 503 Service Unavailable: 1,627 (Redis connection failures)

Resource utilization:
- CPU: 42% average, 78% peak
- Memory: 3.2GB / 8GB
- Database connections: 45 / 100 pool size
- Active sessions: 12,334 concurrent users"""

    context_4 = g.ask(
        name="context_metrics",
        prompt=context_metrics_text
    )

    context_security_audit_text = """SECURITY AUDIT CONTEXT (1000 tokens):

Recent vulnerability scan (2024-04-01):
- SQL Injection: PASS (parameterized queries, ORM usage)
- XSS: PASS (output encoding, CSP headers)
- CSRF: PASS (token validation on state-changing operations)
- Authentication: PASS (bcrypt hashing, secure session tokens)
- Authorization: PASS (role-based checks on protected endpoints)
- Rate Limiting: PARTIAL (not distributed, single-instance only)
- Session Management: PARTIAL (no session fixation protection)

Dependency scan:
- 3 HIGH severity issues in transitive dependencies
- 12 MEDIUM severity issues (mostly outdated packages)
- Recommendations: upgrade jsonwebtoken 8.5 → 9.0, bcrypt 4.0 → 5.1

Compliance checklist (SOC2):
- Encryption at rest: PASS (database volume encryption)
- Encryption in transit: PASS (TLS 1.3 enforced)
- Audit logging: PASS (all auth events logged)
- Access controls: PASS (least privilege, role separation)
- Password policy: PASS (min 12 chars, complexity requirements)
- MFA: NOT IMPLEMENTED (planned for Q3)
- Backup/recovery: PASS (daily backups, tested restore)

Penetration test findings (2024-03-15):
- Token refresh race condition (MEDIUM) - in remediation
- Session cleanup database lock (LOW) - scheduled fix
- Rate limiter not clustered (MEDIUM) - architecture change needed"""

    context_5 = g.ask(
        name="context_security_audit",
        prompt=context_security_audit_text
    )

    # Downstream node: template only references {context_1}, but all five
    # contexts are wired so the LLM sees them at O0. DeadContextElimination
    # (O2) detects that context_2..context_5 are unused and prunes them.
    #
    # input_names lists every Data input (must equal the number of incoming
    # Data edges). The template references only context_1 — that's the whole
    # point of the benchmark.
    analysis = g.think(
        name="analysis",
        prompt="""{context_1}

Based on the database schema above, what are the main entities in this system?
List the top 3 entities and their relationships in 2-3 sentences.

(Note: This prompt only references context_1; context_2..context_5 are dead.)""",
        **{INPUT_NAMES: [
            "context_1",
            "context_2",
            "context_3",
            "context_4",
            "context_5",
        ]},
    )

    # Wire context_2..context_5 explicitly (auto-wire already added context_1).
    g.add_edge(context_2, analysis)
    g.add_edge(context_3, analysis)
    g.add_edge(context_4, analysis)
    g.add_edge(context_5, analysis)

    # Output result
    output = g.print(
        message="=== DEAD CONTEXT ELIMINATION STRESS TEST ===\n\n"
        "Analysis:\n{analysis}\n\n"
        "This workflow created 5 large context chunks (~1000 tokens each).\n"
        "The downstream 'analysis' node only references context_1 in its template.\n\n"
        "O0: All 5 contexts sent to LLM (~5000 tokens)\n"
        "O2 with DeadContextElimination: only referenced context retained statically"
    )

    g.done(output)


if __name__ == "__main__":
    # Output AIR.
    print(dead_context_stress._graph.to_air())
    # To execute directly:
    # import apxm
    # result = apxm.run(dead_context_stress())
    # print(result.content)
