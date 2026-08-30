//! Wire-shape guards for the debug protocol this harness speaks.
//!
//! These are the two failures that silently broke every scenario: a struct
//! variant of the adjacently-tagged `DebugCommandKind` serialized without its
//! `params` key, and an error response read from a bare `error` key instead of
//! `{"type": "error", "message": ...}`. Building the frames from the real
//! `rinch_debug::protocol` types is what fixes them; this pins that down.

use rinch_debug::protocol::{DebugCommandKind, DebugResult, Request, Response};

#[test]
fn dom_tree_request_carries_params_and_verbose() {
    let req = Request {
        id: 9,
        command: DebugCommandKind::DomTree {
            max_depth: Some(1000),
            root_id: None,
            verbose: true,
        },
    };
    let wire = serde_json::to_string(&req).unwrap();
    assert!(wire.contains("\"params\""), "params key missing: {wire}");
    assert!(wire.contains("\"verbose\":true"), "verbose missing: {wire}");

    let back: Request = serde_json::from_str(&wire).unwrap();
    assert!(matches!(
        back.command,
        DebugCommandKind::DomTree { verbose: true, .. }
    ));
}

#[test]
fn responses_deserialize_by_their_type_tag() {
    let json: Response = serde_json::from_str(r#"{"id":1,"type":"json","data":{"a":1}}"#).unwrap();
    assert!(matches!(json.result, DebugResult::Json { .. }));

    let bytes: Response = serde_json::from_str(r#"{"id":2,"type":"bytes","data":"QUJD"}"#).unwrap();
    assert!(matches!(bytes.result, DebugResult::Bytes { .. }));

    let err: Response =
        serde_json::from_str(r#"{"id":3,"type":"error","message":"boom"}"#).unwrap();
    match err.result {
        DebugResult::Error { message } => assert_eq!(message, "boom"),
        other => panic!("expected an error response, got {other:?}"),
    }
}
