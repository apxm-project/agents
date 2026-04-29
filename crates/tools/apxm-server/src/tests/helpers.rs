use super::*;

// ── JSON-RPC helper unit tests ──────────────────────────────────────────

#[test]
fn jsonrpc_ok_format() {
    let resp = jsonrpc_ok(
        serde_json::json!(42),
        serde_json::json!({"answer": "hello"}),
    );
    let body = resp.0;
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 42);
    assert_eq!(body["result"]["answer"], "hello");
    assert!(body.get("error").is_none());
}

#[test]
fn jsonrpc_ok_with_null_id() {
    let resp = jsonrpc_ok(serde_json::json!(null), serde_json::json!("ok"));
    let body = resp.0;
    assert_eq!(body["jsonrpc"], "2.0");
    assert!(body["id"].is_null());
    assert_eq!(body["result"], "ok");
}

#[test]
fn jsonrpc_err_format() {
    let resp = jsonrpc_err(serde_json::json!(7), -32601, "Method not found");
    let body = resp.0;
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 7);
    assert!(body.get("result").is_none());
    assert_eq!(body["error"]["code"], -32601);
    assert_eq!(body["error"]["message"], "Method not found");
}

#[test]
fn jsonrpc_err_with_string_id() {
    let resp = jsonrpc_err(serde_json::json!("req-abc"), -32600, "Invalid request");
    let body = resp.0;
    assert_eq!(body["id"], "req-abc");
    assert_eq!(body["error"]["code"], -32600);
}

// ── mcp_tool_result helper tests ────────────────────────────────────────

#[test]
fn mcp_tool_result_success() {
    let resp = mcp_tool_result(serde_json::json!(1), "tool output text".to_string(), false);
    let body = resp.0;
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["id"], 1);

    let content = &body["result"]["content"];
    assert!(content.is_array());
    let items = content.as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["type"], "text");
    assert_eq!(items[0]["text"], "tool output text");
    assert_eq!(body["result"]["isError"], false);
}

#[test]
fn mcp_tool_result_error() {
    let resp = mcp_tool_result(
        serde_json::json!(2),
        "something went wrong".to_string(),
        true,
    );
    let body = resp.0;
    assert_eq!(body["result"]["isError"], true);
    assert_eq!(body["result"]["content"][0]["text"], "something went wrong");
}
