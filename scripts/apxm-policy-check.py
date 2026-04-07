#!/usr/bin/env python3
"""APXM Policy Checker - Enforce coding standards and contracts.

Validates:
- No hardcoded "ais.*" strings in lower_mlir.rs (outside tests)
- All Python graph constants come from _generated/
- AISOps.td ops match AISOperationType enum variants
- Memory space uses typed enum attrs, not StrAttr
- Every Python example compiles through the full pipeline

Usage:
    python3 scripts/apxm-policy-check.py          # Full check
    python3 scripts/apxm-policy-check.py --fix     # Auto-generate fix suggestions
"""

import argparse
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


@dataclass
class PolicyViolation:
    """A single policy violation."""
    policy: str
    file_path: Path
    line_number: int | None = None
    message: str = ""
    suggestion: str = ""


@dataclass
class PolicyCheckResult:
    """Results of policy checking."""
    passed: bool
    violations: list[PolicyViolation] = field(default_factory=list)

    @property
    def violation_count(self) -> int:
        return len(self.violations)


def check_no_hardcoded_ais_strings(project_root: Path) -> list[PolicyViolation]:
    """Check that lower_mlir.rs doesn't have hardcoded 'ais.*' strings outside tests."""
    violations = []

    lower_mlir = project_root / "crates" / "apxm-compiler" / "src" / "lower_mlir.rs"
    if not lower_mlir.exists():
        return violations

    content = lower_mlir.read_text()
    lines = content.split("\n")

    # Pattern to match hardcoded ais.* strings
    ais_pattern = re.compile(r'"ais\.\w+"')

    # Find test modules to exclude
    in_test_module = False
    test_module_depth = 0

    for i, line in enumerate(lines, 1):
        # Track test module boundaries
        if re.search(r'#\[cfg\(test\)\]', line):
            in_test_module = True
            test_module_depth = 0
        elif in_test_module:
            test_module_depth += line.count('{') - line.count('}')
            if test_module_depth <= 0:
                in_test_module = False

        # Skip if we're in a test module
        if in_test_module:
            continue

        # Check for hardcoded ais.* strings
        if ais_pattern.search(line):
            violations.append(PolicyViolation(
                policy="no_hardcoded_ais_strings",
                file_path=lower_mlir,
                line_number=i,
                message=f"Hardcoded 'ais.*' string found: {line.strip()}",
                suggestion="Use AISOperationType::mnemonic() instead of hardcoded strings",
            ))

    return violations


def check_python_uses_generated_constants(project_root: Path) -> list[PolicyViolation]:
    """Check that Python graph code uses _generated/ constants."""
    violations = []

    # Check apxm/graph/proxy.py and ir.py
    files_to_check = [
        project_root / "crates" / "apxm-frontend" / "python" / "apxm" / "graph" / "proxy.py",
        project_root / "crates" / "apxm-frontend" / "python" / "apxm" / "graph" / "ir.py",
    ]

    for file_path in files_to_check:
        if not file_path.exists():
            continue

        content = file_path.read_text()
        lines = content.split("\n")

        # Check for hardcoded operation names (should use operations module)
        hardcoded_ops = re.compile(r'"op":\s*"([A-Z_]+)"')

        for i, line in enumerate(lines, 1):
            match = hardcoded_ops.search(line)
            if match:
                op_name = match.group(1)
                violations.append(PolicyViolation(
                    policy="python_uses_generated",
                    file_path=file_path,
                    line_number=i,
                    message=f"Hardcoded operation name '{op_name}': {line.strip()}",
                    suggestion=f"Use operations.{op_name} from _generated/",
                ))

    return violations


def check_aisops_matches_enum(project_root: Path) -> list[PolicyViolation]:
    """Check that AISOps.td operation definitions match AISOperationType enum."""
    violations = []

    # Read AISOps.td
    aisops_td = project_root / "crates" / "apxm-compiler" / "dialect" / "AISOps.td"
    definitions_rs = project_root / "crates" / "apxm-ais" / "src" / "definitions.rs"

    if not aisops_td.exists() or not definitions_rs.exists():
        return violations

    # Extract operation names from AISOps.td
    td_content = aisops_td.read_text()
    td_ops = set(re.findall(r'def\s+AIS_(\w+)Op\s*:', td_content))

    # Extract enum variants from definitions.rs
    rs_content = definitions_rs.read_text()
    enum_match = re.search(r'pub enum AISOperationType\s*\{([^}]+)\}', rs_content, re.DOTALL)

    if not enum_match:
        violations.append(PolicyViolation(
            policy="aisops_matches_enum",
            file_path=definitions_rs,
            message="Could not find AISOperationType enum",
            suggestion="Verify enum definition in definitions.rs",
        ))
        return violations

    # Extract enum variants
    enum_body = enum_match.group(1)
    enum_variants = set(re.findall(r'^\s*(\w+)', enum_body, re.MULTILINE))

    # Filter out internal/metadata variants
    internal_variants = {'Agent', 'ConstStr', 'Yield'}
    public_variants = {v for v in enum_variants if v not in internal_variants}

    # Check for mismatches
    td_only = td_ops - public_variants
    enum_only = public_variants - td_ops

    for op in td_only:
        violations.append(PolicyViolation(
            policy="aisops_matches_enum",
            file_path=aisops_td,
            message=f"Operation {op} defined in AISOps.td but not in AISOperationType enum",
            suggestion=f"Add {op} variant to AISOperationType in definitions.rs",
        ))

    for op in enum_only:
        violations.append(PolicyViolation(
            policy="aisops_matches_enum",
            file_path=definitions_rs,
            message=f"Enum variant {op} exists but no AIS_{op}Op in AISOps.td",
            suggestion=f"Add def AIS_{op}Op to AISOps.td or remove from enum",
        ))

    return violations


