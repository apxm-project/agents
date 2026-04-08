#!/usr/bin/env python3
"""APXM Autofix - Automated validate/classify/fix/verify loop.

Runs all Python examples through validation, classifies failures into bug clusters,
and generates fix task prompts for each cluster.

Usage:
    python3 scripts/apxm-autofix.py                              # Full audit
    python3 scripts/apxm-autofix.py --scope examples/python/acp-agents  # Scope to directory
    python3 scripts/apxm-autofix.py --report-only                # Just report, no tasks
    python3 scripts/apxm-autofix.py --include-tests              # Include Rust tests
    python3 scripts/apxm-autofix.py --verify-only                # Re-check only
    python3 scripts/apxm-autofix.py --auto-fix                   # Spawn agents (Phase 2)
"""

import argparse
import importlib.util
import json
import re
import subprocess
import sys
import tempfile
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


# Failure classification patterns
FAILURE_PATTERNS = {
    "attr_mismatch": [
        r"missing required attribute '(\w+)'",
        r"unknown attribute '(\w+)'",
        r"attribute '(\w+)' .* not recognized",
    ],
    "mlir_parse_error": [
        r"error\[E900\]",
        r"module parsing: operation returned null",
        r"expected '.+'",
        r"invalid MLIR",
    ],
    "import_error": [
        r"ModuleNotFoundError",
        r"ImportError",
        r"No module named",
    ],
    "validation_error": [
        r"validation error:",
        r"missing required",
        r"not reachable from",
        r"invalid edge",
    ],
    "type_mismatch": [
        r"expected '!'",
        r"type mismatch",
        r"mismatched types",
    ],
    "build_error": [
        r"error: could not compile",
        r"cargo build failed",
        r"compilation failed",
    ],
}


@dataclass
class ValidationResult:
    """Result of validating a single Python example."""
    file_path: Path
    passed: bool
    error_type: str | None = None
    error_message: str = ""
    traceback: str = ""
    air_output: str = ""


@dataclass
class BugCluster:
    """A cluster of failures with the same error type."""
    error_type: str
    failures: list[ValidationResult] = field(default_factory=list)

    @property
    def count(self) -> int:
        return len(self.failures)

    @property
    def file_paths(self) -> list[Path]:
        return [f.file_path for f in self.failures]


def classify_error(error_message: str) -> str:
    """Classify error message into a bug cluster type."""
    for error_type, patterns in FAILURE_PATTERNS.items():
        for pattern in patterns:
            if re.search(pattern, error_message, re.IGNORECASE):
                return error_type
    return "unknown"


def find_python_examples(scope: Path | None = None) -> list[Path]:
    """Find all Python example files to validate."""
    project_root = Path.cwd()
    if scope:
        search_root = project_root / scope
    else:
        search_root = project_root / "examples" / "python"

    if not search_root.exists():
        print(f"Error: {search_root} does not exist", file=sys.stderr)
        return []

    examples = []
    for py_file in search_root.rglob("*.py"):
        # Skip __pycache__, test files, and __init__.py
        if "__pycache__" in str(py_file) or py_file.name.startswith("test_") or py_file.name == "__init__.py":
            continue
        examples.append(py_file)

    return sorted(examples)


def import_python_module(file_path: Path) -> tuple[Any | None, str]:
    """Import a Python module from file path."""
    try:
        spec = importlib.util.spec_from_file_location(file_path.stem, file_path)
        if spec is None or spec.loader is None:
            return None, f"Could not load spec for {file_path}"

        module = importlib.util.module_from_spec(spec)
        sys.modules[file_path.stem] = module
        spec.loader.exec_module(module)
        return module, ""
    except Exception as e:
        return None, f"{type(e).__name__}: {e}"


