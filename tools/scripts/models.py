"""
dekk apxm models — list and inspect backend model health.

Commands:
  dekk apxm models list        List all models with health, latency, capabilities
  dekk apxm models health      Show only chat-capable models, sorted by health
"""

import json
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
    """
    Fetch model data from backend /models endpoint.

    Returns:
        List of model entries, or None if fetch fails.
    """
    try:
        config = get_config()
    except Exception as e:
        Console().print(f"[red]Failed to load config:[/red] {e}")
        return None

    # Extract backend credentials from config
    endpoint = "https://llm-api.backend.com/models"

    # Look for backend backend in backends section
    backends = config.get("backends", {})
    backend_backend = None

    for name, backend in backends.items():
        if backend.get("type") == "onprem" and "llm-api.backend.com" in backend.get("base_url", ""):
            backend_backend = backend
            break

    if not backend_backend:
        Console().print("[yellow]No backend backend found in config. Checking for API key in environment...[/yellow]")
        api_key = os.environ.get("backend_API_KEY")
        if not api_key:
            Console().print("[red]Error:[/red] No backend backend configured and backend_API_KEY not set.")
            Console().print("\nTo fix, either:")
            Console().print("  1. Add backend backend via: [cyan]dekk apxm backend add backend-onprem --type onprem --protocol openai --endpoint https://llm-api.backend.com/OnPrem --api-key <key>[/cyan]")
            Console().print("  2. Set environment variable: [cyan]export backend_API_KEY=<key>[/cyan]")
            return None
    else:
        # Extract API key from custom headers
        headers = backend_backend.get("custom_headers", {})
        api_key = headers.get("X-Api-Key")

        if not api_key:
            Console().print("[red]Error:[/red] backend backend configured but missing X-Api-Key in custom_headers")
            return None

    # Fetch models
    try:
        response = requests.get(
            endpoint,
            headers={"X-Api-Key": api_key},
            timeout=10,
        )
        response.raise_for_status()
        data = response.json()
        return data.get("data", [])
    except requests.exceptions.RequestException as e:
        Console().print(f"[red]Failed to fetch models:[/red] {e}")
        return None


def models_list():
    """
    List all models with health, latency, and capabilities.

    Displays a rich table with:
    - Model ID
    - Health percentage
    - Average latency
    - Capabilities (Chat, Multimodal, etc.)
    - Expiration date
    """
    console = Console()

    console.print("\n[bold cyan]Fetching backend model data...[/bold cyan]")
    models = _fetch_models()

    if models is None:
        sys.exit(1)

    if not models:
        console.print("[yellow]No models found.[/yellow]")
        return

    # Build table
    table = Table(title=f"backend LLM Models ({len(models)} total)")
    table.add_column("Model ID", style="cyan", no_wrap=True)
    table.add_column("Health", justify="right", style="green")
    table.add_column("Latency", justify="right")
    table.add_column("Capabilities", style="blue")
    table.add_column("Expires", style="dim")

    for model in models:
        model_id = model.get("id", "unknown")

        # Health
        health_rate = model.get("healthRate")
        if health_rate:
            health_pct = health_rate.get("percentage", 0)
            health_str = f"{health_pct:.1f}%"
            # Color-code health
            if health_pct >= 95:
                health_str = f"[green]{health_str}[/green]"
            elif health_pct >= 80:
                health_str = f"[yellow]{health_str}[/yellow]"
            else:
                health_str = f"[red]{health_str}[/red]"
        else:
            health_str = "[dim]N/A[/dim]"

        # Latency
        avg_latency = model.get("averageLatency")
        if avg_latency:
            latency_ms = avg_latency.get("latency", 0)
            # Convert to human-readable
            if latency_ms < 1000:
                latency_str = f"{int(latency_ms)}ms"
            elif latency_ms < 60000:
                latency_str = f"{latency_ms/1000:.1f}s"
            else:
                latency_str = f"{latency_ms/60000:.1f}m"
        else:
            latency_str = "[dim]N/A[/dim]"

        # Capabilities
        caps = model.get("capabilities", {})
        cap_list = []
        for cap_name, enabled in caps.items():
            if enabled:
                cap_list.append(cap_name)
        capabilities = ", ".join(cap_list) if cap_list else "[dim]None[/dim]"

        # Expiration
        expires = model.get("expires", "[dim]∞[/dim]")

        table.add_row(model_id, health_str, latency_str, capabilities, expires)

    console.print(table)
    console.print(f"\n[dim]Endpoint:[/dim] https://llm-api.backend.com/models")


def models_health():
    """
    Show only chat-capable models, sorted by health percentage (descending).

    Filters to models with Chat capability and displays them sorted by health.
    """
    console = Console()

    console.print("\n[bold cyan]Fetching backend model health...[/bold cyan]")
    models = _fetch_models()

    if models is None:
        sys.exit(1)

    if not models:
        console.print("[yellow]No models found.[/yellow]")
        return

    # Filter to chat-capable models
    chat_models = [
        m for m in models
        if m.get("capabilities", {}).get("Chat", False)
    ]

    if not chat_models:
        console.print("[yellow]No chat-capable models found.[/yellow]")
        return

    # Sort by health (descending)
    def get_health(model):
        health_rate = model.get("healthRate")
        if health_rate:
            return health_rate.get("percentage", 0)
        return -1  # No health data goes to bottom

    chat_models.sort(key=get_health, reverse=True)

    # Build table
    table = Table(title=f"Chat-Capable Models by Health ({len(chat_models)} total)")
    table.add_column("Model ID", style="cyan", no_wrap=True)
    table.add_column("Health", justify="right", style="green")
    table.add_column("Latency", justify="right")
    table.add_column("Capabilities", style="blue")

    for model in chat_models:
        model_id = model.get("id", "unknown")

        # Health
        health_pct = get_health(model)
        if health_pct >= 0:
            health_str = f"{health_pct:.1f}%"
            # Color-code health
            if health_pct >= 95:
                health_str = f"[green]{health_str}[/green]"
            elif health_pct >= 80:
                health_str = f"[yellow]{health_str}[/yellow]"
            else:
                health_str = f"[red]{health_str}[/red]"
        else:
            health_str = "[dim]N/A[/dim]"

        # Latency
        avg_latency = model.get("averageLatency")
        if avg_latency:
            latency_ms = avg_latency.get("latency", 0)
            # Convert to human-readable
            if latency_ms < 1000:
                latency_str = f"{int(latency_ms)}ms"
            elif latency_ms < 60000:
                latency_str = f"{latency_ms/1000:.1f}s"
            else:
                latency_str = f"{latency_ms/60000:.1f}m"
        else:
            latency_str = "[dim]N/A[/dim]"

        # Capabilities
        caps = model.get("capabilities", {})
        cap_list = []
        for cap_name, enabled in caps.items():
            if enabled and cap_name != "Chat":  # Skip Chat since it's implicit
                cap_list.append(cap_name)
        capabilities = ", ".join(cap_list) if cap_list else "[dim]None[/dim]"

        table.add_row(model_id, health_str, latency_str, capabilities)

    console.print(table)
    console.print(f"\n[dim]Showing {len(chat_models)} chat-capable models sorted by health[/dim]")
