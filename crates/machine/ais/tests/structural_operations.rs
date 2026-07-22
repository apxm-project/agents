use std::str::FromStr;

use apxm_ais::{SemanticOpKind, StructuralOpKind, generate_structural_tablegen_declarations};

#[test]
fn structural_operation_family_is_closed_and_ais_owned() {
    assert_eq!(
        StructuralOpKind::all(),
        [
            StructuralOpKind::Function,
            StructuralOpKind::Region,
            StructuralOpKind::Block,
            StructuralOpKind::Value,
            StructuralOpKind::Branch,
            StructuralOpKind::Switch,
            StructuralOpKind::Loop,
            StructuralOpKind::ParallelJoin,
            StructuralOpKind::Try,
            StructuralOpKind::Throw,
            StructuralOpKind::Catch,
            StructuralOpKind::Return,
            StructuralOpKind::Yield,
        ],
    );
    assert_eq!(StructuralOpKind::Loop.wire(), "ais.loop");
    assert_eq!(
        serde_json::to_value(StructuralOpKind::Loop).unwrap(),
        "ais.loop"
    );
    assert_eq!(
        StructuralOpKind::from_str("ais.loop").unwrap(),
        StructuralOpKind::Loop
    );
}

#[test]
fn loop_cannot_masquerade_as_a_sixth_semantic_operation() {
    assert_eq!(SemanticOpKind::all().len(), 5);
    assert!(SemanticOpKind::from_str("ais.loop").is_err());
    assert!(SemanticOpKind::from_str("loop").is_err());
    assert!(StructuralOpKind::from_str("loop").is_err());
}

#[test]
fn structural_tablegen_is_generated_from_the_closed_owner_family() {
    let declarations = generate_structural_tablegen_declarations();
    for kind in StructuralOpKind::all() {
        let class = kind.mlir_cpp_class();
        assert!(
            declarations.contains(&format!("def AIS_{class}")),
            "missing TableGen declaration for {kind}"
        );
    }
    assert!(declarations.contains("AIS_LoopOp : AIS_Op<\"loop\""));
}