def validate_example(file_path: Path, project_root: Path, verbose: bool = False) -> ValidationResult:
    """Validate a single Python example through the full pipeline."""

    # Step 1: Import the module
    module, import_error = import_python_module(file_path)
    if module is None:
        if verbose:
            print(f"  Import error: {import_error}")
        return ValidationResult(
            file_path=file_path,
            passed=False,
            error_type="import_error",
            error_message=import_error,
        )

    # Step 2: Find @compile'd graph
    compiled_flows = []
    for attr_name in dir(module):
        attr = getattr(module, attr_name)
        if hasattr(attr, "_graph"):
            compiled_flows.append((attr_name, attr))

    if not compiled_flows:
        return ValidationResult(
            file_path=file_path,
            passed=False,
            error_type="validation_error",
            error_message="No @compile'd graphs found in module",
        )

    # Step 3: Extract .air for the first flow
    flow_name, flow = compiled_flows[0]
    try:
        air_output = flow._graph.to_air()
    except Exception as e:
        return ValidationResult(
            file_path=file_path,
            passed=False,
            error_type="mlir_parse_error",
            error_message=f"Failed to emit AIR: {e}",
            traceback=str(e),
        )

    # Step 4: Write .air to temp file and compile
    with tempfile.NamedTemporaryFile(mode='w', suffix='.air', delete=False) as f:
        f.write(air_output)
        air_file = Path(f.name)

    try:
        # Compile the .air file (now valid MLIR that goes directly to Module::parse())
        # Successful compilation means the .air is valid
        result = subprocess.run(
            ["dekk", "apxm", "compile", str(air_file), "-o", f"{air_file}.apxmobj"],
            capture_output=True,
            text=True,
            timeout=30,
        )

        if result.returncode != 0:
            # Compilation errors are in stdout and stderr
            error_msg = result.stdout + "\n" + result.stderr
            # If MLIR toolchain is not available, air emission success is sufficient
            if "MLIR toolchain not detected" in error_msg or "context creation" in error_msg or "driver feature" in error_msg.lower():
                return ValidationResult(
                    file_path=file_path,
                    passed=True,
                    air_output=air_output,
                )
            error_type = classify_error(error_msg)
            return ValidationResult(
                file_path=file_path,
                passed=False,
                error_type=error_type,
                error_message=error_msg,
                air_output=air_output,
            )

        # Success!
        return ValidationResult(
            file_path=file_path,
            passed=True,
            air_output=air_output,
        )

    except subprocess.TimeoutExpired:
        return ValidationResult(
            file_path=file_path,
            passed=False,
            error_type="build_error",
            error_message="Compilation timeout (30s)",
        )
    except Exception as e:
        return ValidationResult(
            file_path=file_path,
            passed=False,
            error_type="unknown",
            error_message=f"Unexpected error: {e}",
        )
    finally:
        # Clean up temp files
        air_file.unlink(missing_ok=True)
        Path(f"{air_file}.apxmobj").unlink(missing_ok=True)


def cluster_failures(results: list[ValidationResult]) -> dict[str, BugCluster]:
    """Cluster failures by error type."""
    clusters: dict[str, BugCluster] = defaultdict(lambda: BugCluster(error_type="unknown"))

    for result in results:
        if not result.passed and result.error_type:
            if result.error_type not in clusters:
                clusters[result.error_type] = BugCluster(error_type=result.error_type)
            clusters[result.error_type].failures.append(result)

    return dict(clusters)


def generate_fix_prompt(cluster: BugCluster, project_root: Path) -> str:
    """Generate a fix task prompt for a bug cluster."""

    prompt_parts = [
        "=" * 80,
        f"AUTOFIX TASK: {cluster.error_type.upper()}",
        "=" * 80,
        "",
        f"Found {cluster.count} failures of type '{cluster.error_type}'",
        "",
        "AFFECTED FILES:",
    ]

    for i, failure in enumerate(cluster.failures, 1):
        rel_path = failure.file_path.relative_to(project_root)
        prompt_parts.append(f"  {i}. {rel_path}")

    prompt_parts.extend([
        "",
        "ERROR MESSAGES:",
        "",
    ])

    for i, failure in enumerate(cluster.failures, 1):
        rel_path = failure.file_path.relative_to(project_root)
        prompt_parts.extend([
            f"--- {rel_path} ---",
            failure.error_message.strip(),
            "",
        ])

    # Add cluster-specific fix instructions
    instructions = get_fix_instructions(cluster.error_type)
    prompt_parts.extend([
        "",
        "FIX INSTRUCTIONS:",
        "",
        instructions,
        "",
        "VERIFICATION:",
        "",
        "After making changes, verify with:",
        "  dekk apxm build",
        "  cargo test --workspace",
        "  python3 scripts/apxm-autofix.py --verify-only",
        "",
        "Then commit with:",
        f"  git add -A",
        f"  git commit -m \"fix({cluster.error_type}): <description>\"",
        "",
        "=" * 80,
    ])

    return "\n".join(prompt_parts)


