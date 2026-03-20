"""Bootstrap dekk from PyPI for APXM."""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parent.parent
CLI_PATH = Path(__file__).resolve().with_name("apxm_cli.py")
BOOTSTRAP_ENV_DIR = PROJECT_ROOT / ".apxm" / "bootstrap-venv"
DEKK_PACKAGE = "dekk"


def _print(msg: str) -> None:
    print(msg, file=sys.stderr)


def _has_dekk_cli() -> bool:
    """Return True when dekk's Typer surface is importable."""
    try:
        import dekk  # noqa: F401
        from dekk import Argument, Exit, Option, Typer  # noqa: F401
        return True
    except ImportError:
        return False


def _run(cmd: list[str], *, cwd: Path | None = None) -> None:
    subprocess.run(cmd, cwd=cwd, check=True)


def _venv_python() -> Path:
    scripts_dir = "Scripts" if os.name == "nt" else "bin"
    primary = BOOTSTRAP_ENV_DIR / scripts_dir / ("python.exe" if os.name == "nt" else "python3")
    if primary.exists():
        return primary
    return BOOTSTRAP_ENV_DIR / scripts_dir / "python"


def _in_bootstrap_env() -> bool:
    return Path(sys.prefix).resolve() == BOOTSTRAP_ENV_DIR.resolve()


def _ensure_bootstrap_venv() -> Path:
    python = _venv_python()
    if python.exists():
        return python

    BOOTSTRAP_ENV_DIR.parent.mkdir(parents=True, exist_ok=True)
    _print(f"[apxm] Creating bootstrap environment at {BOOTSTRAP_ENV_DIR}...")
    try:
        _run([sys.executable, "-m", "venv", str(BOOTSTRAP_ENV_DIR)], cwd=PROJECT_ROOT)
    except subprocess.CalledProcessError:
        python = _venv_python()
        if python.exists():
            return python
        raise
    return _venv_python()


def _python_has_dekk_cli(python: Path) -> bool:
    code = "import dekk; from dekk import Argument, Exit, Option, Typer"
    result = subprocess.run([str(python), "-c", code], capture_output=True)
    return result.returncode == 0


def _install_dekk_cli(python: Path) -> None:
    _print(f"[apxm] Installing {DEKK_PACKAGE} into the bootstrap environment...")
    _run([str(python), "-m", "pip", "install", "--upgrade", "pip"], cwd=PROJECT_ROOT)
    _run([str(python), "-m", "pip", "install", "--upgrade", DEKK_PACKAGE], cwd=PROJECT_ROOT)


def ensure_dekk_bootstrap() -> None:
    """Ensure fresh clones can import dekk before the CLI starts."""
    try:
        if _has_dekk_cli():
            return

        python = _ensure_bootstrap_venv()
        if not _python_has_dekk_cli(python):
            _install_dekk_cli(python)

        if not _in_bootstrap_env():
            os.execv(str(python), [str(python), str(CLI_PATH), *sys.argv[1:]])

        if not _has_dekk_cli():
            raise RuntimeError("dekk bootstrap completed but dekk is still unavailable")
    except SystemExit:
        raise
    except Exception as exc:
        _print(f"[apxm] Failed to bootstrap dekk: {exc}")
        raise SystemExit(1) from exc
