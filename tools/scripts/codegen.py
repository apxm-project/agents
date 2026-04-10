"""Code generation commands for APXM CLI."""

from pathlib import Path
from typing import Optional

from dekk import Typer, Exit, Option
from dekk import print_error, print_step, print_success

from . import build_apxm_cmd, get_config, resolve_path, run_apxm


def register_commands(app: Typer) -> None:
    """Register code generation subcommands."""
    codegen_app = Typer(
        name="codegen",
        help="Generate frontend assets from Rust-owned registries",
        no_args_is_help=True,
    )

    @codegen_app.command(name="frontend")
    def codegen_frontend(
        output_dir: Optional[Path] = Option(
            None,
            "--output-dir",
            help="Output directory for generated Python files",
        ),
        cargo: bool = Option(
            False,
            "--cargo",
            help="Use cargo run instead of the pre-built binary",
        ),
    ):
        """Generate the Python frontend bindings into apxm/_generated."""
        config = get_config()
        cwd = Path.cwd()
        output_dir = resolve_path(output_dir, cwd)

        print_step("Generating frontend bindings...")

        extra = ["frontend"]
        if output_dir is not None:
            extra.extend(["--output-dir", str(output_dir)])

        cmd = build_apxm_cmd(config, "codegen", extra, cargo=cargo)
        rc = run_apxm(config, cmd)

        if rc == 0:
            target = output_dir or (config.apxm_dir / "crates/compiler/apxm-frontend/python/apxm/_generated")
            print_success(f"Generated frontend bindings in {target}")
        else:
            print_error("Frontend code generation failed!")
        raise Exit(rc)

    app.add_typer(codegen_app, name="codegen")