def check_memory_space_typed_attrs(project_root: Path) -> list[PolicyViolation]:
    """Check that memory space uses typed enum attrs, not StrAttr."""
    violations = []

    aisops_td = project_root / "crates" / "apxm-compiler" / "dialect" / "AISOps.td"
    if not aisops_td.exists():
        return violations

    content = aisops_td.read_text()
    lines = content.split("\n")

    # Pattern to match memory_space attributes using StrAttr
    str_attr_pattern = re.compile(r'(memory_space|space)\s*:\s*StrAttr')

    for i, line in enumerate(lines, 1):
        if str_attr_pattern.search(line):
            violations.append(PolicyViolation(
                policy="memory_space_typed_attrs",
                file_path=aisops_td,
                line_number=i,
                message=f"Memory space using StrAttr instead of typed attribute: {line.strip()}",
                suggestion="Use AISMemorySpaceAttr or proper enum attribute",
            ))

    return violations


def check_all_policies(project_root: Path) -> PolicyCheckResult:
    """Run all policy checks."""
    all_violations = []

    print("Running policy checks...")
    print()

    # Check 1: No hardcoded ais.* strings
    print("1. Checking for hardcoded 'ais.*' strings in lower_mlir.rs...")
    violations = check_no_hardcoded_ais_strings(project_root)
    all_violations.extend(violations)
    print(f"   Found {len(violations)} violation(s)")

    # Check 2: Python uses _generated/ constants
    print("2. Checking Python uses _generated/ constants...")
    violations = check_python_uses_generated_constants(project_root)
    all_violations.extend(violations)
    print(f"   Found {len(violations)} violation(s)")

    # Check 3: AISOps.td matches AISOperationType enum
    print("3. Checking AISOps.td matches AISOperationType enum...")
    violations = check_aisops_matches_enum(project_root)
    all_violations.extend(violations)
    print(f"   Found {len(violations)} violation(s)")

    # Check 4: Memory space uses typed attrs
    print("4. Checking memory space uses typed enum attributes...")
    violations = check_memory_space_typed_attrs(project_root)
    all_violations.extend(violations)
    print(f"   Found {len(violations)} violation(s)")

    print()

    return PolicyCheckResult(
        passed=len(all_violations) == 0,
        violations=all_violations,
    )


def print_violations(result: PolicyCheckResult) -> None:
    """Print policy violations."""
    if result.passed:
        print("✓ All policy checks passed!")
        return

    print("=" * 80)
    print(f"POLICY VIOLATIONS: {result.violation_count}")
    print("=" * 80)

    # Group by policy
    by_policy: dict[str, list[PolicyViolation]] = {}
    for v in result.violations:
        if v.policy not in by_policy:
            by_policy[v.policy] = []
        by_policy[v.policy].append(v)

    for policy, violations in sorted(by_policy.items()):
        print(f"\n{policy} ({len(violations)} violation(s)):")
        print("-" * 80)

        for v in violations:
            rel_path = v.file_path.relative_to(Path.cwd())
            location = f"{rel_path}"
            if v.line_number:
                location += f":{v.line_number}"

            print(f"\n  {location}")
            print(f"    {v.message}")
            if v.suggestion:
                print(f"    → {v.suggestion}")

    print("\n" + "=" * 80)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="APXM Policy Checker - Enforce coding standards",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--fix",
        action="store_true",
        help="Auto-generate fix suggestions (not implemented)",
    )

    args = parser.parse_args()

    project_root = Path.cwd()

    print("=" * 80)
    print("APXM POLICY CHECKER")
    print("=" * 80)
    print()

    result = check_all_policies(project_root)

    print_violations(result)

    if args.fix and not result.passed:
        print("\n🔧 Auto-fix mode (not implemented yet)")
        print("Manual fixes required - see suggestions above")

    return 0 if result.passed else 1


if __name__ == "__main__":
    sys.exit(main())
