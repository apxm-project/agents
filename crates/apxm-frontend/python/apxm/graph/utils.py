"""Shared graph utilities."""

from __future__ import annotations

from collections import defaultdict


def detect_cycle(
    node_ids: set[int] | set[str],
    edges: list[tuple[int | str, int | str]],
) -> int:
    """Run Kahn's topological sort and return the number of nodes NOT visited.

    Parameters
    ----------
    node_ids:
        Set of all node identifiers (ints or strings).
    edges:
        List of ``(from_id, to_id)`` pairs representing directed edges.

    Returns
    -------
    int
        ``0`` if the graph is a DAG.  A positive number indicates how many
        nodes are involved in one or more cycles.
    """
    adjacency: dict[int | str, list[int | str]] = defaultdict(list)
    in_degree: dict[int | str, int] = {nid: 0 for nid in node_ids}

    for src, dst in edges:
        if src in node_ids and dst in node_ids:
            adjacency[src].append(dst)
            in_degree[dst] = in_degree.get(dst, 0) + 1

    queue = [nid for nid, deg in in_degree.items() if deg == 0]
    visited = 0
    while queue:
        nid = queue.pop(0)
        visited += 1
        for neighbor in adjacency.get(nid, []):
            in_degree[neighbor] -= 1
            if in_degree[neighbor] == 0:
                queue.append(neighbor)

    return len(node_ids) - visited
