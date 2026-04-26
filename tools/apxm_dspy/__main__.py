"""Entry point: python3 -m apxm_dspy < request.json > response.json"""

import json
import sys
from contextlib import redirect_stdout

from .types import ResponseKey, Status


def main():
    try:
        request = json.load(sys.stdin)
    except json.JSONDecodeError as e:
        json.dump(
            {ResponseKey.STATUS: Status.ERROR, ResponseKey.ERROR: f"Invalid JSON input: {e}"},
            sys.stdout,
        )
        sys.exit(1)

    try:
        import dspy  # noqa: F401
    except ImportError:
        json.dump(
            {
                ResponseKey.STATUS: Status.ERROR,
                ResponseKey.ERROR: "dspy-ai not installed. Run: pip install dspy-ai>=2.6.0",
            },
            sys.stdout,
        )
        sys.exit(1)

    from .optimizer import optimize_templates

    try:
        with redirect_stdout(sys.stderr):
            result = optimize_templates(request)
        json.dump(result, sys.stdout)
    except Exception as e:
        json.dump({ResponseKey.STATUS: Status.ERROR, ResponseKey.ERROR: str(e)}, sys.stdout)
        sys.exit(1)


if __name__ == "__main__":
    main()
