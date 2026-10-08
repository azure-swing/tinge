use serde_json::{Value, json};

/// Shared process-test invocation for the public named MCP surface.
pub fn call(id: u64, mut arguments: Value) -> Value {
    let command = arguments
        .as_object_mut()
        .unwrap()
        .remove("command")
        .unwrap();
    arguments["_response"] = json!("full");
    arguments["_inline_image"] = json!(true);
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":format!("vibecolor_{}",command.as_str().unwrap()),"arguments":arguments}})
}
