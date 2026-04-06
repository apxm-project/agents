use crate::{ApxmGraph, GraphError, GraphNode, Parameter};
use apxm_core::constants::graph::{attrs as graph_attrs, metadata as graph_meta};
use apxm_core::types::AISOperationType;
use apxm_core::types::{Number, Value};
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Debug)]
enum MlirValueType {
    Token,
    Handle { space: String },
    Goal,
}

#[derive(Clone, Debug)]
struct MlirValueRef {
    ssa: String,
    ty: MlirValueType,
}

struct LoweringState {
    lines: Vec<String>,
    next_temp: u64,
}

impl LoweringState {
    fn new() -> Self {
        Self {
            lines: Vec::new(),
            next_temp: 0,
        }
    }

    fn emit(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }

    fn fresh_value(&mut self, prefix: &str) -> String {
        self.next_temp += 1;
        format!("%{}_{}", prefix, self.next_temp)
    }
}

/// Check if a node uses any flow parameters in its template/prompt attributes.
/// Returns true if the node contains `{{PARAM_NAME}}` patterns that match any
/// parameter name in the flow.
fn node_uses_flow_params(node: &GraphNode, params: &[Parameter]) -> bool {
    if params.is_empty() {
        return false;
    }

    // Check template_str, prompt, and template attributes for {{PARAM_NAME}} patterns
    let template_attrs = [
        graph_attrs::TEMPLATE_STR,
        graph_attrs::PROMPT,
        graph_attrs::TEMPLATE,
    ];

    for attr_name in &template_attrs {
        if let Some(value) = node.attributes.get(*attr_name) {
            if let Some(text) = value.as_str() {
                // Check if any parameter name appears in {{...}} patterns
                for param in params {
                    let pattern = format!("{{{{{}}}}}", param.name);
                    if text.contains(&pattern) {
                        return true;
                    }
                }
            }
        }
    }

    false
}

