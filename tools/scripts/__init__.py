"""APXM CLI Scripts - Modular command implementations.

Environment setup (conda, MLIR, LD_LIBRARY_PATH) is handled by dekk's
auto-activation before any command runs. Scripts just need project paths.
"""

import subprocess
from dataclasses import dataclass
from pathlib import Path

from dekk import Exit
from dekk import print_error, print_info

from . import messages as msg

CARGO_FEATURES = "driver,metrics"


def _find_project_root(start: Path, marker: str = "Cargo.toml") -> Path | None:
    """Walk up from *start* looking for a directory containing *marker*."""
    for parent in (start, *start.parents):
        if (parent / marker).exists():
            return parent
    return None


@dataclass
class ApxmConfig:
    """APXM project paths."""

    apxm_dir: Path
    target_dir: Path

    @classmethod
    def detect(cls) -> "ApxmConfig":
        """Auto-detect APXM project root from the scripts package location."""
        start = Path(__file__).resolve().parent
        apxm_dir = _find_project_root(start)

        # Fallback: tools/ is directly under the repo root.
        if apxm_dir is None and start.name == "scripts":
            apxm_dir = start.parent.parent

        if apxm_dir is None:
            apxm_dir = start

        return cls(apxm_dir=apxm_dir, target_dir=apxm_dir / "target")

    @property
    def compiler_bin(self) -> Path:
        """Path to the compiled apxm binary."""
        return self.target_dir / "release" / "apxm"


def get_config() -> ApxmConfig:
    """Get the APXM configuration."""
    return ApxmConfig.detect()


def ensure_binary(config: ApxmConfig) -> None:
    """Check that the APXM binary is built. Raises Exit(1) if missing."""
    if not config.compiler_bin.exists():
        print_error("APXM binary not built!")
        print_info("Run: apxm build")
        raise Exit(1)


def build_apxm_cmd(config: ApxmConfig, subcommand: str, extra_args: list[str],
                    cargo: bool = False) -> list[str]:
    """Build the command list for running an apxm subcommand."""
    if cargo:
        return [
            "cargo", "run", "-p", "apxm-cli",
            "--features", CARGO_FEATURES, "--release",
            "--", subcommand, *extra_args,
        ]
    ensure_binary(config)
    return [str(config.compiler_bin), subcommand, *extra_args]


def resolve_path(path: "Path | None", cwd: Path) -> "Path | None":
    """Resolve an optional path relative to cwd."""
    if path is not None and not path.is_absolute():
        return (cwd / path).resolve()
    return path


def run_apxm(config: ApxmConfig, cmd: list[str], env: dict | None = None) -> int:
    """Run a command in the APXM directory and return the exit code."""
    result = subprocess.run(cmd, cwd=config.apxm_dir, env=env)
    return result.returncode
