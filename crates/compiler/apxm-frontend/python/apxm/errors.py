"""APXM error types for graph compilation and execution."""

from __future__ import annotations


class ApxmError(Exception):
    """Base exception for APXM operations."""


class CompilationError(ApxmError):
    """Graph failed validation or MLIR compilation."""


class ExecutionError(ApxmError):
    """Workflow execution failed at runtime."""


class ServerError(ApxmError):
    """Cannot communicate with apxm-server."""