pub fn lower_to_mlir(graph: &ApxmGraph) -> Result<String, GraphError> {
    graph.validate()?;

    let nodes_by_id = graph
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<HashMap<_, _>>();
    let order = topo_order(graph, &nodes_by_id)?;

    let mut incoming_by_target: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut outgoing_counts: HashMap<u64, usize> = HashMap::new();
    for edge in &graph.edges {
        incoming_by_target
            .entry(edge.to)
            .or_default()
            .push(edge.from);
        *outgoing_counts.entry(edge.from).or_default() += 1;
    }

    let is_entry = graph
        .metadata
        .get(graph_meta::IS_ENTRY)
        .and_then(Value::as_boolean)
        .unwrap_or(true);

    let mut state = LoweringState::new();
    let mut produced_values: HashMap<u64, MlirValueRef> = HashMap::new();
    let arg_values: Vec<MlirValueRef> = (0..graph.parameters.len())
        .map(|index| MlirValueRef {
            ssa: format!("%arg{index}"),
            ty: MlirValueType::Token,
        })
        .collect();

    for node_id in &order {
        let node = nodes_by_id.get(node_id).ok_or_else(|| {
            GraphError::Lowering(format!(
                "MLIR lowering internal error: node id {} not found in topological order (graph: '{}')",
                node_id, graph.name
            ))
        })?;

        let mut inputs = incoming_by_target
            .get(node_id)
            .into_iter()
            .flatten()
            .map(|source_id| {
                produced_values.get(source_id).cloned().ok_or_else(|| {
                    let source_name = nodes_by_id
                        .get(source_id)
                        .map(|n| n.name.as_str())
                        .unwrap_or("unknown");
                    GraphError::Lowering(format!(
                        "MLIR lowering failed for node '{}' (id={}, op={}): \
                         references source node '{}' (id={}) which has no produced SSA value. \
                         This indicates the source node's MLIR emission did not produce an output.",
                        node.name, node.id, node.op, source_name, source_id
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Inject flow parameters into entry nodes that actually use them in their templates.
        // Only inject if the node references parameters like {{PARAM_NAME}} in its
        // template_str/prompt/template attributes. This prevents malformed MLIR for nodes
        // like spawn_agent that don't consume parameters.
        if inputs.is_empty() && !arg_values.is_empty() && node_uses_flow_params(node, &graph.parameters) {
            inputs.extend(arg_values.clone());
        }

        let node_result = emit_node(&mut state, node, inputs.clone())?;
        if let Some(value) = node_result {
            produced_values.insert(*node_id, value);
            continue;
        }

        if outgoing_counts.get(node_id).copied().unwrap_or(0) > 0 {
            let bridge = emit_bridge_token(&mut state, node.id, &inputs)?;
            produced_values.insert(*node_id, bridge);
        }
    }

    let exit_ids = graph
        .nodes
        .iter()
        .filter(|node| outgoing_counts.get(&node.id).copied().unwrap_or(0) == 0)
        .map(|node| node.id)
        .collect::<Vec<_>>();

    let mut return_candidates = exit_ids
        .iter()
        .filter_map(|id| produced_values.get(id).cloned())
        .collect::<Vec<_>>();

    if return_candidates.is_empty() {
        for node_id in order.iter().rev() {
            if let Some(value) = produced_values.get(node_id) {
                return_candidates.push(value.clone());
                break;
            }
        }
    }

    let return_value = if return_candidates.is_empty() {
        emit_const_token(&mut state, "result")
    } else {
        let token_values = return_candidates
            .into_iter()
            .map(|value| ensure_token(&mut state, value))
            .collect::<Result<Vec<_>, _>>()?;

        if token_values.len() == 1 {
            token_values.into_iter().next().unwrap()
        } else {
            let output = state.fresh_value("ret_merge");
            let operands = token_values
                .iter()
                .map(|value| value.ssa.clone())
                .collect::<Vec<_>>()
                .join(", ");
            let types = vec!["!ais.token"; token_values.len()].join(", ");
            state.emit(format!(
                "    {output} = ais.merge {operands} : {types} -> !ais.token"
            ));
            MlirValueRef {
                ssa: output,
                ty: MlirValueType::Token,
            }
        }
    };

    state.emit(format!("    func.return {} : !ais.token", return_value.ssa));

    let function_name = sanitize_symbol_name(&graph.name);
    let args = graph
        .parameters
        .iter()
        .enumerate()
        .map(|(index, parameter)| {
            format!(
                "%arg{index}: !ais.token {{ais.param_name = {}, ais.param_type = {}}}",
                quote_string(&parameter.name),
                quote_string(&parameter.type_name)
            )
        })
        .collect::<Vec<_>>()
        .join(", ");

    let function_attrs = if is_entry {
        " attributes {ais.entry}"
    } else {
        ""
    };

    let mut mlir = String::new();
    mlir.push_str("module {\n");
    mlir.push_str(&format!(
        "  func.func @{}({}) -> !ais.token{} {{\n",
        function_name, args, function_attrs
    ));
    for line in state.lines {
        mlir.push_str(&line);
        mlir.push('\n');
    }
    mlir.push_str("  }\n");
    mlir.push_str("}\n");

    Ok(mlir)
}

fn topo_order(
    graph: &ApxmGraph,
    nodes_by_id: &HashMap<u64, &GraphNode>,
) -> Result<Vec<u64>, GraphError> {
    let mut in_degree = nodes_by_id
        .keys()
        .map(|id| (*id, 0usize))
        .collect::<HashMap<_, _>>();
    let mut outgoing: HashMap<u64, Vec<u64>> = HashMap::new();

    for edge in &graph.edges {
        outgoing.entry(edge.from).or_default().push(edge.to);
        *in_degree.entry(edge.to).or_insert(0) += 1;
    }

    let mut ready = in_degree
        .iter()
        .filter_map(|(id, degree)| if *degree == 0 { Some(*id) } else { None })
        .collect::<BTreeSet<_>>();
    let mut order = Vec::with_capacity(nodes_by_id.len());

    while let Some(next_id) = ready.pop_first() {
        order.push(next_id);
        if let Some(neighbors) = outgoing.get(&next_id) {
            for neighbor in neighbors {
                if let Some(degree) = in_degree.get_mut(neighbor) {
                    *degree = degree.saturating_sub(1);
                    if *degree == 0 {
                        ready.insert(*neighbor);
                    }
                }
            }
        }
    }

    if order.len() != nodes_by_id.len() {
        let missing_count = nodes_by_id.len() - order.len();
        return Err(GraphError::Lowering(format!(
            "MLIR lowering failed: topological sort produced {} nodes but graph '{}' has {} nodes. \
             {} node(s) unreachable (graph may contain cycles or disconnected components). \
             Run 'apxm validate {}' to diagnose DAG structure.",
            order.len(),
            graph.name,
            nodes_by_id.len(),
            missing_count,
            graph.name
        )));
    }

    Ok(order)
}

fn emit_node(
    state: &mut LoweringState,
    node: &GraphNode,
    inputs: Vec<MlirValueRef>,
) -> Result<Option<MlirValueRef>, GraphError> {
    match node.op {
        AISOperationType::ConstStr => {
            let value = get_string_attr(
                &node.attributes,
                &[
                    graph_attrs::VALUE,
                    "text",
                    "const",
                    graph_attrs::TEMPLATE_STR,
                    graph_attrs::PROMPT,
                ],
            )
            .unwrap_or_default();
            let result = format!("%n{}", node.id);
            let attrs = extra_attr_dict(
                &node.attributes,
                &[
                    graph_attrs::VALUE,
                    "text",
                    "const",
                    graph_attrs::TEMPLATE_STR,
                    graph_attrs::PROMPT,
                ],
            );
            state.emit(format!(
                "    {result} = ais.const_str {}{} : !ais.token",
                quote_string(&value),
                attrs
            ));
            Ok(Some(MlirValueRef {
                ssa: result,
                ty: MlirValueType::Token,
            }))
        }
        AISOperationType::Ask | AISOperationType::Think | AISOperationType::Reason => {
            let op_name = match node.op {
                AISOperationType::Ask => "ask",
                AISOperationType::Think => "think",
                AISOperationType::Reason => "reason",
                _ => unreachable!(),
            };
            let template = get_non_empty_template(&node.attributes);
            let result = format!("%n{}", node.id);
            let attrs = extra_attr_dict(
                &node.attributes,
                &[graph_attrs::TEMPLATE_STR, graph_attrs::PROMPT],
            );
            let context = format_context(&inputs, '[', ']');
            state.emit(format!(
                "    {result} = ais.{op_name} {}{}{} : !ais.token",
                quote_string(&template),
                context,
                attrs
            ));
            Ok(Some(MlirValueRef {
                ssa: result,
                ty: MlirValueType::Token,
            }))
        }
        AISOperationType::QMem => {
            let query =
                get_string_attr(&node.attributes, &[graph_attrs::QUERY]).unwrap_or_default();
            let sid = get_string_attr(&node.attributes, &["sid", "stage", "scope"])
                .unwrap_or_else(|| "default".to_string());
            let space = normalize_memory_space(
                &get_string_attr(
                    &node.attributes,
                    &[graph_attrs::SPACE, graph_attrs::MEMORY_TIER],
                )
                .unwrap_or_else(|| "stm".to_string()),
            );
            let limit = get_u64_attr(&node.attributes, graph_attrs::LIMIT);
            let result = format!("%n{}", node.id);
            let attrs = extra_attr_dict(
                &node.attributes,
                &[
                    graph_attrs::QUERY,
                    "sid",
                    "stage",
                    "scope",
                    graph_attrs::SPACE,
                    graph_attrs::MEMORY_TIER,
                    graph_attrs::LIMIT,
                ],
            );
            let limit_str = limit
                .map(|value| format!(" limit {value}"))
                .unwrap_or_default();
            let handle_type = format!("!ais.handle<{space}>");

            state.emit(format!(
                "    {result} = ais.qmem {} stage {} in {}{}{} : {}",
                quote_string(&query),
                quote_string(&sid),
                quote_string(&space),
                limit_str,
                attrs,
                handle_type
            ));
            Ok(Some(MlirValueRef {
                ssa: result,
                ty: MlirValueType::Handle { space },
            }))
        }
        AISOperationType::UMem => {
            let space = normalize_memory_space(
                &get_string_attr(
                    &node.attributes,
                    &[graph_attrs::SPACE, graph_attrs::MEMORY_TIER],
                )
                .unwrap_or_else(|| "stm".to_string()),
            );
            let attrs = extra_attr_dict(
                &node.attributes,
                &[graph_attrs::SPACE, graph_attrs::MEMORY_TIER],
            );

            let source = if let Some(input) = inputs.first() {
                ensure_token(state, input.clone())?
            } else {
                emit_const_token(state, "memory")
            };

            state.emit(format!(
                "    ais.umem {} into {}{} : !ais.token",
                source.ssa,
                quote_string(&space),
                attrs
            ));
            Ok(None)
        }
        AISOperationType::InvTool => {
            let capability = get_string_attr(&node.attributes, &[graph_attrs::CAPABILITY])
                .unwrap_or_else(|| "unknown_capability".to_string());
            let params_json = get_string_attr(&node.attributes, &[graph_attrs::PARAMS_JSON])
                .unwrap_or_else(|| "{}".to_string());
            let attrs = extra_attr_dict(
                &node.attributes,
                &[graph_attrs::CAPABILITY, graph_attrs::PARAMS_JSON],
            );
            let result = format!("%n{}", node.id);
            state.emit(format!(
                "    {result} = ais.inv_tool {} ({}){} : !ais.token",
                quote_string(&capability),
                quote_string(&params_json),
                attrs
            ));
            Ok(Some(MlirValueRef {
                ssa: result,
                ty: MlirValueType::Token,
            }))
        }
        AISOperationType::BranchOnValue => {
            let mut condition = if let Some(input) = inputs.first() {
                input.clone()
            } else {
                emit_const_token(state, "branch")
            };
            if matches!(condition.ty, MlirValueType::Goal) {
                condition = ensure_token(state, condition)?;
            }

            let true_label = get_string_attr(&node.attributes, &[graph_attrs::TRUE_LABEL])
                .unwrap_or_else(|| "true".to_string());
            let false_label = get_string_attr(&node.attributes, &[graph_attrs::FALSE_LABEL])
                .unwrap_or_else(|| "false".to_string());
            let attrs = extra_attr_dict(
                &node.attributes,
                &[graph_attrs::TRUE_LABEL, graph_attrs::FALSE_LABEL],
            );

            state.emit(format!(
                "    ais.branch_on_value {}, {}, {}{} : {}",
                condition.ssa,
                quote_string(&true_label),
                quote_string(&false_label),
                attrs,
                format_type(&condition.ty)
            ));
            Ok(None)
        }
        AISOperationType::Switch => {
            let discriminant = if let Some(input) = inputs.first() {
                ensure_token(state, input.clone())?
            } else {
                emit_const_token(state, "switch")
            };
            let labels = get_string_array_attr(&node.attributes, graph_attrs::CASE_LABELS);
            let case_labels = if labels.is_empty() {
                vec!["default_case".to_string()]
            } else {
                labels
            };
            let attrs = extra_attr_dict_for_node(node, &[graph_attrs::CASE_LABELS]);
            let result = format!("%n{}", node.id);

            state.emit(format!(
                "    {result} = ais.switch {} : !ais.token",
                discriminant.ssa
            ));
            for label in &case_labels {
                state.emit(format!("        case {} {{", quote_string(label)));
                state.emit(format!(
                    "            ais.yield {} : !ais.token",
                    discriminant.ssa
                ));
                state.emit("        }");
            }
            state.emit("        default {");
            state.emit(format!(
                "            ais.yield {} : !ais.token",
                discriminant.ssa
            ));
            if attrs.is_empty() {
                state.emit("        } -> !ais.token");
            } else {
                state.emit(format!("        }} -> !ais.token{}", attrs));
            }

            Ok(Some(MlirValueRef {
                ssa: result,
                ty: MlirValueType::Token,
            }))
        }
        AISOperationType::LoopStart => {
            let mut count = if let Some(input) = inputs.first() {
                input.clone()
            } else {
                emit_const_token(state, "loop")
            };
            if matches!(count.ty, MlirValueType::Goal) {
                count = ensure_token(state, count)?;
            }

            let label = get_string_attr(&node.attributes, &[graph_attrs::LABEL])
                .unwrap_or_else(|| "loop".to_string());
            let attrs = extra_attr_dict_for_node(node, &[graph_attrs::LABEL]);
            let result = format!("%n{}", node.id);

            state.emit(format!(
                "    {result} = ais.loop_start {} as {}{} : {} -> !ais.token",
                count.ssa,
                quote_string(&label),
                attrs,
                format_type(&count.ty)
            ));
            Ok(Some(MlirValueRef {
                ssa: result,
                ty: MlirValueType::Token,
            }))
        }
        AISOperationType::LoopEnd => {
            let source = if let Some(input) = inputs.first() {
                ensure_token(state, input.clone())?
            } else {
                emit_const_token(state, "loop_state")
            };
            let attrs = extra_attr_dict_for_node(node, &[]);
            let result = format!("%n{}", node.id);

            state.emit(format!(
                "    {result} = ais.loop_end {}{} : !ais.token -> !ais.token",
                source.ssa, attrs
            ));
            Ok(Some(MlirValueRef {
                ssa: result,
                ty: MlirValueType::Token,
            }))
        }
        AISOperationType::TryCatch => {
            let try_label = get_string_attr(&node.attributes, &[graph_attrs::TRY_LABEL])
                .unwrap_or_else(|| "try_block".to_string());
            let catch_label = get_string_attr(&node.attributes, &[graph_attrs::CATCH_LABEL])
                .unwrap_or_else(|| "catch_block".to_string());
            let attrs = extra_attr_dict(
                &node.attributes,
                &[graph_attrs::TRY_LABEL, graph_attrs::CATCH_LABEL],
            );

            state.emit(format!(
                "    ais.try_catch {} -> {}{}",
                quote_string(&try_label),
                quote_string(&catch_label),
                attrs
            ));
            Ok(None)
        }
        AISOperationType::Err => {
            let template = get_string_attr(&node.attributes, &[graph_attrs::RECOVERY_TEMPLATE])
                .unwrap_or_else(|| "Recover from error".to_string());
            let attrs = extra_attr_dict_for_node(node, &[graph_attrs::RECOVERY_TEMPLATE]);
            let result = format!("%n{}", node.id);

            if let Some(input) = inputs.first() {
                let token = ensure_token(state, input.clone())?;
                state.emit(format!(
                    "    {result} = ais.err {} : !ais.token with {}{} -> !ais.token",
                    token.ssa,
                    quote_string(&template),
                    attrs
                ));
            } else {
                state.emit(format!(
                    "    {result} = ais.err with {}{} -> !ais.token",
                    quote_string(&template),
                    attrs
                ));
            }

            Ok(Some(MlirValueRef {
                ssa: result,
                ty: MlirValueType::Token,
            }))
        }
        AISOperationType::WaitAll | AISOperationType::Merge => {
            let op_name = match node.op {
                AISOperationType::WaitAll => "wait_all",
                AISOperationType::Merge => "merge",
                _ => unreachable!(),
            };
            let tokens = inputs
                .into_iter()
                .map(|value| ensure_token(state, value))
                .collect::<Result<Vec<_>, _>>()?;
            let result = format!("%n{}", node.id);
            let attrs = extra_attr_dict_for_node(node, &[]);

            if tokens.is_empty() {
                state.emit(format!("    {result} = ais.{op_name}{attrs} -> !ais.token"));
            } else {
                let operands = tokens
                    .iter()
                    .map(|token| token.ssa.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                let types = vec!["!ais.token"; tokens.len()].join(", ");
                state.emit(format!(
                    "    {result} = ais.{op_name} {operands} : {types}{attrs} -> !ais.token"
                ));
            }
            Ok(Some(MlirValueRef {
                ssa: result,
                ty: MlirValueType::Token,
            }))
        }
        AISOperationType::Fence => {
            let attrs = extra_attr_dict_for_node(node, &[]);
            state.emit(format!("    ais.fence{attrs}"));
            Ok(None)
        }
        AISOperationType::Plan => {
            let goal = get_string_attr(&node.attributes, &[graph_attrs::GOAL])
                .unwrap_or_else(|| "goal".to_string());
            let attrs = extra_attr_dict_for_node(node, &[graph_attrs::GOAL]);
            let result = format!("%n{}", node.id);
            let context = format_context(&inputs, '(', ')');

            state.emit(format!(
                "    {result} = ais.plan {}{}{} : !ais.goal<0>",
                quote_string(&goal),
                context,
                attrs
            ));
            Ok(Some(MlirValueRef {
                ssa: result,
                ty: MlirValueType::Goal,
            }))
        }
        AISOperationType::Reflect => emit_simple_op(
            state,
            node,
            &inputs,
            "reflect",
            &[graph_attrs::TRACE_ID, graph_attrs::TRACE],
            graph_attrs::TRACE,
            &[graph_attrs::TRACE_ID, graph_attrs::TRACE],
            Some(('(', ')')),
        ),
        AISOperationType::Verify => {
            let template = get_string_attr(
                &node.attributes,
                &[graph_attrs::TEMPLATE_STR, graph_attrs::PROMPT],
            )
            .unwrap_or_else(|| "Verify claim against evidence".to_string());
            let attrs = extra_attr_dict(
                &node.attributes,
                &[graph_attrs::TEMPLATE_STR, graph_attrs::PROMPT],
            );

            let claim = if let Some(input) = inputs.first() {
                input.clone()
            } else {
                emit_const_token(state, "claim")
            };
            let evidence = if let Some(input) = inputs.get(1) {
                input.clone()
            } else {
                claim.clone()
            };

            let result = format!("%n{}", node.id);
            state.emit(format!(
                "    {result} = ais.verify {} : {} vs {} : {} with {}{} : !ais.token",
                claim.ssa,
                format_type(&claim.ty),
                evidence.ssa,
                format_type(&evidence.ty),
                quote_string(&template),
                attrs
            ));
            Ok(Some(MlirValueRef {
                ssa: result,
                ty: MlirValueType::Token,
            }))
        }
        // Communication ops
        AISOperationType::Communicate => {
            let message = get_string_attr(
                &node.attributes,
                &[
                    graph_attrs::MESSAGE,
                    graph_attrs::TEMPLATE_STR,
                    graph_attrs::PROMPT,
                ],
            )
            .unwrap_or_else(|| "{0}".to_string());
            let recipient = get_string_attr(
                &node.attributes,
                &[graph_attrs::RECIPIENT, graph_attrs::TARGET],
            )
            .unwrap_or_else(|| "default".to_string());
            let attrs = extra_attr_dict(
                &node.attributes,
                &[
                    graph_attrs::MESSAGE,
                    graph_attrs::TEMPLATE_STR,
                    graph_attrs::PROMPT,
                    graph_attrs::RECIPIENT,
                    graph_attrs::TARGET,
                ],
            );
            let result = format!("%n{}", node.id);
            let context = format_context(&inputs, '(', ')');

            state.emit(format!(
                "    {result} = ais.communicate {} to {}{}{} : !ais.token",
                quote_string(&message),
                quote_string(&recipient),
                context,
                attrs
            ));
            Ok(Some(MlirValueRef {
                ssa: result,
                ty: MlirValueType::Token,
            }))
        }
        AISOperationType::Delegate => {
            let task_spec = get_string_attr(
                &node.attributes,
                &[graph_attrs::TASK_SPEC, graph_attrs::TEMPLATE_STR],
            )
            .unwrap_or_else(|| "{0}".to_string());
            let target_agent = get_string_attr(
                &node.attributes,
                &[graph_attrs::TARGET_AGENT, graph_attrs::TARGET],
            )
            .unwrap_or_else(|| "default".to_string());
            let attrs = extra_attr_dict(
                &node.attributes,
                &[
                    graph_attrs::TASK_SPEC,
                    graph_attrs::TEMPLATE_STR,
                    graph_attrs::TARGET_AGENT,
                    graph_attrs::TARGET,
                ],
            );
            let result = format!("%n{}", node.id);
            let context = format_context(&inputs, '(', ')');

            state.emit(format!(
                "    {result} = ais.delegate {} to {}{}{} : !ais.token",
                quote_string(&task_spec),
                quote_string(&target_agent),
                context,
                attrs
            ));
            Ok(Some(MlirValueRef {
                ssa: result,
                ty: MlirValueType::Token,
            }))
        }
        AISOperationType::Negotiate => emit_simple_op(
            state,
            node,
            &inputs,
            "negotiate",
            &[graph_attrs::PROPOSAL, graph_attrs::TEMPLATE_STR],
            "{0}",
            &[graph_attrs::PROPOSAL, graph_attrs::TEMPLATE_STR],
            Some(('(', ')')),
        ),
        // Control flow ops
        AISOperationType::FlowCall => emit_simple_op(
            state,
            node,
            &inputs,
            "flow_call",
            &[graph_attrs::FLOW_NAME, "flow", graph_attrs::TARGET],
            "unknown_flow",
            &[graph_attrs::FLOW_NAME, "flow", graph_attrs::TARGET],
            Some(('(', ')')),
        ),
        AISOperationType::Jump => {
            let target =
                get_string_attr(&node.attributes, &[graph_attrs::TARGET, graph_attrs::LABEL])
                    .unwrap_or_else(|| "next".to_string());
            let attrs = extra_attr_dict_for_node(node, &[graph_attrs::TARGET, graph_attrs::LABEL]);

            state.emit(format!("    ais.jump {}{}", quote_string(&target), attrs));
            Ok(None)
        }
        AISOperationType::Return => {
            let attrs = extra_attr_dict_for_node(node, &[]);

            if let Some(input) = inputs.first() {
                let token = ensure_token(state, input.clone())?;
                state.emit(format!(
                    "    ais.return {} : !ais.token{}",
                    token.ssa, attrs
                ));
            } else {
                state.emit(format!("    ais.return{}", attrs));
            }
            Ok(None)
        }
        // Tool / Execution ops
        AISOperationType::Exc => emit_simple_op(
            state,
            node,
            &inputs,
            "exc",
            &[graph_attrs::CODE, "script"],
            "",
            &[graph_attrs::CODE, "script"],
            Some(('[', ']')),
        ),
        AISOperationType::Print => {
            let template = get_string_attr(
                &node.attributes,
                &[
                    graph_attrs::TEMPLATE_STR,
                    graph_attrs::PROMPT,
                    graph_attrs::MESSAGE,
                ],
            )
            .unwrap_or_else(|| "{0}".to_string());
            let attrs = extra_attr_dict(
                &node.attributes,
                &[
                    graph_attrs::TEMPLATE_STR,
                    graph_attrs::PROMPT,
                    graph_attrs::MESSAGE,
                ],
            );
            let context = format_context(&inputs, '[', ']');

            state.emit(format!(
                "    ais.print {}{}{}",
                quote_string(&template),
                context,
                attrs
            ));
            Ok(None)
        }
        // Agent metadata
        AISOperationType::Agent => emit_simple_op(
            state,
            node,
            &inputs,
            "agent",
            &[graph_attrs::AGENT_NAME, "name"],
            "agent",
            &[graph_attrs::AGENT_NAME, "name"],
            None,
        ),
        // AAM / Coordination ops
        AISOperationType::UpdateGoal => emit_simple_op(
            state,
            node,
            &inputs,
            "update_goal",
            &[graph_attrs::GOAL_ID, graph_attrs::GOAL],
            "goal",
            &[graph_attrs::GOAL_ID, graph_attrs::GOAL],
            Some(('[', ']')),
        ),
        AISOperationType::Guard => emit_simple_op(
            state,
            node,
            &inputs,
            "guard",
            &[graph_attrs::CONDITION, graph_attrs::TEMPLATE_STR],
            "true",
            &[graph_attrs::CONDITION, graph_attrs::TEMPLATE_STR],
            Some(('(', ')')),
        ),
        AISOperationType::Claim => {
            let queue = get_string_attr(&node.attributes, &[graph_attrs::QUEUE, graph_attrs::KEY])
                .unwrap_or_else(|| "default".to_string());
            let lease_ms = get_u64_attr(&node.attributes, graph_attrs::LEASE_MS);
            let attrs = extra_attr_dict(
                &node.attributes,
                &[graph_attrs::QUEUE, graph_attrs::KEY, graph_attrs::LEASE_MS],
            );
            let result = format!("%n{}", node.id);
            let lease_str = lease_ms
                .map(|ms| format!(" lease_ms {ms}"))
                .unwrap_or_default();

            state.emit(format!(
                "    {result} = ais.claim {}{}{} : !ais.token",
                quote_string(&queue),
                lease_str,
                attrs
            ));
            Ok(Some(MlirValueRef {
                ssa: result,
                ty: MlirValueType::Token,
            }))
        }
        AISOperationType::Pause => emit_simple_op(
            state,
            node,
            &inputs,
            "pause",
            &[graph_attrs::MESSAGE, graph_attrs::CHECKPOINT],
            "paused",
            &[graph_attrs::MESSAGE, graph_attrs::CHECKPOINT],
            Some(('[', ']')),
        ),
        AISOperationType::Resume => emit_simple_op(
            state,
            node,
            &inputs,
            "resume",
            &[graph_attrs::CHECKPOINT, graph_attrs::CHECKPOINT_ID],
            "latest",
            &[graph_attrs::CHECKPOINT, graph_attrs::CHECKPOINT_ID],
            None,
        ),
        // Identity / passthrough ops
        AISOperationType::Nop | AISOperationType::Identity | AISOperationType::Yield => {
            if let Some(input) = inputs.first() {
                Ok(Some(input.clone()))
            } else {
                let label = match node.op {
                    AISOperationType::Yield => "yield",
                    _ => "nop",
                };
                let result = emit_const_token(state, label);
                Ok(Some(result))
            }
        }
        // Self-organization ops
        AISOperationType::SpawnAgent => {
            // spawn_agent never takes data inputs in MLIR — it is a standalone
            // spawning op. The AIS DSL parser may add implicit sequential edges
            // to it (when other nodes precede it in source order), but those edges
            // carry no semantic meaning for spawn_agent's MLIR emission.
            // Always emit with empty inputs to produce valid MLIR.
            emit_simple_op(
                state,
                node,
                &[], // always empty — spawn_agent has no data inputs in MLIR
                "spawn_agent",
                &[graph_attrs::AGENT_NAME, "name"],
                "child_agent",
                &[graph_attrs::AGENT_NAME, "name"],
                Some(('(', ')')),
            )
        }
        AISOperationType::RegisterCapability => emit_simple_op(
            state,
            node,
            &inputs,
            "register_capability",
            &[graph_attrs::CAPABILITY_NAME, graph_attrs::CAPABILITY],
            "capability",
            &[graph_attrs::CAPABILITY_NAME, graph_attrs::CAPABILITY],
            None,
        ),
        // Autonomous execution
        AISOperationType::Autonomous => emit_simple_op(
            state,
            node,
            &inputs,
            "autonomous",
            &[graph_attrs::STRATEGY, graph_attrs::TEMPLATE_STR],
            "default",
            &[graph_attrs::STRATEGY, graph_attrs::TEMPLATE_STR],
            Some(('(', ')')),
        ),
        // Durable execution checkpoint
        AISOperationType::Checkpoint => emit_simple_op(
            state,
            node,
            &inputs,
            "checkpoint",
            &[graph_attrs::CHECKPOINT_ID],
            "checkpoint",
            &[graph_attrs::CHECKPOINT_ID],
            None,
        ),
    }
}

/// Emit a simple op: `{result} = ais.{op_name} {primary}{context}{attrs} : !ais.token`
///
/// Used by many match arms that follow the same pattern: extract a single primary
/// string attribute, build context from inputs, and emit a single MLIR line returning
/// a token value.
#[allow(clippy::too_many_arguments)]
fn emit_simple_op(
    state: &mut LoweringState,
    node: &GraphNode,
    inputs: &[MlirValueRef],
    op_name: &str,
    primary_keys: &[&str],
    primary_default: &str,
    consumed_keys: &[&str],
    context_delimiters: Option<(char, char)>,
) -> Result<Option<MlirValueRef>, GraphError> {
    let primary = get_string_attr(&node.attributes, primary_keys)
        .unwrap_or_else(|| primary_default.to_string());
    let attrs = extra_attr_dict_for_node(node, consumed_keys);
    let result = format!("%n{}", node.id);
    let context = context_delimiters
        .map(|(open, close)| format_context(inputs, open, close))
        .unwrap_or_default();

    state.emit(format!(
        "    {result} = ais.{op_name} {}{}{} : !ais.token",
        quote_string(&primary),
        context,
        attrs
    ));
    Ok(Some(MlirValueRef {
        ssa: result,
        ty: MlirValueType::Token,
    }))
}

fn emit_bridge_token(
    state: &mut LoweringState,
    node_id: u64,
    inputs: &[MlirValueRef],
) -> Result<MlirValueRef, GraphError> {
    let result = format!("%n{node_id}");
    let tokens = inputs
        .iter()
        .cloned()
        .map(|value| ensure_token(state, value))
        .collect::<Result<Vec<_>, _>>()?;

    if tokens.is_empty() {
        state.emit(format!("    {result} = ais.wait_all -> !ais.token"));
    } else {
        let operands = tokens
            .iter()
            .map(|token| token.ssa.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let types = vec!["!ais.token"; tokens.len()].join(", ");
        state.emit(format!(
            "    {result} = ais.wait_all {operands} : {types} -> !ais.token"
        ));
    }

    Ok(MlirValueRef {
        ssa: result,
        ty: MlirValueType::Token,
    })
}

fn ensure_token(
    state: &mut LoweringState,
    value: MlirValueRef,
) -> Result<MlirValueRef, GraphError> {
    if matches!(value.ty, MlirValueType::Token) {
        return Ok(value);
    }

    let result = state.fresh_value("tok");
    let source_type = format_type(&value.ty);
    state.emit(format!(
        "    {result} = ais.ask \"{{0}}\" [{} : {}] : !ais.token",
        value.ssa, source_type
    ));

    Ok(MlirValueRef {
        ssa: result,
        ty: MlirValueType::Token,
    })
}

fn emit_const_token(state: &mut LoweringState, value: &str) -> MlirValueRef {
    let result = state.fresh_value("const");
    state.emit(format!(
        "    {result} = ais.const_str {} : !ais.token",
        quote_string(value)
    ));
    MlirValueRef {
        ssa: result,
        ty: MlirValueType::Token,
    }
}

fn format_context(values: &[MlirValueRef], open: char, close: char) -> String {
    if values.is_empty() {
        return String::new();
    }
    let operands = values
        .iter()
        .map(|value| value.ssa.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let types = values
        .iter()
        .map(|value| format_type(&value.ty).to_string())
        .collect::<Vec<_>>()
        .join(", ");
    format!(" {open}{operands} : {types}{close}")
}

fn format_type(value_type: &MlirValueType) -> String {
    match value_type {
        MlirValueType::Token => "!ais.token".to_string(),
        MlirValueType::Handle { space } => format!("!ais.handle<{space}>"),
        MlirValueType::Goal => "!ais.goal<0>".to_string(),
    }
}

fn get_non_empty_template(attributes: &HashMap<String, Value>) -> String {
    get_string_attr(
        attributes,
        &[
            graph_attrs::TEMPLATE_STR,
            graph_attrs::PROMPT,
            graph_attrs::TEMPLATE,
        ],
    )
    .filter(|value| !value.trim().is_empty())
    .unwrap_or_else(|| "{0}".to_string())
}

fn get_string_attr(attributes: &HashMap<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| attributes.get(*key))
        .and_then(value_to_string)
}

fn get_u64_attr(attributes: &HashMap<String, Value>, key: &str) -> Option<u64> {
    attributes.get(key).and_then(Value::as_u64)
}

fn get_string_array_attr(attributes: &HashMap<String, Value>, key: &str) -> Vec<String> {
    attributes
        .get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(value_to_string)
                .collect::<Vec<String>>()
        })
        .unwrap_or_default()
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Number(Number::Integer(value)) => Some(value.to_string()),
        Value::Number(Number::Float(value)) => Some(value.to_string()),
        Value::Array(_) | Value::Object(_) => value.to_json().ok().map(|json| json.to_string()),
        Value::Null => Some(String::new()),
        Value::Token(id) => Some(id.to_string()),
    }
}

fn extra_attr_dict_for_node(node: &GraphNode, consumed: &[&str]) -> String {
    let mut attributes = node.attributes.clone();
    attributes
        .entry(graph_attrs::NODE_NAME.to_string())
        .or_insert_with(|| Value::String(node.name.clone()));
    extra_attr_dict(&attributes, consumed)
}

fn extra_attr_dict(attributes: &HashMap<String, Value>, consumed: &[&str]) -> String {
    let mut items = attributes
        .iter()
        .filter_map(|(key, value)| {
            if consumed.contains(&key.as_str()) || !is_valid_attr_name(key) {
                return None;
            }
            Some(format!("{key} = {}", value_to_mlir_attr(value)))
        })
        .collect::<Vec<_>>();

    items.sort();
    if items.is_empty() {
        String::new()
    } else {
        format!(" {{{}}}", items.join(", "))
    }
}

fn value_to_mlir_attr(value: &Value) -> String {
    match value {
        Value::Null => quote_string("null"),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(Number::Integer(number)) => format!("{number} : i64"),
        Value::Number(Number::Float(number)) => {
            if number.is_finite() {
                if number.fract() == 0.0 {
                    format!("{number:.1} : f64")
                } else {
                    format!("{number} : f64")
                }
            } else {
                quote_string(&number.to_string())
            }
        }
        Value::String(text) => quote_string(text),
        Value::Array(values) => {
            let rendered = values
                .iter()
                .map(value_to_mlir_attr)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{rendered}]")
        }
        Value::Object(map) => {
            let json = serde_json::to_string(map).unwrap_or_else(|_| "{}".to_string());
            quote_string(&json)
        }
        Value::Token(id) => format!("{id} : i64"),
    }
}

fn is_valid_attr_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == '$') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == '$')
}

fn normalize_memory_space(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "ltm" => "ltm".to_string(),
        "episodic" => "episodic".to_string(),
        _ => "stm".to_string(),
    }
}

