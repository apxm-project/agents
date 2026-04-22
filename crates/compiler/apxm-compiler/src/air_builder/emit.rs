use crate::{AirError, AirModule, AirNode, AirParam};
use apxm_core::constants::graph::{attrs as graph_attrs, metadata as graph_meta};
use apxm_core::constants::mlir::types as mlir_types;
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

struct EmitState {
    lines: Vec<String>,
    next_temp: u64,
}

impl EmitState {
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

fn node_uses_flow_params(node: &AirNode, params: &[AirParam]) -> bool {
    if params.is_empty() {
        return false;
    }

    let template_attrs = [
        graph_attrs::TEMPLATE_STR,
        graph_attrs::PROMPT,
        graph_attrs::TEMPLATE,
    ];

    for attr_name in &template_attrs {
        if let Some(value) = node.attributes.get(*attr_name) {
            if let Some(text) = value.as_str() {
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

pub fn emit_air(module: &AirModule) -> Result<String, AirError> {
    module.validate()?;

    let nodes_by_id = module
        .nodes
        .iter()
        .map(|node| (node.id, node))
        .collect::<HashMap<_, _>>();
    let order = topo_order(module, &nodes_by_id)?;

    let mut incoming_by_target: HashMap<u64, Vec<u64>> = HashMap::new();
    let mut outgoing_counts: HashMap<u64, usize> = HashMap::new();
    for edge in &module.edges {
        incoming_by_target
            .entry(edge.to)
            .or_default()
            .push(edge.from);
        *outgoing_counts.entry(edge.from).or_default() += 1;
    }

    let is_entry = module
        .metadata
        .get(graph_meta::IS_ENTRY)
        .and_then(Value::as_boolean)
        .unwrap_or(true);

    let mut state = EmitState::new();
    let mut produced_values: HashMap<u64, MlirValueRef> = HashMap::new();
    let arg_values: Vec<MlirValueRef> = (0..module.parameters.len())
        .map(|index| MlirValueRef {
            ssa: format!("%arg{index}"),
            ty: MlirValueType::Token,
        })
        .collect();

    for node_id in &order {
        let node = nodes_by_id.get(node_id).ok_or_else(|| {
            AirError::Emission(format!(
                "internal error: node id {} not found in topological order (module: '{}')",
                node_id, module.name
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
                    AirError::Emission(format!(
                        "emission failed for node '{}' (id={}, op={}): \
                         references source '{}' (id={}) with no produced SSA value.",
                        node.name, node.id, node.op, source_name, source_id
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        if inputs.is_empty()
            && !arg_values.is_empty()
            && node_uses_flow_params(node, &module.parameters)
        {
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

    let exit_ids = module
        .nodes
        .iter()
        .filter(|node| outgoing_counts.get(&node.id).copied().unwrap_or(0) == 0)
        .map(|node| node.id)
        .collect::<Vec<_>>();

    let has_return_node = module
        .nodes
        .iter()
        .any(|node| node.op == AISOperationType::Return);

    if !has_return_node {
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
    }

    let function_name = sanitize_symbol_name(&module.name);
    let args = module
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
    module: &AirModule,
    nodes_by_id: &HashMap<u64, &AirNode>,
) -> Result<Vec<u64>, AirError> {
    let mut in_degree = nodes_by_id
        .keys()
        .map(|id| (*id, 0usize))
        .collect::<HashMap<_, _>>();
    let mut outgoing: HashMap<u64, Vec<u64>> = HashMap::new();

    for edge in &module.edges {
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
        return Err(AirError::Emission(format!(
            "topological sort produced {} nodes but module '{}' has {} nodes. \
             {} node(s) unreachable (may contain cycles or disconnected components).",
            order.len(),
            module.name,
            nodes_by_id.len(),
            missing_count,
        )));
    }

    Ok(order)
}

fn emit_node(
    state: &mut EmitState,
    node: &AirNode,
    inputs: Vec<MlirValueRef>,
) -> Result<Option<MlirValueRef>, AirError> {
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
            let op_name = node.op.mlir_mnemonic();
            let template = get_non_empty_template(&node.attributes);
            let result = format!("%n{}", node.id);
            let attrs = extra_attr_dict(
                &node.attributes,
                &[graph_attrs::TEMPLATE_STR, graph_attrs::PROMPT],
            );
            let context = format_context(&inputs, '[', ']');
            state.emit(format!(
                "    {result} = ais.{op_name} {}{}{} : {}",
                quote_string(&template),
                context,
                attrs,
                mlir_types::TOKEN
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
                &get_string_attr(&node.attributes, &[graph_attrs::MEMORY_TIER])
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
                    graph_attrs::MEMORY_TIER,
                    graph_attrs::LIMIT,
                ],
            );
            let limit_str = limit
                .map(|value| format!(" limit {value}"))
                .unwrap_or_default();

            state.emit(format!(
                "    {result} = ais.qmem {} stage {} in {}{}{} : !ais.handle<{}>",
                quote_string(&query),
                quote_string(&sid),
                space,
                limit_str,
                attrs,
                space
            ));
            Ok(Some(MlirValueRef {
                ssa: result,
                ty: MlirValueType::Handle { space },
            }))
        }
        AISOperationType::UMem => {
            let key = get_string_attr(&node.attributes, &[graph_attrs::KEY]);
            let space = normalize_memory_space(
                &get_string_attr(&node.attributes, &[graph_attrs::MEMORY_TIER])
                    .unwrap_or_else(|| "stm".to_string()),
            );
            let attrs = extra_attr_dict(
                &node.attributes,
                &[graph_attrs::KEY, graph_attrs::MEMORY_TIER],
            );

            let source = if let Some(input) = inputs.first() {
                ensure_token(state, input.clone())?
            } else {
                emit_const_token(state, "memory")
            };

            let full_attrs = match (key.as_ref(), attrs.as_str()) {
                (Some(k), "") => format!(" {{key = {}}}", quote_string(k)),
                (Some(k), a) => {
                    let inner = a.trim().trim_start_matches('{').trim_end_matches('}');
                    format!(" {{key = {}, {}}}", quote_string(k), inner)
                }
                (None, a) => a.to_string(),
            };

            state.emit(format!(
                "    ais.umem {} into {}{} : !ais.token",
                source.ssa, space, full_attrs
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
            let op_name = node.op.mlir_mnemonic();
            let tokens = inputs
                .into_iter()
                .map(|value| ensure_token(state, value))
                .collect::<Result<Vec<_>, _>>()?;
            let result = format!("%n{}", node.id);
            let attrs = extra_attr_dict_for_node(node, &[]);

            if tokens.is_empty() {
                state.emit(format!(
                    "    {result} = ais.{op_name}{attrs} -> {}",
                    mlir_types::TOKEN
                ));
            } else {
                let operands = tokens
                    .iter()
                    .map(|token| token.ssa.clone())
                    .collect::<Vec<_>>()
                    .join(", ");
                let types = vec![mlir_types::TOKEN; tokens.len()].join(", ");
                state.emit(format!(
                    "    {result} = ais.{op_name} {operands} : {types}{attrs} -> {}",
                    mlir_types::TOKEN
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
        AISOperationType::Communicate => {
            let message = get_string_attr(
                &node.attributes,
                &[
                    graph_attrs::MESSAGE,
                    graph_attrs::TEMPLATE_STR,
                    graph_attrs::PROMPT,
                ],
            )
            .unwrap_or_else(|| "{input}".to_string());
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
        AISOperationType::Handoff => {
            let from = get_string_attr(
                &node.attributes,
                &[graph_attrs::HANDOFF_FROM],
            )
            .unwrap_or_else(|| "source".to_string());
            let to = get_string_attr(
                &node.attributes,
                &[graph_attrs::HANDOFF_TO],
            )
            .unwrap_or_else(|| "target".to_string());
            let attrs = extra_attr_dict(
                &node.attributes,
                &[
                    graph_attrs::HANDOFF_FROM,
                    graph_attrs::HANDOFF_TO,
                ],
            );
            let result = format!("%n{}", node.id);
            let context = format_context(&inputs, '(', ')');

            state.emit(format!(
                "    {result} = ais.handoff {} to {}{}{} : !ais.token",
                quote_string(&from),
                quote_string(&to),
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
            .unwrap_or_else(|| "{input}".to_string());
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
            &[graph_attrs::PROPOSAL, graph_attrs::TEMPLATE_STR],
            "{input}",
            &[graph_attrs::PROPOSAL, graph_attrs::TEMPLATE_STR],
            Some(('(', ')')),
        ),
        AISOperationType::FlowCall => emit_simple_op(
            state,
            node,
            &inputs,
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
                    "    ais.return {}{} : !ais.token",
                    token.ssa, attrs
                ));
            } else {
                state.emit(format!("    ais.return{}", attrs));
            }
            Ok(None)
        }
        AISOperationType::Exc => emit_simple_op(
            state,
            node,
            &inputs,
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
            .unwrap_or_else(|| "{input}".to_string());
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
        AISOperationType::Agent => emit_simple_op(
            state,
            node,
            &inputs,
            &[graph_attrs::AGENT_NAME],
            "agent",
            &[graph_attrs::AGENT_NAME],
            None,
        ),
        AISOperationType::UpdateGoal => emit_simple_op(
            state,
            node,
            &inputs,
            &[graph_attrs::GOAL_ID, graph_attrs::GOAL],
            "goal",
            &[graph_attrs::GOAL_ID, graph_attrs::GOAL],
            Some(('[', ']')),
        ),
        AISOperationType::Guard => emit_simple_op(
            state,
            node,
            &inputs,
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
            &[graph_attrs::MESSAGE, graph_attrs::CHECKPOINT],
            "paused",
            &[graph_attrs::MESSAGE, graph_attrs::CHECKPOINT],
            Some(('[', ']')),
        ),
        AISOperationType::Resume => emit_simple_op(
            state,
            node,
            &inputs,
            &[graph_attrs::CHECKPOINT, graph_attrs::CHECKPOINT_ID],
            "latest",
            &[graph_attrs::CHECKPOINT, graph_attrs::CHECKPOINT_ID],
            None,
        ),
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
        AISOperationType::SpawnAgent => emit_simple_op(
            state,
            node,
            &[],
            &[graph_attrs::AGENT_NAME],
            "agent",
            &[graph_attrs::AGENT_NAME],
            Some(('(', ')')),
        ),
        AISOperationType::SpawnTeam => emit_simple_op(
            state,
            node,
            &[],
            &[graph_attrs::TEAM_NAME],
            "team",
            &[graph_attrs::TEAM_NAME],
            Some(('(', ')')),
        ),
        AISOperationType::RegisterCapability => emit_simple_op(
            state,
            node,
            &inputs,
            &[graph_attrs::CAPABILITY_NAME, graph_attrs::CAPABILITY],
            "capability",
            &[graph_attrs::CAPABILITY_NAME, graph_attrs::CAPABILITY],
            None,
        ),
        AISOperationType::Autonomous => emit_simple_op(
            state,
            node,
            &inputs,
            &[graph_attrs::STRATEGY, graph_attrs::TEMPLATE_STR],
            "default",
            &[graph_attrs::STRATEGY, graph_attrs::TEMPLATE_STR],
            Some(('(', ')')),
        ),
        AISOperationType::Checkpoint => emit_simple_op(
            state,
            node,
            &inputs,
            &[graph_attrs::CHECKPOINT_ID],
            "checkpoint",
            &[graph_attrs::CHECKPOINT_ID],
            None,
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_simple_op(
    state: &mut EmitState,
    node: &AirNode,
    inputs: &[MlirValueRef],
    primary_keys: &[&str],
    primary_default: &str,
    consumed_keys: &[&str],
    context_delimiters: Option<(char, char)>,
) -> Result<Option<MlirValueRef>, AirError> {
    let op_name = node.op.mlir_mnemonic();
    let primary = get_string_attr(&node.attributes, primary_keys)
        .unwrap_or_else(|| primary_default.to_string());
    let attrs = extra_attr_dict_for_node(node, consumed_keys);
    let result = format!("%n{}", node.id);
    let context = context_delimiters
        .map(|(open, close)| format_context(inputs, open, close))
        .unwrap_or_default();

    state.emit(format!(
        "    {result} = ais.{op_name} {}{}{} : {}",
        quote_string(&primary),
        context,
        attrs,
        mlir_types::TOKEN
    ));
    Ok(Some(MlirValueRef {
        ssa: result,
        ty: MlirValueType::Token,
    }))
}

fn emit_bridge_token(
    state: &mut EmitState,
    node_id: u64,
    inputs: &[MlirValueRef],
) -> Result<MlirValueRef, AirError> {
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

fn ensure_token(state: &mut EmitState, value: MlirValueRef) -> Result<MlirValueRef, AirError> {
    if matches!(value.ty, MlirValueType::Token) {
        return Ok(value);
    }

    let result = state.fresh_value("tok");
    let source_type = format_type(&value.ty);
    state.emit(format!(
        "    {result} = ais.ask \"{{input}}\" [{} : {}] {{{} = [\"input\"]}} : !ais.token",
        value.ssa,
        source_type,
        quote_string(graph_attrs::INPUT_NAMES),
    ));

    Ok(MlirValueRef {
        ssa: result,
        ty: MlirValueType::Token,
    })
}

fn emit_const_token(state: &mut EmitState, value: &str) -> MlirValueRef {
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
    .unwrap_or_else(|| "{input}".to_string())
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

fn extra_attr_dict_for_node(node: &AirNode, consumed: &[&str]) -> String {
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
            if consumed.contains(&key.as_str())
                || graph_attrs::MLIR_DERIVED_BARE_ATTRS.contains(&key.as_str())
                || !is_valid_attr_name(key)
            {
                return None;
            }
            Some(format!(
                "{} = {}",
                quote_string(key),
                value_to_mlir_attr(value)
            ))
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
