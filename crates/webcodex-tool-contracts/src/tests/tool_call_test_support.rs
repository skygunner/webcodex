pub(super) use crate::test_support::sample_tool_args;
use serde_json::{json, Value};

pub(super) fn sample_tool_args_with_session(name: &str) -> Value {
    // Browser Session association exists only on explicit handoff actions;
    // launch must retain its closed, Session-independent contract.
    let mut args = if name == "control_browser" {
        json!({
            "action": "begin_user_handoff",
            "client_id": "mini",
            "browser_id": "browser_abcdefghijklmnop",
            "message_id": "wc_msg_abcdefghijklmnop",
        })
    } else {
        sample_tool_args(name)
    };
    let obj = args
        .as_object_mut()
        .unwrap_or_else(|| panic!("{name} does not accept object arguments"));
    obj.insert(
        "session_id".to_string(),
        Value::String("wc_sess_accessor".to_string()),
    );
    args
}
