//! Shared partial-failure semantics for CLI, JSONL, MCP, batches and jobs.
use serde_json::Value;

/// A successful dispatch can still contain an irreversible commit/export and
/// a failed follow-up. Preserve that receipt while signalling the failure.
pub fn failed(data: &Value) -> bool {
    data.get("failures")
        .and_then(Value::as_u64)
        .is_some_and(|n| n > 0)
        || data.get("preview_error").is_some_and(|v| !v.is_null())
        || data.get("status").is_some_and(|v| v == "failed")
        || data
            .get("failed")
            .and_then(Value::as_array)
            .is_some_and(|files| !files.is_empty())
        || data.get("registry_error").is_some_and(|v| !v.is_null())
}
