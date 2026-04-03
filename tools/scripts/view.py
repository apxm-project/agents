#!/usr/bin/env python3
"""Standalone script: launch the interactive graph visualizer.

Invoked by dekk as: python tools/scripts/view.py <graph.json> [--no-open]
"""

import argparse
import signal
import subprocess
import sys
import webbrowser
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser(description="Open interactive graph visualizer")
    parser.add_argument("file", type=Path, help="Graph JSON file to visualize")
    parser.add_argument("--no-open", action="store_true", help="Don't auto-open the browser")
    args = parser.parse_args()

    # dekk runs with cwd=project_root, so resolve relative paths from there
    graph = args.file.resolve()

    if not graph.exists():
        print(f"error: file not found: {graph}", file=sys.stderr)
        return 1

    if graph.suffix != ".json":
        print(f"error: expected a .json file, got: {graph.suffix}", file=sys.stderr)
        return 1

    # Derive viewer_dir from this script's location: tools/scripts/ -> tools/graph-viewer/
    viewer_dir = Path(__file__).resolve().parent.parent / "graph-viewer"
    if not viewer_dir.exists():
        print(f"error: graph viewer not found at {viewer_dir}", file=sys.stderr)
        return 1

    # Auto-install node_modules if missing
    if not (viewer_dir / "node_modules").exists():
        print(":: Installing graph viewer dependencies...")
        result = subprocess.run(["npm", "install"], cwd=viewer_dir)
        if result.returncode != 0:
            print("error: failed to install graph viewer dependencies", file=sys.stderr)
            return 1

    url = "http://127.0.0.1:4174/"
    print(f":: Starting graph viewer for {graph.name}")
    print(f"   URL: {url}")
    print("   Press Ctrl+C to stop")

    if not args.no_open:
        webbrowser.open(url)

    cmd = ["npx", "tsx", "server.ts", "--graph", str(graph)]
    try:
        proc = subprocess.Popen(cmd, cwd=viewer_dir)
        for sig in (signal.SIGINT, signal.SIGTERM):
            signal.signal(sig, lambda s, f: proc.send_signal(s))
        return proc.wait()
    except KeyboardInterrupt:
        proc.terminate()
        proc.wait()
        return 0


if __name__ == "__main__":
    sys.exit(main())
