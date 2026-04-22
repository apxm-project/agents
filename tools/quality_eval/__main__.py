"""Command-line entry: ``python -m quality_eval --fixture <name> [--all] ...``.

Wraps ``runner.run_fixture`` / ``runner.sample_fixture``. Exit code:
    0   every requested fixture passed
    1   at least one fixture failed (rubric, judge, or budget)
    2   harness misuse (bad args, missing fixture root)
"""

from __future__ import annotations

import argparse
import sys

from .judge import make_judge
from .runner import format_result, list_fixtures, run_fixture, sample_fixture


def _build_parser() -> argparse.ArgumentParser:
    ap = argparse.ArgumentParser(prog="quality_eval")
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--fixture", help="single fixture name under tests/quality_fixtures/")
    g.add_argument("--all", action="store_true", help="run every discovered fixture")
    ap.add_argument("--opt", type=int, default=2, help="-O level passed to dekk apxm execute")
    ap.add_argument("--backend", default="configured",
                    help="backend label for reporting; actual selection lives in ~/.apxm/config.toml")
    ap.add_argument("--judge", choices=("none", "llm"), default="none",
                    help="LLM judge mode; 'none' uses NullJudge")
    ap.add_argument("--samples", type=int, default=1,
                    help="number of runs per fixture (stability sampling)")
    ap.add_argument("--threshold", type=int, default=0,
                    help="pass count required when --samples>1 (default ceil(samples/2)+1)")
    return ap


def _resolve_threshold(samples: int, threshold: int) -> int:
    if threshold > 0:
        return threshold
    # Default: majority — for samples=3 → 2, samples=5 → 3.
    return samples // 2 + 1


def main(argv: list[str] | None = None) -> int:
    args = _build_parser().parse_args(argv)
    judge = make_judge(args.judge, backend=args.backend if args.judge == "llm" else None)

    if args.all:
        names = list_fixtures()
        if not names:
            print("error: no fixtures found under tests/quality_fixtures/", file=sys.stderr)
            return 2
    else:
        names = [args.fixture]

    threshold = _resolve_threshold(args.samples, args.threshold)
    overall_ok = True
    for name in names:
        if args.samples > 1:
            stab = sample_fixture(name, args.opt, args.backend, judge,
                                  samples=args.samples, threshold=threshold)
            print(f"{name}: {stab.pass_count}/{args.samples} pass "
                  f"(threshold {threshold}) -> {'PASS' if stab.passed else 'FAIL'}")
            for r in stab.runs:
                print(format_result(r))
            if not stab.passed:
                overall_ok = False
        else:
            r = run_fixture(name, args.opt, args.backend, judge)
            print(format_result(r))
            if not r.passed:
                overall_ok = False

    return 0 if overall_ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
