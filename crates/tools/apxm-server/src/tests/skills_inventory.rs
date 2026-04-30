use super::*;

// ── /v1/skills (server-owned skill library) ──────────────────────────────

#[tokio::test]
async fn skills_list_returns_installed_manifests() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_valid_skill(temp.path(), FIXTURE_PACKAGE_DIR);
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (status, body) = get_json(app, routes::SKILLS).await;

    assert_eq!(status, StatusCode::OK, "list skills failed: {body}");
    assert_eq!(body["object"], "list");
    assert_eq!(body["data"][0]["skill_id"], FIXTURE_SKILL_ID);
    assert_eq!(body["data"][0]["version"], FIXTURE_SKILL_VERSION);
    assert_eq!(
        body["data"][0]["manifest"]["entry_flow"],
        FIXTURE_ENTRY_FLOW
    );
    assert_eq!(body["data"][0]["compile_status"], "not_compiled");
    assert_eq!(body["data"][0]["validation"]["status"], "valid");
}

#[tokio::test]
async fn skill_detail_returns_manifest_and_status() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_valid_skill(temp.path(), FIXTURE_PACKAGE_DIR);
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (status, body) = get_json(app, &skill_detail_route(FIXTURE_SKILL_ID)).await;

    assert_eq!(status, StatusCode::OK, "skill detail failed: {body}");
    assert_eq!(body["skill_id"], FIXTURE_SKILL_ID);
    assert_eq!(body["manifest"]["display_name"], FIXTURE_DISPLAY_NAME);
    assert_eq!(body["validation"]["status"], "valid");
}

#[tokio::test]
async fn skill_validate_reports_hash_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let skill_dir = temp.path().join(FIXTURE_PACKAGE_DIR);
    write_skill_manifest(
        &skill_dir,
        &format!(
            r#"
skill_id = "{FIXTURE_SKILL_ID}"
version = "{FIXTURE_SKILL_VERSION}"
entry_flow = "{FIXTURE_ENTRY_FLOW}"
artifact_hash = "{BAD_ARTIFACT_HASH}"
"#
        ),
    );
    std::fs::write(
        skill_dir.join(FILE_SKILL_ARTIFACT),
        skill_artifact_bytes(AISOperationType::ConstStr),
    )
    .expect(FILE_SKILL_ARTIFACT);
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (status, body) = post_json(
        app,
        &skill_validate_route(FIXTURE_SKILL_ID),
        serde_json::json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "validate failed: {body}");
    assert_eq!(body["validation"]["status"], "invalid");
    assert!(
        body["validation"]["errors"][0]
            .as_str()
            .unwrap_or_default()
            .contains("artifact_hash mismatch"),
        "expected hash mismatch: {body}"
    );
}

#[tokio::test]
async fn skill_detail_unknown_returns_404() {
    let app = build_app(test_state().await);
    let (status, body) = get_json(app, &skill_detail_route(UNKNOWN_SKILL_ID)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "expected 404: {body}");
}

#[tokio::test]
async fn skills_registry_reports_duplicate_id_version_as_ambiguous() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_complete_skill_with_version(temp.path(), FIXTURE_PACKAGE_DIR, FIXTURE_SKILL_VERSION);
    write_complete_skill_with_version(temp.path(), FIXTURE_PACKAGE_ALT_DIR, FIXTURE_SKILL_VERSION);
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (list_status, list_body) = get_json(app.clone(), routes::SKILLS).await;
    assert_eq!(list_status, StatusCode::OK, "list failed: {list_body}");
    let records = list_body["data"].as_array().expect("skill records");
    assert_eq!(records.len(), 2);
    for record in records {
        assert_eq!(record["validation"]["status"], "invalid");
        assert!(
            record["validation"]["errors"]
                .as_array()
                .expect("errors")
                .iter()
                .any(|error| error.as_str().unwrap_or_default().contains("duplicate")),
            "duplicate error missing: {record}"
        );
    }

    let (detail_status, detail_body) = get_json(app, &skill_detail_route(FIXTURE_SKILL_ID)).await;
    assert_eq!(
        detail_status,
        StatusCode::BAD_REQUEST,
        "ambiguous detail should fail: {detail_body}"
    );
}

#[tokio::test]
async fn skills_registry_selects_versioned_skill_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_complete_skill_with_version(temp.path(), FIXTURE_PACKAGE_DIR, FIXTURE_SKILL_VERSION);
    write_complete_skill_with_version(
        temp.path(),
        FIXTURE_PACKAGE_V2_DIR,
        FIXTURE_SKILL_NEXT_VERSION,
    );
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);
    let requested_id = format!("{FIXTURE_SKILL_ID}@{FIXTURE_SKILL_NEXT_VERSION}");

    let (status, body) = get_json(app, &skill_detail_route(&requested_id)).await;

    assert_eq!(status, StatusCode::OK, "versioned lookup failed: {body}");
    assert_eq!(body["skill_id"], FIXTURE_SKILL_ID);
    assert_eq!(body["version"], FIXTURE_SKILL_NEXT_VERSION);
    assert_eq!(body["package"]["name"], FIXTURE_PACKAGE_V2_DIR);
}