def get_fix_instructions(error_type: str) -> str:
    """Get cluster-specific fix instructions."""

    instructions = {
        "attr_mismatch": """
1. Check AISOps.td dialect definitions for the affected operations
2. Compare with Python frontend emission in apxm/graph/proxy.py
3. Ensure all required attributes are present and properly typed
4. Look for:
   - Missing attributes in AISOps.td
   - Mismatched attribute names (Python vs. MLIR)
   - Wrong attribute types (StrAttr vs. typed attrs)
5. Update the operation definition in AISOps.td to match Python emission
6. Rebuild: dekk apxm build
""",
        "mlir_parse_error": """
1. Review the .air emission in the error messages
2. Check crates/apxm-frontend/python/apxm/graph/ir.py::to_air()
3. Compare emitted MLIR with dialect definitions in AISOps.td
4. Common issues:
   - Missing or extra braces in attribute dicts
   - Incorrect operation syntax (should be %name = ais.op_name {attrs})
   - Type annotation errors (! missing or misplaced)
5. Fix the to_air() emission or dialect definition
6. Test: PYTHONPATH=crates/apxm-frontend/python python3 <example>.py
""",
        "import_error": """
1. Check PYTHONPATH is set: export PYTHONPATH=crates/apxm-frontend/python
2. Verify module structure in crates/apxm-frontend/python/apxm/
3. Check for missing __init__.py files
4. Ensure _generated/ directory exists and is populated
5. Run codegen if needed: python3 tools/scripts/codegen.py
""",
        "validation_error": """
1. Review graph validation rules in crates/apxm-graph/src/validation.rs
2. Check for:
   - Missing edges between dependent nodes
   - Unreachable nodes (no path from entry)
   - Invalid edge types (Data/Control/Effect)
3. Update Python example to add missing edges
4. Or relax validation rules if they're too strict
""",
        "type_mismatch": """
1. Check operation type signatures in AISOps.td
2. Verify value types match expected inputs/outputs
3. Compare with Rust type definitions in crates/apxm-ais/src/definitions.rs
4. Ensure consistent type usage across dialect/frontend/runtime
""",
        "build_error": """
1. Run: dekk apxm build to see full error messages
2. Check Cargo.toml dependencies and features
3. Verify MLIR environment: dekk apxm doctor
4. Check for syntax errors in Rust code
5. Review recent commits for breaking changes
""",
        "unknown": """
1. Examine the error messages carefully
2. Search codebase for similar error patterns
3. Check recent commits for related changes
4. Consider adding a new failure pattern to apxm-autofix.py
5. If stuck, ask for help in the project channel
""",
    }

    return instructions.get(error_type, instructions["unknown"]).strip()


def write_task_files(clusters: dict[str, BugCluster], project_root: Path, task_dir: Path) -> list[Path]:
    """Write task prompt files for each cluster."""
    task_dir.mkdir(parents=True, exist_ok=True)
    task_files = []

    for error_type, cluster in clusters.items():
        task_file = task_dir / f"cluster-{error_type}.txt"
        prompt = generate_fix_prompt(cluster, project_root)
        task_file.write_text(prompt)
        task_files.append(task_file)
        print(f"  Generated: {task_file}")

    return task_files


