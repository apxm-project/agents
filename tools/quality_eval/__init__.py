"""Tier-3 quality-contract harness.

Compiles a fixture graph at the requested -O level, executes it against a
real (or mock) backend, and scores the captured output against an
``expected.toml`` rubric. Built so the harness has a sane offline default
(``NullJudge`` + mock backend) for contributors without API keys.
"""
