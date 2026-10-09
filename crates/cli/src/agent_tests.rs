use crate::{
    api::{Request, Session},
    protocol,
};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

fn project() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.png");
    let path = dir.path().join("test.vcolor");
    let frame = vibecolor_core::Frame::new(8, 6, vec![[0.2, 0.3, 0.4, 1.0]; 48]).unwrap();
    vibecolor_io::export(&frame, &source, Default::default()).unwrap();
    vibecolor_project::init(&path, &source, None).unwrap();
    (dir, path)
}
fn request(value: Value) -> Request {
    serde_json::from_value(value).unwrap()
}
fn edits() -> Value {
    json!([{"type":"upsert_node","node":{"id":"exposure","op":{"type":"exposure","stops":0.2}}},{"type":"set_output","id":"exposure"}])
}
fn call(session: &mut Session, arguments: Value, lean: bool) -> Value {
    let mut arguments = arguments;
    let command = arguments
        .as_object_mut()
        .unwrap()
        .remove("command")
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned();
    let command = if command == "job_submit" {
        let mut request = arguments
            .as_object_mut()
            .unwrap()
            .remove("request")
            .unwrap();
        let nested = request
            .as_object_mut()
            .unwrap()
            .remove("command")
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned();
        if let Some(key) = arguments.as_object_mut().unwrap().remove("idempotency_key") {
            request["idempotency_key"] = key;
        }
        arguments = request;
        format!("submit_{nested}")
    } else if lean && command == "show" {
        "project_info".into()
    } else {
        command
    };
    if !lean {
        arguments["_response"] = json!("full");
        arguments["_inline_image"] = json!(true);
        if command == "preview" {
            arguments["include_analysis"] = json!(true);
        }
    }
    protocol::handle(session,json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":format!("vibecolor_{command}"),"arguments":arguments}}),&mut true).unwrap()["result"].clone()
}
fn await_job(session: &mut Session, id: u64) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let result = session.run(Request::JobStatus { job: id }).unwrap();
        if ["completed", "failed", "cancelled"].contains(&result["status"].as_str().unwrap()) {
            return result;
        }
        assert!(Instant::now() < deadline, "job deadline exceeded");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn discovery_exposes_static_schemas_with_transitive_definitions() {
    let mut session = Session::default();
    let tools = protocol::handle(
        &mut session,
        json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        &mut true,
    )
    .unwrap();
    assert!(tools["result"]["tools"].as_array().unwrap().len() > 40);
    let preview = protocol::target_schema("preview").unwrap();
    assert_eq!(preview["additionalProperties"], false);
    assert!(serde_json::to_vec(&preview).unwrap().len() < 2000);
    let op = protocol::target_schema("op:primary").unwrap();
    assert_eq!(op["properties"]["type"]["const"], "primary");
    let mask = protocol::target_schema("mask:combine").unwrap();
    assert!(mask["$defs"]["Mask"].is_object());
    let edit = protocol::target_schema("edit:upsert_node").unwrap();
    assert!(edit["$defs"]["Node"].is_object());
    assert!(edit["$defs"]["Operation"].is_object());
    assert!(protocol::target_schema("typo").is_err());
    let result = call(
        &mut session,
        json!({"command":"capabilities","typo":1}),
        true,
    );
    assert_eq!(result["isError"], true);
}

#[test]
fn project_summaries_paginate_and_legacy_show_stays_full() {
    let (_dir, path) = project();
    let mut session = Session::default();
    for expected in 0..6 {
        session.run(request(json!({"command":"apply","project":path,"expect_revision":expected,"edits":edits()}))).unwrap();
    }
    let page = session
        .run(request(
            json!({"command":"project_info","project":path,"history_limit":2}),
        ))
        .unwrap();
    assert_eq!(page["history"].as_array().unwrap().len(), 2);
    assert_eq!(page["next_before_revision"], 5);
    assert!(page.get("recipe").is_none());
    assert!(page["history"][0].get("recipe").is_none());
    let older = session.run(request(json!({"command":"project_info","project":path,"history_limit":2,"before_revision":5,"revision":3,"include_recipe":true}))).unwrap();
    assert_eq!(older["history"][0]["revision"], 4);
    assert_eq!(older["revision"], 3);
    assert!(older["recipe"]["nodes"].is_array());
    let legacy = call(
        &mut session,
        json!({"command":"show","project":path}),
        false,
    );
    assert!(legacy["structuredContent"]["history"][0]["recipe"].is_object());
    let lean = call(&mut session, json!({"command":"show","project":path}), true);
    assert!(
        lean["structuredContent"]["history"][0]
            .get("recipe")
            .is_none()
    );
    assert!(lean["content"][0]["text"].as_str().unwrap().len() < 80);
}

#[test]
fn compact_preview_is_lazy_and_does_not_sort_full_image_statistics() {
    let (dir, path) = project();
    let mut session = Session::default();
    let response = call(
        &mut session,
        json!({"command":"preview","project":path,"output":dir.path().join("lean.png")}),
        true,
    );
    assert_eq!(response["isError"], false);
    assert!(response["structuredContent"].get("analysis").is_none());
    assert_eq!(response["content"][1]["type"], "resource_link");
    assert!(session.analyses.is_empty());
    let legacy = call(
        &mut session,
        json!({"command":"preview","project":path,"output":dir.path().join("legacy.png")}),
        false,
    );
    assert_eq!(legacy["content"][1]["type"], "image");
    assert!(legacy["structuredContent"]["analysis"]["histogram"].is_object());
    assert_eq!(session.analyses.len(), 1);
    session
        .run(request(json!({"command":"stats","project":path})))
        .unwrap();
    assert_eq!(session.analyses.len(), 1);
    session.run(Request::ClearCache {}).unwrap();
    assert!(session.analyses.is_empty());
}

#[test]
fn edit_preview_checks_output_before_commit_and_reports_cancel_after_commit() {
    let (dir, path) = project();
    let output = dir.path().join("exists.png");
    std::fs::write(&output, b"keep this").unwrap();
    let mut session = Session::default();
    let body = json!({"command":"edit_preview","project":path,"expect_revision":0,"edits":edits(),"output":output});
    assert!(session.run(request(body)).is_err());
    assert_eq!(vibecolor_project::load(&path).unwrap().revision, 0);
    let cancel = session.cancel.clone();
    session.observer = Some(Arc::new(move |event| {
        if event["committed"] == true {
            cancel.store(true, Ordering::Relaxed);
        }
    }));
    let receipt = session.run(request(json!({"command":"edit_preview","project":path,"expect_revision":0,"edits":edits(),"output":dir.path().join("cancelled.png")}))).unwrap();
    assert_eq!(receipt["committed"], true);
    assert_eq!(receipt["revision"], 1);
    assert!(receipt["preview_error"].is_object());
    assert!(!dir.path().join("cancelled.png").exists());
    assert_eq!(vibecolor_project::load(&path).unwrap().revision, 1);

    let (batch_dir, batch_path) = project();
    let mut batch_session = Session::default();
    let batch_cancel = batch_session.cancel.clone();
    batch_session.observer = Some(Arc::new(move |event| {
        if event["committed"] == true {
            batch_cancel.store(true, Ordering::Relaxed);
        }
    }));
    let batch = batch_session.run(request(json!({"command":"batch","jobs":[{"command":"edit_preview","project":batch_path,"expect_revision":0,"edits":edits(),"output":batch_dir.path().join("cancelled.png")}]}))).unwrap();
    assert_eq!(batch["failures"], 1);
    assert_eq!(batch["results"][0]["data"]["committed"], true);
}

#[test]
fn async_edit_is_idempotent_and_resources_are_readable_from_main_session() {
    let (dir, path) = project();
    let mut session = Session {
        persistent: true,
        ..Default::default()
    };
    let body = json!({"command":"job_submit","idempotency_key":"once","request":{"command":"edit_preview","project":path,"expect_revision":0,"edits":edits(),"output":dir.path().join("job.png")}});
    let first = session.run(request(body.clone())).unwrap();
    let duplicate = session.run(request(body.clone())).unwrap();
    assert_eq!(first["job"], duplicate["job"]);
    assert_eq!(duplicate["reused"], true);
    let complete = await_job(&mut session, first["job"].as_u64().unwrap());
    assert_eq!(complete["status"], "completed", "{complete}");
    assert_eq!(complete["result"]["committed"], true);
    assert_eq!(complete["commit"]["revision"], 1);
    let uri = complete["result"]["preview"]["resource_uri"]
        .as_str()
        .unwrap();
    let read = protocol::handle(
        &mut session,
        json!({"jsonrpc":"2.0","id":2,"method":"resources/read","params":{"uri":uri}}),
        &mut true,
    )
    .unwrap();
    assert!(
        read["result"]["contents"][0]["blob"]
            .as_str()
            .unwrap()
            .starts_with("iVBOR")
    );
    let repeated = session.run(request(body.clone())).unwrap();
    assert_eq!(repeated["job"], first["job"]);
    assert_eq!(vibecolor_project::load(&path).unwrap().revision, 1);
    let mut different = body;
    different["request"]["label"] = json!("different");
    assert!(
        session
            .run(request(different))
            .unwrap_err()
            .to_string()
            .contains("different request")
    );
    assert!(
        session
            .run(request(
                json!({"command":"job_submit","request":{"command":"clear_cache"}})
            ))
            .is_err()
    );
}

#[test]
fn evicted_jobs_keep_mutation_receipts_and_never_reexecute_retries() {
    let (dir, path) = project();
    let mut session = Session {
        persistent: true,
        ..Default::default()
    };
    let body = json!({"command":"job_submit","idempotency_key":"mutation","request":{"command":"edit_preview","project":path,"expect_revision":0,"edits":edits(),"output":dir.path().join("once.png")}});
    let first = session.run(request(body.clone())).unwrap();
    await_job(&mut session, first["job"].as_u64().unwrap());
    for _ in 0..65 {
        let next = session
            .run(request(
                json!({"command":"job_submit","request":{"command":"stats","project":path}}),
            ))
            .unwrap();
        assert_eq!(
            await_job(&mut session, next["job"].as_u64().unwrap())["status"],
            "completed"
        );
    }
    let retry = session.run(request(body)).unwrap();
    assert_eq!(retry["status"], "expired");
    assert_eq!(retry["commit"]["revision"], 1);
    assert_eq!(retry["reused"], true);
    assert_eq!(vibecolor_project::load(&path).unwrap().revision, 1);
}

#[test]
fn agent_background_preview_skips_analysis_and_idle_releases_cache() {
    let (dir, path) = project();
    let mut session = Session {
        persistent: true,
        ..Default::default()
    };
    session
        .run(request(
            json!({"command":"configure","cache_budget_mib":8,"idle_seconds":1}),
        ))
        .unwrap();
    let submitted = call(
        &mut session,
        json!({"command":"job_submit","request":{"command":"preview","project":path,"output":dir.path().join("async.png")}}),
        true,
    );
    let done = await_job(
        &mut session,
        submitted["structuredContent"]["job"].as_u64().unwrap(),
    );
    assert_eq!(done["status"], "completed");
    assert!(done["result"].get("analysis").is_none());
    assert!(
        session.run(Request::CacheInfo {}).unwrap()["job_cached_bytes"]
            .as_u64()
            .unwrap()
            > 0
    );
    let deadline = Instant::now() + Duration::from_secs(4);
    while session.run(Request::CacheInfo {}).unwrap()["job_cached_bytes"] != 0 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn managed_preview_reuses_filename_and_exports_need_explicit_draft_flag() {
    let (dir, path) = project();
    let mut session = Session::default();
    let body = json!({"command":"preview","project":path,"include_analysis":false});
    let first = session.run(request(body.clone())).unwrap();
    let again = session.run(request(body)).unwrap();
    assert_eq!(first["preview"]["path"], again["preview"]["path"]);
    assert_eq!(
        std::fs::read_dir(format!("{}.work", path.display()))
            .unwrap()
            .count(),
        1
    );
    session
        .run(request(
            json!({"command":"render","project":path,"output":dir.path().join("export.png")}),
        ))
        .unwrap();
    session
        .run(request(
            json!({"command":"apply","project":path,"expect_revision":0,"edits":edits()}),
        ))
        .unwrap();
    session.run(request(json!({"command":"render","project":path,"output":dir.path().join("draft.png"),"temporary":true,"revision":0}))).unwrap();
    let plan = session
        .run(request(
            json!({"command":"cleanup_plan","project":path,"revision":1}),
        ))
        .unwrap();
    assert_eq!(plan["files"].as_array().unwrap().len(), 2);
    assert_eq!(plan["retained"].as_array().unwrap().len(), 1);
}

#[test]
fn analysis_cache_distinguishes_source_color_interpretation() {
    let (dir, srgb) = project();
    let linear = dir.path().join("linear.vcolor");
    vibecolor_project::init(
        &linear,
        &dir.path().join("source.png"),
        Some(vibecolor_color::ColorSpace {
            transfer: vibecolor_color::Transfer::Linear,
            ..Default::default()
        }),
    )
    .unwrap();
    let stats = |path: &PathBuf| request(json!({"command":"stats","project":path}));
    let mut shared = Session::default();
    let first = shared.run(stats(&srgb)).unwrap();
    let second = shared.run(stats(&linear)).unwrap();
    let fresh = Session::default().run(stats(&linear)).unwrap();
    assert_ne!(first["analysis"]["rgb_mean"], fresh["analysis"]["rgb_mean"]);
    assert_eq!(second["analysis"], fresh["analysis"]);
    assert_eq!(
        shared.run(stats(&srgb)).unwrap()["analysis"],
        first["analysis"]
    );
}

#[test]
fn full_diagnostics_do_not_implicitly_transfer_pixels() {
    let (_dir, path) = project();
    let mut session = Session::default();
    for (inline, expected) in [
        (None, "resource_link"),
        (Some(false), "resource_link"),
        (Some(true), "image"),
    ] {
        let mut arguments = json!({"project":path,"_response":"full"});
        if let Some(inline) = inline {
            arguments["_inline_image"] = json!(inline);
        }
        let result = protocol::handle(&mut session, json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"vibecolor_preview","arguments":arguments}}), &mut true).unwrap();
        assert_eq!(result["result"]["isError"], false);
        assert_eq!(result["result"]["content"][1]["type"], expected);
        assert!(
            result["result"]["structuredContent"]
                .get("analysis")
                .is_none()
        );
    }
}

