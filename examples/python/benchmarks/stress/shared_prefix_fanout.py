#!/usr/bin/env python3
"""shared_prefix_fanout.py - Shared-prefix benchmark source

Tests: compiler-emitted shared-prefix hints across parallel nodes.
Measures: emitted graph hints, backend cache telemetry, and total latency.

Usage:
  dekk apxm execute examples/python/benchmarks/stress/shared_prefix_fanout.py -O0
  dekk apxm execute examples/python/benchmarks/stress/shared_prefix_fanout.py -O2
"""

from apxm import compile, GraphRecorder

from _config import VLLM, VLLM_ROUTE


@compile(
    default_provider=VLLM,
    default_route=VLLM_ROUTE,
)
def shared_prefix_fanout(g: GraphRecorder):
    """One large context shared by 4 parallel review nodes."""

    # Define shared context as a constant that will be referenced
    context_text = """You are reviewing a large codebase. Here is the complete context:

        MODULE: Authentication System
        ================================
        The authentication system handles user login, session management, and JWT token validation.
        It consists of the following components:

        1. LoginHandler - processes login requests, validates credentials against database
        2. SessionManager - creates and manages user sessions with Redis backing
        3. TokenService - generates and validates JWT tokens with RSA-2048 signing
        4. PasswordHasher - uses bcrypt with cost factor 12 for password hashing
        5. RateLimiter - prevents brute force attacks, max 5 attempts per 15 minutes

        Current implementation uses PostgreSQL for user data, Redis for sessions,
        and implements OAuth2 flows for third-party authentication.

        Known issues:
        - Token refresh race condition in high-load scenarios
        - Session cleanup cron job causing database locks
        - Rate limiter not distributed across instances

        Performance characteristics:
        - Average login latency: 120ms
        - Token validation: 5ms
        - Session lookup: 3ms (Redis cache hit)
        - Password verification: 80ms (bcrypt cost)

        Dependencies: postgresql 14, redis 7, jsonwebtoken 9.0, bcrypt 5.1
        Test coverage: 87% (missing edge cases in OAuth flow)
        """

    # Parallel review tasks - all share the same large context
    review_security = g.ask(
        name="review_security",
        prompt=context_text + "\n\nFocus on SECURITY aspects:\n"
        "1. Are there any authentication vulnerabilities?\n"
        "2. Is the JWT implementation secure?\n"
        "3. Are rate limits sufficient?\n"
        "Provide a security assessment in 3-4 sentences."
    )

    review_performance = g.ask(
        name="review_performance",
        prompt=context_text + "\n\nFocus on PERFORMANCE aspects:\n"
        "1. Are there any performance bottlenecks?\n"
        "2. Is caching used effectively?\n"
        "3. Can any operations be optimized?\n"
        "Provide a performance assessment in 3-4 sentences."
    )

    review_reliability = g.ask(
        name="review_reliability",
        prompt=context_text + "\n\nFocus on RELIABILITY aspects:\n"
        "1. What are the failure modes?\n"
        "2. Is error handling comprehensive?\n"
        "3. Are there any race conditions?\n"
        "Provide a reliability assessment in 3-4 sentences."
    )

    review_scalability = g.ask(
        name="review_scalability",
        prompt=context_text + "\n\nFocus on SCALABILITY aspects:\n"
        "1. Will this work with multiple instances?\n"
        "2. Are there any single points of contention?\n"
        "3. Can load be distributed effectively?\n"
        "Provide a scalability assessment in 3-4 sentences."
    )

    # Merge all reviews
    final_report = g.merge(
        "final_report",
        review_security,
        review_performance,
        review_reliability,
        review_scalability
    )

    output = g.print(
        message="=== COMPLETE CODE REVIEW ===\n\n"
        "Security Review:\n{review_security}\n\n"
        "Performance Review:\n{review_performance}\n\n"
        "Reliability Review:\n{review_reliability}\n\n"
        "Scalability Review:\n{review_scalability}"
    )

    g.add_edge(output, final_report, dependency="Control")
    g.done(final_report)


if __name__ == "__main__":
    # Output AIR.
    print(shared_prefix_fanout._graph.to_air())
    # To execute directly:
    # import apxm
    # result = apxm.run(shared_prefix_fanout())
    # print(result.content)
