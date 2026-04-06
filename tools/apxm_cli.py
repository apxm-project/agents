#!/usr/bin/env python3
"""
APXM CLI - Python driver for APXM compiler and runtime.

Automatically handles conda environment detection, MLIR environment setup,
and provides convenient commands for building, running, and testing.

Usage:
    apxm doctor                     # Check environment status (built-in via dekk)
    apxm version                    # Show version information (built-in via dekk)
    apxm build                      # Build compiler and runtime
    apxm execute workflow.json      # Compile and execute an ApxmGraph file
    apxm compile workflow.json -o out.apxmobj  # Compile to artifact
    apxm run out.apxmobj            # Run pre-compiled artifact
    apxm test                       # Run test suite
    apxm install                    # Install/update environment
    apxm --help                     # Show all available commands
"""

from bootstrap_dekk import ensure_dekk_bootstrap

ensure_dekk_bootstrap()

from dekk import Typer

from scripts.build import register_commands as register_build
from scripts.codegen import register_commands as register_codegen
from scripts.compile import register_commands as register_compile
from scripts.execute import register_commands as register_execute
from scripts.install import register_commands as register_install
from scripts.models import register_commands as register_models
from scripts.run import register_commands as register_run
from scripts.test import register_commands as register_test

VERSION = "0.2.0"

# Main CLI app with auto-activation
app = Typer(
    name="apxm",
    auto_activate=True,
    fail_fast=False,
    add_doctor_command=True,
    add_version_command=True,
    project_version=VERSION,
    help="APXM CLI - Compiler and runtime driver",
    no_args_is_help=True,
)

# Register top-level commands
register_build(app)
register_codegen(app)
register_compile(app)
register_execute(app)
register_models(app)
register_run(app)
register_test(app)
register_install(app)


if __name__ == "__main__":
    app()