#[tokio::test]
async fn skills_registry_reports_missing_required_manifest_fields() {
    let temp = tempfile::tempdir().expect("tempdir");
    let skill_dir = temp.path().join(FIXTURE_PACKAGE_DIR);
    write_skill_manifest(
        &skill_dir,
        &format!(
            r#"
skill_id = "{FIXTURE_SKILL_ID}"
version = "{FIXTURE_SKILL_VERSION}"
"#
        ),
    );
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);

    let (status, body) = get_json(app, routes::SKILLS).await;

    assert_eq!(status, StatusCode::OK, "list failed: {body}");
    assert_eq!(body["data"][0]["validation"]["status"], "invalid");
    assert!(
        body["data"][0]["validation"]["errors"]
            .as_array()
            .expect("errors")
            .iter()
            .any(|error| error.as_str().unwrap_or_default().contains("entry_flow")),
        "missing entry_flow error absent: {body}"
    );
}

#[test]
fn parse_cli_skill_roots_uses_repeated_roots_and_dedupes() {
    let first_root = std::path::PathBuf::from("/tmp/apxm-skill-root-a");
    let second_root = std::path::PathBuf::from("/tmp/apxm-skill-root-b");
    let args = vec![
        CLI_ARG_BIN.to_string(),
        CLI_ARG_SKILL_ROOT.to_string(),
        first_root.to_string_lossy().to_string(),
        CLI_ARG_SKILL_ROOT.to_string(),
        second_root.to_string_lossy().to_string(),
        CLI_ARG_SKILL_ROOT.to_string(),
        first_root.to_string_lossy().to_string(),
    ];

    let roots = crate::skills::parse_cli_skill_roots(&args);

    assert_eq!(roots, vec![first_root, second_root]);
}

#[tokio::test]
async fn skills_registry_rest_and_mcp_share_read_only_validated_inventory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = write_complete_skill(temp.path());
    let app = build_app(test_state_with_skill_roots(vec![temp.path().to_path_buf()]).await);
    let versioned_id = versioned_skill_id();

    // REST list must ignore request-provided roots and use only AppState roots.
    let injected_list_route = format!("{}?{QUERY_ROOT_INJECTION}", routes::SKILLS);
    let (list_status, list_body) = get_json(app.clone(), &injected_list_route).await;
    assert_eq!(list_status, StatusCode::OK, "list failed: {list_body}");
    assert!(
        list_body.get("roots").is_none(),
        "normal API responses must not expose filesystem roots: {list_body}"
    );
    assert_eq!(list_body["data"].as_array().unwrap().len(), 1);
    assert_complete_skill_record(&list_body["data"][0], &fixture);

    let (detail_status, detail_body) =
        get_json(app.clone(), &skill_detail_route(&versioned_id)).await;
    assert_eq!(
        detail_status,
        StatusCode::OK,
        "detail failed: {detail_body}"
    );
    assert_complete_skill_record(&detail_body, &fixture);

    // Validate re-reads the installed package and ignores client body paths.
    let mut validate_request = serde_json::Map::new();
    validate_request.insert(
        REQUEST_ROOT_FIELD.to_string(),
        serde_json::Value::String(REQUEST_ROOT_INJECTION.to_string()),
    );
    let (validate_status, validate_body) = post_json(
        app.clone(),
        &skill_validate_route(&versioned_id),
        serde_json::Value::Object(validate_request),
    )
    .await;
    assert_eq!(
        validate_status,
        StatusCode::OK,
        "validate failed: {validate_body}"
    );
    assert_complete_skill_record(&validate_body, &fixture);

    let (mcp_list_status, mcp_list_body) = post_json(
        app.clone(),
        routes::MCP,
        serde_json::json!({
            "jsonrpc": MCP_JSONRPC_VERSION,
            "id": 1,
            "method": MCP_METHOD_TOOLS_LIST,
            "params": {}
        }),
    )
    .await;
    assert_eq!(
        mcp_list_status,
        StatusCode::OK,
        "MCP tools/list failed: {mcp_list_body}"
    );
    let tool_names: Vec<&str> = mcp_list_body["result"]["tools"]
        .as_array()
        .expect("MCP tools array")
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(tool_names.contains(&MCP_TOOL_APXM_SKILLS_LIST));
    assert!(tool_names.contains(&MCP_TOOL_APXM_SKILL_GET));
    assert!(tool_names.contains(&MCP_TOOL_APXM_SKILL_VALIDATE));
    assert!(tool_names.contains(&MCP_TOOL_APXM_SKILL_CALL));

    let mut mcp_args = serde_json::Map::new();
    mcp_args.insert(
        MCP_ARG_ID.to_string(),
        serde_json::Value::String(versioned_id),
    );
    let (mcp_validate_status, mcp_validate_body) = post_json(
        app,
        routes::MCP,
        mcp_call(
            MCP_TOOL_APXM_SKILL_VALIDATE,
            serde_json::Value::Object(mcp_args),
        ),
    )
    .await;
    assert_eq!(
        mcp_validate_status,
        StatusCode::OK,
        "MCP validate failed: {mcp_validate_body}"
    );
    assert_eq!(mcp_validate_body["result"]["isError"], false);
    let mcp_record: serde_json::Value =
        serde_json::from_str(tool_text(&mcp_validate_body)).expect("MCP skill JSON");
    assert_complete_skill_record(&mcp_record, &fixture);
}