#[test]
fn compact_analyze_omits_histograms_in_direct_and_job_results() {
    let (dir, _path) = project();
    let mut session = Session {
        persistent: true,
        ..Default::default()
    };
    let body = json!({"command":"analyze","input":dir.path().join("source.png")});
    let direct = call(&mut session, body.clone(), true);
    assert_eq!(direct["isError"], false);
    assert!(direct["structuredContent"].get("histogram").is_none());
    assert!(call(&mut session, body.clone(), false)["structuredContent"]["histogram"].is_object());
    let submitted = call(
        &mut session,
        json!({"command":"job_submit","request":body}),
        true,
    );
    let id = submitted["structuredContent"]["job"].as_u64().unwrap();
    await_job(&mut session, id);
    let done = call(&mut session, json!({"command":"job_status","job":id}), true);
    assert!(
        done["structuredContent"]["result"]
            .get("histogram")
            .is_none()
    );
    assert!(done["structuredContent"]["result"]["rgb_mean"].is_array());
}

#[test]
fn preview_failure_keeps_commit_and_signals_failure_in_mcp_and_jobs() {
    let (dir, path) = project();
    // A directory cannot be replaced with PNG bytes, even with overwrite=true.
    let output = dir.path().join("blocked.png");
    std::fs::create_dir(&output).unwrap();
    let mut session = Session {
        persistent: true,
        ..Default::default()
    };
    let body = json!({"command":"edit_preview","project":path,"expect_revision":0,"edits":edits(),"output":output,"overwrite":true});
    let result = call(&mut session, body, true);
    assert_eq!(result["isError"], true);
    assert!(
        result["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("failed")
    );
    assert_eq!(result["structuredContent"]["committed"], true);
    assert_eq!(result["structuredContent"]["revision"], 1);
    assert!(result["structuredContent"]["preview_error"].is_object());
    assert!(output.is_dir());
    let submitted = call(
        &mut session,
        json!({"command":"job_submit","idempotency_key":"partial","request":{"command":"edit_preview","project":path,"expect_revision":1,"edits":edits(),"output":output,"overwrite":true}}),
        true,
    );
    let done = await_job(
        &mut session,
        submitted["structuredContent"]["job"].as_u64().unwrap(),
    );
    assert_eq!(done["status"], "failed");
    assert_eq!(done["commit"]["revision"], 2);
    assert_eq!(done["result"]["committed"], true);
    assert!(dir.path().join("source.png").exists());
}

#[test]
fn synchronous_preview_resource_index_is_bounded_and_latest_is_readable() {
    let (_dir, path) = project();
    let mut session = Session::default();
    for revision in 0..66 {
        session.run(request(json!({"command":"apply","project":path,"expect_revision":revision,"edits":[{"type":"upsert_node","node":{"id":"light","op":{"type":"exposure","stops":revision as f64 / 50.0}}},{"type":"set_output","id":"light"}]}))).unwrap();
        let result = session
            .run(request(
                json!({"command":"preview","project":path,"include_analysis":false}),
            ))
            .unwrap();
        let uri = result["resource_uri"].as_str().unwrap();
        assert!(session.previews.len() <= 64);
        let read = protocol::handle(
            &mut session,
            json!({"jsonrpc":"2.0","id":1,"method":"resources/read","params":{"uri":uri}}),
            &mut true,
        )
        .unwrap();
        assert!(read["result"]["contents"][0]["blob"].is_string());
    }
    assert_eq!(session.previews.len(), 64);
}