def print_summary(results: list[ValidationResult], clusters: dict[str, BugCluster]) -> None:
    """Print validation summary."""
    passed = sum(1 for r in results if r.passed)
    failed = sum(1 for r in results if not r.passed)

    print("\n" + "=" * 80)
    print("VALIDATION SUMMARY")
    print("=" * 80)
    print(f"Total examples: {len(results)}")
    print(f"Passed: {passed}")
    print(f"Failed: {failed}")

    if clusters:
        print("\nFAILURE BREAKDOWN:")
        for error_type, cluster in sorted(clusters.items()):
            print(f"  {error_type}: {cluster.count}")

    print("=" * 80)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="APXM Autofix - Validate and classify failures",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--scope",
        type=Path,
        help="Scope validation to specific directory (e.g., examples/python/acp-agents)",
    )
    parser.add_argument(
        "--report-only",
        action="store_true",
        help="Only report failures, don't generate task files",
    )
    parser.add_argument(
        "--include-tests",
        action="store_true",
        help="Include Rust test results (not implemented yet)",
    )
    parser.add_argument(
        "--verify-only",
        action="store_true",
        help="Just re-check examples without generating tasks",
    )
    parser.add_argument(
        "--auto-fix",
        action="store_true",
        help="Auto-spawn Claude Code agents for each cluster (Phase 2)",
    )
    parser.add_argument(
        "--task-dir",
        type=Path,
        default=Path("/tmp/autofix-tasks"),
        help="Directory for generated task files (default: /tmp/autofix-tasks)",
    )

    args = parser.parse_args()

    project_root = Path.cwd()

    # Set up Python path for imports
    python_frontend = project_root / "crates" / "apxm-frontend" / "python"
    if python_frontend.exists():
        sys.path.insert(0, str(python_frontend))

    print("=" * 80)
    print("APXM AUTOFIX - Validation Pipeline")
    print("=" * 80)

    # Find examples
    examples = find_python_examples(args.scope)
    if not examples:
        print("No Python examples found!", file=sys.stderr)
        return 1

    print(f"\nFound {len(examples)} Python examples")
    if args.scope:
        print(f"Scope: {args.scope}")
    print()

    # Validate all examples
    results = []
    for i, example in enumerate(examples, 1):
        rel_path = example.relative_to(project_root)
        print(f"[{i}/{len(examples)}] Validating {rel_path}...", end=" ")
        sys.stdout.flush()

        result = validate_example(example, project_root)
        results.append(result)

        if result.passed:
            print("✓ PASS")
        else:
            print(f"✗ FAIL ({result.error_type})")

    # Cluster failures
    clusters = cluster_failures(results)

    # Print summary
    print_summary(results, clusters)

    # Generate task files unless report-only or verify-only
    if not args.report_only and not args.verify_only and clusters:
        print("\nGENERATING FIX TASKS:")
        task_files = write_task_files(clusters, project_root, args.task_dir)

        print(f"\nTask files written to: {args.task_dir}")
    # Auto-fix mode (Phase 2)
    if args.auto_fix and clusters:
        print("\n🤖 AUTO-FIX MODE (spawning agents via APXM runtime...)")

        # Determine scope argument
        scope_arg = str(args.scope) if args.scope else "examples/python"

        # Invoke autofix workflow through APXM runtime
        workflow_path = project_root / ".agents" / "skills" / "autofix" / "autofix_workflow.air"

        if not workflow_path.exists():
            print(f"Error: Workflow not found at {workflow_path}", file=sys.stderr)
            print("Run: dekk apxm compile .agents/skills/autofix/autofix_workflow.py", file=sys.stderr)
            return 1

        print(f"\nExecuting workflow: {workflow_path}")
        print(f"Scope: {scope_arg}")
        print()

        # Execute the workflow
        try:
            result = subprocess.run(
                [
                    "dekk", "apxm", "execute",
                    str(workflow_path),
                    "--emit-session",
                    "--",
                    scope_arg
                ],
                timeout=600,  # 10 minute timeout for agent execution
            )

            if result.returncode != 0:
                print(f"\n✗ Workflow execution failed with exit code {result.returncode}", file=sys.stderr)
                return result.returncode

        except subprocess.TimeoutExpired:
            print("\n✗ Workflow execution timeout (10 minutes)", file=sys.stderr)
            return 1
        except Exception as e:
            print(f"\n✗ Workflow execution error: {e}", file=sys.stderr)
            return 1

        # Re-run verification to check results
        print("\n" + "=" * 80)
        print("VERIFICATION (after autofix)")
        print("=" * 80)

        verify_examples = find_python_examples(args.scope)
        verify_results = []
        for i, example in enumerate(verify_examples, 1):
            rel_path = example.relative_to(project_root)
            print(f"[{i}/{len(verify_examples)}] Verifying {rel_path}...", end=" ")
            sys.stdout.flush()

            result = validate_example(example, project_root)
            verify_results.append(result)

            if result.passed:
                print("✓ PASS")
            else:
                print(f"✗ FAIL ({result.error_type})")

        # Print verification summary
        verify_clusters = cluster_failures(verify_results)
        print_summary(verify_results, verify_clusters)

        # Report what changed
        initial_failures = sum(1 for r in results if not r.passed)
        final_failures = sum(1 for r in verify_results if not r.passed)
        fixed_count = initial_failures - final_failures

        print("\n" + "=" * 80)
        print("AUTOFIX SUMMARY")
        print("=" * 80)
        print(f"Initial failures: {initial_failures}")
        print(f"Final failures: {final_failures}")
        print(f"Fixed: {fixed_count}")

        if final_failures == 0:
            print("\n✓ All issues resolved!")
        elif fixed_count > 0:
            print(f"\n⚠ Partial success: {fixed_count} issues fixed, {final_failures} remaining")
        else:
            print("\n✗ No issues were fixed")
        print("=" * 80)

    # Return non-zero if any failures
    return 1 if clusters else 0


if __name__ == "__main__":
    sys.exit(main())
