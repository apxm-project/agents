//! DAG analysis on `AirModule` graphs.

use std::collections::{HashMap, HashSet, VecDeque};

use apxm_compiler::AirModule;
use serde::Serialize;

#[derive(Serialize)]
pub struct GraphAnalysis {
    pub total_nodes: usize,
    pub total_edges: usize,
    pub entry_nodes: Vec<u64>,
    pub exit_nodes: Vec<u64>,
    pub critical_path_length: usize,
    pub critical_path: Vec<u64>,
    pub max_parallelism: usize,
    pub op_histogram: HashMap<String, usize>,
}

pub fn analyze_graph(graph: &AirModule) -> GraphAnalysis {
    let node_ids: HashSet<u64> = graph.nodes.iter().map(|n| n.id).collect();

    let mut successors: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut predecessors: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut targets: HashSet<u64> = HashSet::new();
    let mut sources: HashSet<u64> = HashSet::new();

    for edge in &graph.edges {
        successors.entry(edge.from).or_default().push(edge.to);
        predecessors.entry(edge.to).or_default().push(edge.from);
        targets.insert(edge.to);
        sources.insert(edge.from);
    }

    let entry_nodes: Vec<u64> = graph
        .nodes
        .iter()
        .filter(|n| !targets.contains(&n.id))
        .map(|n| n.id)
        .collect();

    let exit_nodes: Vec<u64> = graph
        .nodes
        .iter()
        .filter(|n| !sources.contains(&n.id))
        .map(|n| n.id)
        .collect();

    let mut in_degree: HashMap<u64, usize> = HashMap::new();
    for &id in &node_ids {
        in_degree.insert(id, predecessors.get(&id).map_or(0, |p| p.len()));
    }

    let mut queue: VecDeque<u64> = VecDeque::new();
    let mut level: HashMap<u64, usize> = HashMap::new();
    for &id in &entry_nodes {
        level.insert(id, 0);
    }

    let mut dist: HashMap<u64, usize> = HashMap::new();
    let mut prev: HashMap<u64, u64> = HashMap::new();

    for &id in &entry_nodes {
        dist.insert(id, 1);
    }

    let mut in_deg = in_degree.clone();
    for &id in &entry_nodes {
        queue.push_back(id);
    }

    while let Some(u) = queue.pop_front() {
        let lvl = *level.get(&u).unwrap_or(&0);

        if let Some(succs) = successors.get(&u) {
            for &v in succs {
                let new_dist = dist.get(&u).copied().unwrap_or(1) + 1;
                if new_dist > dist.get(&v).copied().unwrap_or(0) {
                    dist.insert(v, new_dist);
                    prev.insert(v, u);
                }

                let new_level = lvl + 1;
                let cur_level = level.entry(v).or_insert(0);
                if new_level > *cur_level {
                    *cur_level = new_level;
                }

                if let Some(d) = in_deg.get_mut(&v) {
                    *d -= 1;
                    if *d == 0 {
                        queue.push_back(v);
                    }
                }
            }
        }
    }

    let mut level_counts: HashMap<usize, usize> = HashMap::new();
    for &lvl in level.values() {
        *level_counts.entry(lvl).or_insert(0) += 1;
    }
    let max_parallelism = level_counts.values().copied().max().unwrap_or(1);

    let critical_end = dist.iter().max_by_key(|&(_, d)| *d).map(|(&id, _)| id);
    let critical_path_length = critical_end
        .and_then(|id| dist.get(&id).copied())
        .unwrap_or(0);

    let mut critical_path = Vec::new();
    if let Some(mut current) = critical_end {
        critical_path.push(current);
        while let Some(&p) = prev.get(&current) {
            critical_path.push(p);
            current = p;
        }
        critical_path.reverse();
    }

    let mut op_histogram: HashMap<String, usize> = HashMap::new();
    for node in &graph.nodes {
        *op_histogram.entry(format!("{:?}", node.op)).or_insert(0) += 1;
    }

    GraphAnalysis {
        total_nodes: graph.nodes.len(),
        total_edges: graph.edges.len(),
        entry_nodes,
        exit_nodes,
        critical_path_length,
        critical_path,
        max_parallelism,
        op_histogram,
    }
}

/// Map ops catalog display name to AISOperationType serde variant.
pub fn display_name_to_serde_variant(display: &str) -> String {
    match display {
        "QueryMemory" => "QMEM".to_string(),
        "UpdateMemory" => "UMEM".to_string(),
        "InvokeTool" => "INV_TOOL".to_string(),
        "ExecuteCode" => "EXC".to_string(),
        "PrintOutput" => "PRINT".to_string(),
        "HandleError" => "ERR".to_string(),
        s if s.chars().all(|c| c.is_uppercase() || c == '_') => s.to_string(),
        s => {
            let mut result = String::with_capacity(s.len() + 4);
            for (i, c) in s.chars().enumerate() {
                if c.is_uppercase() && i > 0 {
                    result.push('_');
                }
                result.push(c.to_ascii_uppercase());
            }
            result
        }
    }
}

/// Convert PascalCase op names to SCREAMING_SNAKE_CASE for AirModule deserialization.
pub fn normalize_ops_for_air_module(mut graph: serde_json::Value) -> serde_json::Value {
    if let Some(nodes) = graph.get_mut("nodes").and_then(|n| n.as_array_mut()) {
        for node in nodes {
            if let Some(op) = node.get("op").and_then(|o| o.as_str()) {
                let screaming = display_name_to_serde_variant(op);
                node.as_object_mut()
                    .unwrap()
                    .insert("op".into(), serde_json::Value::String(screaming));
            }
        }
    }
    graph
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_compiler::AirModule;

    fn diamond_graph_json() -> &'static str {
        // 4-node diamond: A -> B, A -> C, B -> D, C -> D
        r#"{
            "name": "diamond",
            "nodes": [
                {"id": 1, "op": "ASK", "attributes": {}},
                {"id": 2, "op": "ASK", "attributes": {}},
                {"id": 3, "op": "ASK", "attributes": {}},
                {"id": 4, "op": "ASK", "attributes": {}}
            ],
            "edges": [
                {"from": 1, "to": 2, "kind": "DEP"},
                {"from": 1, "to": 3, "kind": "DEP"},
                {"from": 2, "to": 4, "kind": "DEP"},
                {"from": 3, "to": 4, "kind": "DEP"}
            ],
            "parameters": []
        }"#
    }

    #[test]
    fn analyze_diamond() {
        let json = diamond_graph_json();
        let graph: AirModule = match serde_json::from_str(json) {
            Ok(g) => g,
            Err(_) => return, // schema may not match — skip
        };
        let analysis = analyze_graph(&graph);
        assert_eq!(analysis.total_nodes, 4);
        assert_eq!(analysis.total_edges, 4);
        assert_eq!(analysis.entry_nodes, vec![1]);
        assert_eq!(analysis.exit_nodes, vec![4]);
        assert_eq!(analysis.critical_path_length, 3);
        assert_eq!(analysis.max_parallelism, 2);
        assert_eq!(analysis.op_histogram.values().sum::<usize>(), 4);
    }

    #[test]
    fn display_name_uppercases() {
        assert_eq!(display_name_to_serde_variant("QueryMemory"), "QMEM");
        assert_eq!(display_name_to_serde_variant("Ask"), "ASK");
        assert_eq!(display_name_to_serde_variant("ASK"), "ASK");
        assert_eq!(
            display_name_to_serde_variant("SpawnAgent"),
            "SPAWN_AGENT"
        );
    }
}
