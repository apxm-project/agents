//! Unit tests for the core event envelope.

use super::event::{ApxmEvent, EventSource};
use super::payload::GraphEdgePayload;

/// A valid `EventMeta` JSON object, used to build well-formed envelope
/// fixtures for the negative-path test below without depending on
/// `ApxmEvent::root`'s serialization.
fn sample_meta_json() -> serde_json::Value {
    serde_json::json!({
        "seq": 0,
        "timestamp": "2026-07-09T00:00:00Z",
        "trace_id": "trace-1",
        "source": "runtime",
        "span_id": "span-1",
        "parent_span_id": null,
    })
}

/// Positive: for each real edge kind, a `GraphEdgePayload` round-trips its
/// `edge_kind` field through serialize -> JSON -> deserialize without the
/// envelope's own `"kind": "graph_edge"` discriminator clobbering it.
#[test]
fn graph_edge_round_trips_edge_kind_through_serialize_deserialize() {
    for edge_kind in ["dispatch", "tool_invocation", "synthesis_feed"] {
        let event = ApxmEvent::root(
            GraphEdgePayload {
                from_node_id: 1,
                to_node_id: 2,
                edge_kind: edge_kind.to_string(),
            },
            EventSource::Runtime,
            "trace-1",
        );

        let value = serde_json::to_value(&event).expect("serialize ApxmEvent");
        let payload_json = value.get("payload").expect("payload key present");

        // Envelope discriminator is intact...
        assert_eq!(
            payload_json.get("kind").and_then(serde_json::Value::as_str),
            Some("graph_edge"),
            "envelope kind must remain graph_edge for edge_kind={edge_kind}"
        );
        // ...and the payload's own discriminator survived alongside it.
        assert_eq!(
            payload_json
                .get("edge_kind")
                .and_then(serde_json::Value::as_str),
            Some(edge_kind),
            "edge_kind must survive serialization unclobbered"
        );

        let round_tripped: ApxmEvent =
            serde_json::from_value(value).expect("deserialize ApxmEvent");
        let decoded = round_tripped
            .payload
            .downcast_ref::<GraphEdgePayload>()
            .expect("payload decodes back to GraphEdgePayload");
        assert_eq!(decoded.from_node_id, 1);
        assert_eq!(decoded.to_node_id, 2);
        assert_eq!(decoded.edge_kind, edge_kind);
    }
}

/// Negative: a pre-fix corrupted wire shape (no `edge_kind`, only the
/// overwritten envelope `"kind":"graph_edge"`) must fail closed with a
/// typed error naming the missing field — never silently default.
#[test]
fn graph_edge_missing_edge_kind_fails_closed_not_default() {
    let value = serde_json::json!({
        "meta": sample_meta_json(),
        "payload": {
            "kind": "graph_edge",
            "from_node_id": 1,
            "to_node_id": 2,
        },
    });

    let result = serde_json::from_value::<ApxmEvent>(value);
    let err = result.expect_err("missing edge_kind must fail closed, not default");
    let message = err.to_string();
    assert!(
        message.contains("edge_kind"),
        "error must name the missing edge_kind field, got: {message}"
    );
}

/// Regression pin: no `GraphEdgePayload` value, however constructed, can
/// produce a serialized envelope whose `"kind"` differs from the fixed
/// literal `"graph_edge"` — the payload's own discriminator lives in a
/// structurally separate field (`edge_kind`) that can never collide with
/// the envelope's reserved `"kind"` key.
#[test]
fn graph_edge_envelope_kind_is_never_shadowed_by_payload_field() {
    for edge_kind in [
        "dispatch",
        "tool_invocation",
        "synthesis_feed",
        "",
        "graph_edge",
        "kind",
        "some_other_event_kind_name",
    ] {
        let event = ApxmEvent::root(
            GraphEdgePayload {
                from_node_id: 7,
                to_node_id: 9,
                edge_kind: edge_kind.to_string(),
            },
            EventSource::Runtime,
            "trace-2",
        );

        let value = serde_json::to_value(&event).expect("serialize ApxmEvent");
        let envelope_kind = value
            .get("payload")
            .and_then(|payload| payload.get("kind"))
            .and_then(serde_json::Value::as_str);
        assert_eq!(
            envelope_kind,
            Some("graph_edge"),
            "envelope kind must never be shadowed by an arbitrary edge_kind value {edge_kind:?}"
        );
    }
}
