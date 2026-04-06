"""
dekk apxm models — list and inspect backend model health.

Commands:
  dekk apxm models list        List all models with health, latency, capabilities
  dekk apxm models health      Show only chat-capable models, sorted by health
"""

import os
import sys
from typing import Optional

import requests
from rich.console import Console
from rich.table import Table

from scripts import get_config


def register_commands(app):
    """Register models subcommands."""
    from dekk import Typer

    models_app = Typer(
        name="models",
        help="Inspect backend model health and capabilities",
        no_args_is_help=True,
    )

    @models_app.command(name="list")
    def models_list_cmd():
        """List all models with health, latency, and capabilities."""
        models_list()

    @models_app.command(name="health")
    def models_health_cmd():
        """Show only chat-capable models, sorted by health percentage."""
        models_health()

    app.add_typer(models_app)


def _fetch_models() -> Optional[list[dict]]:
    """Fetch model data from backend /models endpoint."""
    try:
        config = get_config()
    except Exception as e:
        Console().print(f"[red]Failed to load config:[/red] {e}")
        return None

    backends = config.get("backends", {})
    amd_backend = None

    for _name, backend in backends.items():
        if backend.get("type") == "onprem" and "llm.example.com" in backend.get("base_url", ""):
            amd_backend = backend
            break

    if not amd_backend:
        api_key = os.environ.get("BACKEND_API_KEY")
        if not api_key:
            console = Console()
            console.print("[red]Error:[/red] No Enterprise backend configured and BACKEND_API_KEY not set.")
            console.print("\nTo fix, either:")
            console.print("  1. Add enterprise backend via: [cyan]dekk apxm backend add corp-gateway --type onprem --protocol openai --endpoint https://llm.example.com/v1 --api-key <key>[/cyan]")
            console.print("  2. Set environment variable: [cyan]export BACKEND_API_KEY=<key>[/cyan]")
            return None
    else:
        headers = amd_backend.get("custom_headers", {})
        api_key = headers.get("X-Custom-Gateway-Key")
        if not api_key:
            Console().print("[red]Error:[/red] Enterprise backend configured but missing X-Custom-Gateway-Key in custom_headers")
            return None

    try:
        response = requests.get(
            "https://llm.example.com/models",
            headers={"X-Custom-Gateway-Key": api_key},
            timeout=10,
        )
        response.raise_for_status()
        return response.json().get("data", [])
    except requests.exceptions.RequestException as e:
        Console().print(f"[red]Failed to fetch models:[/red] {e}")
        return None


def _format_health(model: dict) -> str:
    """Format health percentage with color coding."""
    health_rate = model.get("healthRate")
    if not health_rate:
        return "[dim]N/A[/dim]"
    pct = health_rate.get("percentage", 0)
    text = f"{pct:.1f}%"
    if pct >= 95:
        return f"[green]{text}[/green]"
    elif pct >= 80:
        return f"[yellow]{text}[/yellow]"
    return f"[red]{text}[/red]"


def _get_health_pct(model: dict) -> float:
    """Extract health percentage for sorting (-1 if unavailable)."""
    health_rate = model.get("healthRate")
    if health_rate:
        return health_rate.get("percentage", 0)
    return -1


def _format_latency(model: dict) -> str:
    """Format latency in human-readable units."""
    avg_latency = model.get("averageLatency")
    if not avg_latency:
        return "[dim]N/A[/dim]"
    ms = avg_latency.get("latency", 0)
    if ms < 1000:
        return f"{int(ms)}ms"
    elif ms < 60000:
        return f"{ms/1000:.1f}s"
    return f"{ms/60000:.1f}m"


def _format_capabilities(model: dict, exclude: Optional[set[str]] = None) -> str:
    """Format capability list, optionally excluding some."""
    caps = model.get("capabilities", {})
    exclude = exclude or set()
    names = [name for name, enabled in caps.items() if enabled and name not in exclude]
    return ", ".join(names) if names else "[dim]None[/dim]"


def models_list():
    """List all models with health, latency, and capabilities."""
    console = Console()
    console.print("\n[bold cyan]Fetching backend model data...[/bold cyan]")
    models = _fetch_models()

    if models is None:
        sys.exit(1)
    if not models:
        console.print("[yellow]No models found.[/yellow]")
        return

    table = Table(title=f"LLM Models ({len(models)} total)")
    table.add_column("Model ID", style="cyan", no_wrap=True)
    table.add_column("Health", justify="right", style="green")
    table.add_column("Latency", justify="right")
    table.add_column("Capabilities", style="blue")
    table.add_column("Expires", style="dim")

    for model in models:
        table.add_row(
            model.get("id", "unknown"),
            _format_health(model),
            _format_latency(model),
            _format_capabilities(model),
            model.get("expires", "[dim]∞[/dim]"),
        )

    console.print(table)
    console.print(f"\n[dim]Endpoint:[/dim] https://llm.example.com/models")


def models_health():
    """Show only chat-capable models, sorted by health percentage (descending)."""
    console = Console()
    console.print("\n[bold cyan]Fetching backend model health...[/bold cyan]")
    models = _fetch_models()

    if models is None:
        sys.exit(1)
    if not models:
        console.print("[yellow]No models found.[/yellow]")
        return

    chat_models = [m for m in models if m.get("capabilities", {}).get("Chat", False)]
    if not chat_models:
        console.print("[yellow]No chat-capable models found.[/yellow]")
        return

    chat_models.sort(key=_get_health_pct, reverse=True)

    table = Table(title=f"Chat-Capable Models by Health ({len(chat_models)} total)")
    table.add_column("Model ID", style="cyan", no_wrap=True)
    table.add_column("Health", justify="right", style="green")
    table.add_column("Latency", justify="right")
    table.add_column("Capabilities", style="blue")

    for model in chat_models:
        table.add_row(
            model.get("id", "unknown"),
            _format_health(model),
            _format_latency(model),
            _format_capabilities(model, exclude={"Chat"}),
        )

    console.print(table)
    console.print(f"\n[dim]Showing {len(chat_models)} chat-capable models sorted by health[/dim]")