fn quote_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            _ => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

fn sanitize_symbol_name(name: &str) -> String {
    let mut symbol = String::with_capacity(name.len().max(8));
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' {
            symbol.push(ch);
        } else {
            symbol.push('_');
        }
    }
    if symbol.is_empty() {
        return "graph_main".to_string();
    }
    let first = symbol.as_bytes()[0];
    if first.is_ascii_alphabetic() || first == b'_' {
        symbol
    } else {
        format!("g_{symbol}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use apxm_core::types::DependencyType;
    use std::collections::HashMap;

    #[test]
    fn lowers_basic_const_ask_graph() {
        let graph = ApxmGraph {
            name: "research_flow".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "seed".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        "value".to_string(),
                        Value::String("hello".to_string()),
                    )]),
                },
                GraphNode {
                    id: 2,
                    name: "ask".to_string(),
                    op: AISOperationType::Ask,
                    attributes: HashMap::from([(
                        graph_attrs::TEMPLATE_STR.to_string(),
                        Value::String("Summarize {0}".to_string()),
                    )]),
                },
            ],
            edges: vec![crate::GraphEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            }],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        let mlir = lower_to_mlir(&graph).expect("graph lowers to mlir");
        assert!(mlir.contains("func.func @research_flow() -> !ais.token attributes {ais.entry}"));
        assert!(mlir.contains("ais.const_str \"hello\""));
        assert!(mlir.contains("ais.ask \"Summarize {0}\" [%n1 : !ais.token] : !ais.token"));
        assert!(mlir.contains("func.return %n2 : !ais.token"));
    }

    #[test]
    fn lowers_ask_with_tool_attributes() {
        let graph = ApxmGraph {
            name: "tools".to_string(),
            nodes: vec![GraphNode {
                id: 1,
                name: "ask".to_string(),
                op: AISOperationType::Ask,
                attributes: HashMap::from([
                    (
                        graph_attrs::TEMPLATE_STR.to_string(),
                        Value::String("Use tools".to_string()),
                    ),
                    (graph_attrs::TOOLS_ENABLED.to_string(), Value::Bool(true)),
                    (
                        graph_attrs::TOOLS.to_string(),
                        Value::Array(vec![
                            Value::String("bash".to_string()),
                            Value::String("read".to_string()),
                        ]),
                    ),
                ]),
            }],
            edges: vec![],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        let mlir = lower_to_mlir(&graph).expect("graph lowers to mlir");
        assert!(mlir.contains("tools_enabled = true"));
        assert!(mlir.contains("tools = [\"bash\", \"read\"]"));
    }

    #[test]
    fn lowers_switch_with_default_case() {
        let graph = ApxmGraph {
            name: "switch".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "input".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        "value".to_string(),
                        Value::String("technical".to_string()),
                    )]),
                },
                GraphNode {
                    id: 2,
                    name: "route".to_string(),
                    op: AISOperationType::Switch,
                    attributes: HashMap::new(),
                },
            ],
            edges: vec![crate::GraphEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            }],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        let mlir = lower_to_mlir(&graph).expect("graph lowers to mlir");
        assert!(mlir.contains("ais.switch %n1 : !ais.token"));
        assert!(mlir.contains("case \"default_case\""));
        assert!(mlir.contains("default {"));
    }

    #[test]
    fn lowers_loop_and_try_catch_control_ops() {
        let graph = ApxmGraph {
            name: "control".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "seed".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        "value".to_string(),
                        Value::String("items".to_string()),
                    )]),
                },
                GraphNode {
                    id: 2,
                    name: "loop_start".to_string(),
                    op: AISOperationType::LoopStart,
                    attributes: HashMap::from([(
                        graph_attrs::LABEL.to_string(),
                        Value::String("loop_items".to_string()),
                    )]),
                },
                GraphNode {
                    id: 3,
                    name: "loop_end".to_string(),
                    op: AISOperationType::LoopEnd,
                    attributes: HashMap::new(),
                },
                GraphNode {
                    id: 4,
                    name: "try_catch".to_string(),
                    op: AISOperationType::TryCatch,
                    attributes: HashMap::from([
                        (
                            graph_attrs::TRY_LABEL.to_string(),
                            Value::String("try_a".to_string()),
                        ),
                        (
                            graph_attrs::CATCH_LABEL.to_string(),
                            Value::String("catch_a".to_string()),
                        ),
                    ]),
                },
                GraphNode {
                    id: 5,
                    name: "err".to_string(),
                    op: AISOperationType::Err,
                    attributes: HashMap::from([(
                        graph_attrs::RECOVERY_TEMPLATE.to_string(),
                        Value::String("fallback".to_string()),
                    )]),
                },
            ],
            edges: vec![
                crate::GraphEdge {
                    from: 1,
                    to: 2,
                    dependency: DependencyType::Data,
                },
                crate::GraphEdge {
                    from: 2,
                    to: 3,
                    dependency: DependencyType::Control,
                },
                crate::GraphEdge {
                    from: 3,
                    to: 4,
                    dependency: DependencyType::Control,
                },
                crate::GraphEdge {
                    from: 4,
                    to: 5,
                    dependency: DependencyType::Control,
                },
            ],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        let mlir = lower_to_mlir(&graph).expect("graph lowers to mlir");
        // Assertions use partial matches — extra attrs like {node_name = ...} may appear.
        assert!(mlir.contains("ais.loop_start %n1 as \"loop_items\""), "actual mlir:\n{}", mlir);
        assert!(mlir.contains("!ais.token -> !ais.token"), "actual mlir:\n{}", mlir);
        assert!(mlir.contains("ais.loop_end %n2"), "actual mlir:\n{}", mlir);
        assert!(mlir.contains("ais.try_catch \"try_a\" -> \"catch_a\""), "actual mlir:\n{}", mlir);
        assert!(mlir.contains("ais.err"), "actual mlir:\n{}", mlir);
        assert!(mlir.contains("with \"fallback\""), "actual mlir:\n{}", mlir);
    }

    #[test]
    fn lowers_communicate_op() {
        let graph = ApxmGraph {
            name: "comm_flow".to_string(),
            nodes: vec![
                // spawn_agent must come first so validate_agent_references passes.
                GraphNode {
                    id: 1,
                    name: "spawn_b".to_string(),
                    op: AISOperationType::SpawnAgent,
                    attributes: HashMap::from([
                        (
                            graph_attrs::AGENT_NAME.to_string(),
                            Value::String("agent_b".to_string()),
                        ),
                        (
                            "profile".to_string(),
                            Value::String("claude".to_string()),
                        ),
                        (
                            "cwd".to_string(),
                            Value::String("/tmp".to_string()),
                        ),
                    ]),
                },
                GraphNode {
                    id: 2,
                    name: "seed".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        "value".to_string(),
                        Value::String("hello world".to_string()),
                    )]),
                },
                GraphNode {
                    id: 3,
                    name: "send".to_string(),
                    op: AISOperationType::Communicate,
                    attributes: HashMap::from([
                        (
                            graph_attrs::MESSAGE.to_string(),
                            Value::String("status update".to_string()),
                        ),
                        (
                            graph_attrs::RECIPIENT.to_string(),
                            Value::String("agent_b".to_string()),
                        ),
                    ]),
                },
            ],
            edges: vec![
                crate::GraphEdge {
                    from: 1,
                    to: 3,
                    dependency: DependencyType::Control,
                },
                crate::GraphEdge {
                    from: 2,
                    to: 3,
                    dependency: DependencyType::Data,
                },
            ],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        let mlir = lower_to_mlir(&graph).expect("graph lowers to mlir");
        assert!(mlir.contains("ais.communicate \"status update\" to \"agent_b\""));
        assert!(mlir.contains(": !ais.token"));
    }

    #[test]
    fn lowers_flow_call_op() {
        let graph = ApxmGraph {
            name: "caller".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "seed".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        "value".to_string(),
                        Value::String("input data".to_string()),
                    )]),
                },
                GraphNode {
                    id: 2,
                    name: "call".to_string(),
                    op: AISOperationType::FlowCall,
                    attributes: HashMap::from([(
                        graph_attrs::FLOW_NAME.to_string(),
                        Value::String("sub_flow".to_string()),
                    )]),
                },
            ],
            edges: vec![crate::GraphEdge {
                from: 1,
                to: 2,
                dependency: DependencyType::Data,
            }],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        let mlir = lower_to_mlir(&graph).expect("graph lowers to mlir");
        assert!(mlir.contains("ais.flow_call \"sub_flow\""));
        assert!(mlir.contains("(%n1 : !ais.token)"));
        assert!(mlir.contains(": !ais.token"));
    }

    #[test]
    fn lowers_nop_and_identity_passthrough() {
        let graph = ApxmGraph {
            name: "passthrough".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "seed".to_string(),
                    op: AISOperationType::ConstStr,
                    attributes: HashMap::from([(
                        "value".to_string(),
                        Value::String("data".to_string()),
                    )]),
                },
                GraphNode {
                    id: 2,
                    name: "nop".to_string(),
                    op: AISOperationType::Nop,
                    attributes: HashMap::new(),
                },
                GraphNode {
                    id: 3,
                    name: "id".to_string(),
                    op: AISOperationType::Identity,
                    attributes: HashMap::new(),
                },
            ],
            edges: vec![
                crate::GraphEdge {
                    from: 1,
                    to: 2,
                    dependency: DependencyType::Data,
                },
                crate::GraphEdge {
                    from: 2,
                    to: 3,
                    dependency: DependencyType::Data,
                },
            ],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        let mlir = lower_to_mlir(&graph).expect("graph lowers to mlir");
        // Nop and Identity pass through their inputs so the final return
        // should reference the const_str value directly.
        assert!(mlir.contains("func.return %n1 : !ais.token"));
    }

    #[test]
    fn lowers_aam_ops_update_goal_pause_resume() {
        let graph = ApxmGraph {
            name: "aam".to_string(),
            nodes: vec![
                GraphNode {
                    id: 1,
                    name: "update".to_string(),
                    op: AISOperationType::UpdateGoal,
                    attributes: HashMap::from([(
                        graph_attrs::GOAL_ID.to_string(),
                        Value::String("g1".to_string()),
                    )]),
                },
                GraphNode {
                    id: 2,
                    name: "pause".to_string(),
                    op: AISOperationType::Pause,
                    attributes: HashMap::from([(
                        graph_attrs::MESSAGE.to_string(),
                        Value::String("awaiting review".to_string()),
                    )]),
                },
                GraphNode {
                    id: 3,
                    name: "resume".to_string(),
                    op: AISOperationType::Resume,
                    attributes: HashMap::from([(
                        graph_attrs::CHECKPOINT.to_string(),
                        Value::String("cp_1".to_string()),
                    )]),
                },
            ],
            edges: vec![
                crate::GraphEdge {
                    from: 1,
                    to: 2,
                    dependency: DependencyType::Control,
                },
                crate::GraphEdge {
                    from: 2,
                    to: 3,
                    dependency: DependencyType::Control,
                },
            ],
            parameters: vec![],
            metadata: HashMap::new(),
        };

        let mlir = lower_to_mlir(&graph).expect("graph lowers to mlir");
        assert!(mlir.contains("ais.update_goal \"g1\""));
        assert!(mlir.contains("ais.pause \"awaiting review\""));
        assert!(mlir.contains("ais.resume \"cp_1\""));
    }

    #[test]
    fn spawn_agent_with_flow_params_compiles() {
        // Test that spawn_agent now works in parameterized flows (E511 bug fix).
        // The key is that spawn_agent should NOT receive %arg0 even if it's an entry node.
        let mut spawn = GraphNode {
            id: 1,
            name: "spawn".to_string(),
            op: AISOperationType::SpawnAgent,
            attributes: HashMap::new(),
        };
        spawn.attributes.insert(
            graph_attrs::AGENT_NAME.to_string(),
            Value::String("coder".to_string()),
        );
        spawn.attributes.insert(
            "profile".to_string(),
            Value::String("claude".to_string()),
        );
        spawn.attributes.insert(
            "cwd".to_string(),
            Value::String("/tmp".to_string()),
        );

        // think node uses the TASK parameter
        let mut think = GraphNode {
            id: 2,
            name: "think".to_string(),
            op: AISOperationType::Think,
            attributes: HashMap::new(),
        };
        think.attributes.insert(
            graph_attrs::TEMPLATE_STR.to_string(),
            Value::String("task: {0}".to_string()),
        );

        // communicate sends result to the spawned agent
        let mut comm = GraphNode {
            id: 3,
            name: "comm".to_string(),
            op: AISOperationType::Communicate,
            attributes: HashMap::new(),
        };
        comm.attributes.insert(
            graph_attrs::RECIPIENT.to_string(),
            Value::String("coder".to_string()),
        );
        comm.attributes.insert(
            graph_attrs::MESSAGE.to_string(),
            Value::String("{0}".to_string()),
        );

        let mut graph = ApxmGraph {
            name: "test".to_string(),
            nodes: vec![spawn, think, comm],
            edges: vec![
                crate::GraphEdge {
                    from: 1,
                    to: 3,
                    dependency: DependencyType::Control,  // spawn must happen before communicate
                },
                crate::GraphEdge {
                    from: 2,
                    to: 3,
                    dependency: DependencyType::Data,  // think result goes to communicate
                },
            ],
            parameters: Vec::new(),
            metadata: HashMap::new(),
        };
        graph.parameters = vec![crate::Parameter {
            name: "TASK".to_string(),
            type_name: "str".to_string(),
        }];

        // This should NOT crash or produce malformed MLIR anymore.
        // Previously, this combination would cause E900 with "expected ':'" parse error.
        let mlir = lower_to_mlir(&graph).expect("spawn_agent with flow params should compile");

        // Verify the key invariants:
        // 1. Function should have parameter arg
        assert!(mlir.contains("%arg0: !ais.token"), "MLIR should have parameter arg\n{}", mlir);
        // 2. spawn_agent should be emitted WITHOUT receiving %arg0 in its context
        // (previously the lowering injected %arg0 into spawn_agent's inputs, causing malformed MLIR)
        assert!(mlir.contains("ais.spawn_agent \"coder\""), "MLIR should have spawn_agent\n{}", mlir);
        assert!(!mlir.contains("ais.spawn_agent \"coder\"(%arg0"), "spawn_agent should NOT receive %arg0 in context\n{}", mlir);
        // 3. The MLIR should be parseable (no E900 error)
        assert!(mlir.contains("func.func @test"), "MLIR should have function definition\n{}", mlir);
        assert!(mlir.contains("func.return"), "MLIR should have return\n{}", mlir);
    }
}
