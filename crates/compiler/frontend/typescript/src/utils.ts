/** Kahn's-algorithm topological sort, mirroring apxm.utils.topological_sort. */
export function topologicalSort(nodeIds: readonly number[], edges: readonly (readonly [number, number])[]): number[] {
  const adjacency = new Map<number, number[]>();
  const inDegree = new Map<number, number>();
  for (const id of nodeIds) inDegree.set(id, 0);

  for (const [src, dst] of edges) {
    if (inDegree.has(src) && inDegree.has(dst)) {
      if (!adjacency.has(src)) adjacency.set(src, []);
      adjacency.get(src)!.push(dst);
      inDegree.set(dst, (inDegree.get(dst) ?? 0) + 1);
    }
  }

  const queue: number[] = [];
  for (const [id, deg] of inDegree) if (deg === 0) queue.push(id);

  const order: number[] = [];
  while (queue.length > 0) {
    const id = queue.shift()!;
    order.push(id);
    for (const next of adjacency.get(id) ?? []) {
      inDegree.set(next, (inDegree.get(next) ?? 0) - 1);
      if (inDegree.get(next) === 0) queue.push(next);
    }
  }

  if (order.length !== nodeIds.length) {
    throw new Error("graph contains a cycle; cannot produce a topological order");
  }
  return order;
}

/** Return the number of nodes involved in a cycle (0 if the graph is a DAG). */
export function detectCycle(nodeIds: readonly number[], edges: readonly (readonly [number, number])[]): number {
  try {
    topologicalSort(nodeIds, edges);
    return 0;
  } catch {
    // Re-run Kahn's without throwing to count unresolved nodes.
    const inDegree = new Map<number, number>();
    for (const id of nodeIds) inDegree.set(id, 0);
    const adjacency = new Map<number, number[]>();
    for (const [src, dst] of edges) {
      if (inDegree.has(src) && inDegree.has(dst)) {
        if (!adjacency.has(src)) adjacency.set(src, []);
        adjacency.get(src)!.push(dst);
        inDegree.set(dst, (inDegree.get(dst) ?? 0) + 1);
      }
    }
    const queue: number[] = [];
    for (const [id, deg] of inDegree) if (deg === 0) queue.push(id);
    let visited = 0;
    while (queue.length > 0) {
      const id = queue.shift()!;
      visited += 1;
      for (const next of adjacency.get(id) ?? []) {
        inDegree.set(next, (inDegree.get(next) ?? 0) - 1);
        if (inDegree.get(next) === 0) queue.push(next);
      }
    }
    return nodeIds.length - visited;
  }
}
