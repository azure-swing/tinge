//! A loopback-only, embedded viewer. This is deliberately not a general filesystem/RPC server.
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
use tinge_engine::Engine;
use tinge_project::{self as project, Project};

const MAX_BODY: usize = 1024 * 1024;
const IMAGE_BUDGET: usize = 128 * 1024 * 1024;
type Images = BTreeMap<String, Arc<Vec<u8>>>;
pub fn random_token() -> Result<String> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("OS random source: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
#[derive(Default)]
struct Job {
    state: Value,
    images: Images,
    preview: Option<Preview>,
    cancel: Arc<AtomicBool>,
    last_used: u64,
}
#[derive(Default)]
struct Jobs {
    next: u64,
    entries: BTreeMap<u64, Job>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Preview {
    revision: u64,
    #[serde(default)]
    reference: Option<u64>,
    max_edge: u32,
    #[serde(default)]
    client_id: String,
}
impl Preview {
    fn same_image(&self, other: &Self) -> bool {
        self.revision == other.revision
            && self.reference == other.reference
            && self.max_edge == other.max_edge
    }
}
struct Work {
    id: u64,
    preview: Preview,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
struct WorkQueue {
    pending: Mutex<VecDeque<Work>>,
    ready: Condvar,
}
impl WorkQueue {
    fn push(&self, work: Work) -> Result<()> {
        let mut pending = self.pending.lock().unwrap();
        pending.retain(|w| !w.cancel.load(Ordering::Relaxed));
        ensure!(pending.len() < 8, "预览服务繁忙，请稍后重试");
        pending.push_back(work);
        self.ready.notify_one();
        Ok(())
    }
    fn pop(&self) -> Option<Work> {
        let mut pending = self.pending.lock().unwrap();
        loop {
            if let Some(work) = pending.pop_front() {
                return Some(work);
            }
            let (next, timeout) = self
                .ready
                .wait_timeout(pending, Duration::from_secs(60))
                .unwrap();
            pending = next;
            if timeout.timed_out() {
                return None;
            }
        }
    }
}
struct Server {
    project: PathBuf,
    token: String,
    authority: String,
    jobs: Arc<Mutex<Jobs>>,
    work: Arc<WorkQueue>,
}
fn json_response(value: Value) -> Response {
    Response {
        status: 200,
        mime: "application/json; charset=utf-8",
        bytes: Arc::new(serde_json::to_vec(&value).unwrap()),
    }
}
struct Response {
    status: u16,
    mime: &'static str,
    bytes: Arc<Vec<u8>>,
}
struct HttpRequest {
    method: String,
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}
impl HttpRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}
fn read_request(stream: &mut TcpStream) -> Result<HttpRequest> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut bytes = Vec::new();
    let head_end = loop {
        let mut block = [0u8; 2048];
        let count = stream.read(&mut block)?;
        ensure!(count > 0, "incomplete HTTP request");
        bytes.extend_from_slice(&block[..count]);
        if let Some(pos) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
            ensure!(pos + 4 <= 16 * 1024, "headers exceed 16KiB");
            break pos + 4;
        }
        ensure!(bytes.len() <= 16 * 1024, "headers exceed 16KiB");
    };
    let mut raw_headers = [httparse::EMPTY_HEADER; 64];
    let mut parsed = httparse::Request::new(&mut raw_headers);
    ensure!(
        parsed.parse(&bytes[..head_end])?.is_complete(),
        "incomplete headers"
    );
    ensure!(parsed.version == Some(1), "HTTP/1.1 required");
    let method = parsed.method.context("missing method")?.to_string();
    let path = parsed.path.context("missing path")?.to_string();
    let mut headers = BTreeMap::new();
    for h in parsed.headers.iter() {
        let key = h.name.to_ascii_lowercase();
        let value = std::str::from_utf8(h.value)?.trim().to_string();
        ensure!(
            headers.insert(key, value).is_none(),
            "duplicate HTTP header"
        );
    }
    ensure!(
        !headers.contains_key("transfer-encoding"),
        "chunked requests unsupported"
    );
    let length = headers
        .get("content-length")
        .map(|v| v.parse::<usize>())
        .transpose()?
        .unwrap_or(0);
    ensure!(length <= MAX_BODY, "request exceeds 1MiB");
    ensure!(
        method == "POST" || length == 0,
        "body only allowed for POST"
    );
    ensure!(bytes.len() <= head_end + length, "pipelining unsupported");
    while bytes.len() < head_end + length {
        let mut block = [0u8; 8192];
        let remaining = head_end + length - bytes.len();
        let limit = remaining.min(block.len());
        let count = stream.read(&mut block[..limit])?;
        ensure!(count > 0, "incomplete HTTP body");
        bytes.extend_from_slice(&block[..count]);
    }
    Ok(HttpRequest {
        method,
        path,
        headers,
        body: bytes[head_end..].to_vec(),
    })
}
fn authenticate(request: &HttpRequest, server: &Server) -> Result<()> {
    ensure!(
        request.header("host") == Some(server.authority.as_str()),
        "invalid local Host"
    );
    if let Some(origin) = request.header("origin") {
        ensure!(
            origin == format!("http://{}", server.authority),
            "foreign Origin"
        );
    }
    ensure!(
        request.header("sec-fetch-site") != Some("cross-site"),
        "cross-site request rejected"
    );
    ensure!(
        request.path.starts_with(&format!("/{}/", server.token)),
        "unknown viewer URL"
    );
    ensure!(
        matches!(request.method.as_str(), "GET" | "POST"),
        "unsupported HTTP method"
    );
    if request.method == "POST" {
        ensure!(
            request.header("x-tinge") == Some(server.token.as_str()),
            "missing viewer token"
        );
        ensure!(
            request
                .header("content-type")
                .is_some_and(|v| v.split(';').next() == Some("application/json")),
            "JSON content-type required"
        );
    }
    Ok(())
}
fn summary(p: &Project, path: &Path) -> Result<Value> {
    let doc = project::selections::load(path)?;
    Ok(json!({
        "name":path.file_name().unwrap_or_default().to_string_lossy(),
        "head":p.revision, "branch":p.current_branch, "tags":p.tags,
        "final_revision":crate::workfiles::final_revision_for(path,p),
        "history":p.history.iter().rev().map(|r| json!({"id":r.id,"parent":r.parent,"label":r.label,"timestamp":r.timestamp})).collect::<Vec<_>>(),
        "selection_count":doc.items.len(),
        "selections":doc.items.iter().rev().take(64).collect::<Vec<_>>(),
        "preview_color":"sRGB SDR · ICC tagged · 8-bit",
        "full_resolve_parity":false
    }))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    expect_revision: u64,
    revision: u64,
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Restore {
    expect_revision: u64,
    revision: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Finalize {
    expect_revision: u64,
    revision: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionSave {
    expect_revision: u64,
    job: u64,
    mask: tinge_core::Mask,
    #[serde(default)]
    note: String,
}

fn enqueue_preview(server: &Server, preview: Preview) -> Result<u64> {
    let mut jobs = server.jobs.lock().unwrap();
    jobs.next += 1;
    let access = jobs.next;
    // Each tab has its own latest request; another tab's render is left alone.
    let reusable = jobs.entries.iter().find_map(|(id, job)| {
        let previous = job.preview.as_ref()?;
        if !previous.same_image(&preview) {
            return None;
        }
        let ready = job.state["status"] == "ready";
        let active = matches!(job.state["status"].as_str(), Some("queued" | "rendering"));
        (ready || (active && previous.client_id == preview.client_id)).then_some(*id)
    });
    for (id, job) in &mut jobs.entries {
        if Some(*id) != reusable
            && job
                .preview
                .as_ref()
                .is_some_and(|p| p.client_id == preview.client_id)
            && matches!(job.state["status"].as_str(), Some("queued" | "rendering"))
        {
            job.cancel.store(true, Ordering::Relaxed);
            job.state = json!({"status":"cancelled","id":id});
        }
    }
    if let Some(id) = reusable {
        jobs.entries.get_mut(&id).unwrap().last_used = access;
        return Ok(id);
    }
    while jobs.entries.len() >= 64 {
        let old = jobs
            .entries
            .iter()
            .find(|(_, j)| {
                matches!(
                    j.state["status"].as_str(),
                    Some("ready" | "error" | "cancelled")
                )
            })
            .map(|(id, _)| *id);
        if let Some(id) = old {
            jobs.entries.remove(&id);
        } else {
            break;
        }
    }
    let id = jobs.next;
    let cancel = Arc::new(AtomicBool::new(false));
    jobs.entries.insert(
        id,
        Job {
            state: json!({"status":"queued","id":id}),
            preview: Some(preview.clone()),
            cancel: cancel.clone(),
            last_used: access,
            ..Default::default()
        },
    );
    if let Err(e) = server.work.push(Work {
        id,
        preview,
        cancel,
    }) {
        jobs.entries.remove(&id);
        return Err(e);
    }
    Ok(id)
}
fn route(request: HttpRequest, server: &Server) -> Result<Response> {
    authenticate(&request, server)?;
    let path = request
        .path
        .strip_prefix(&format!("/{}/", server.token))
        .unwrap();
    if request.method == "GET" {
        let asset = match path {
            "" => Some((
                "text/html; charset=utf-8",
                include_bytes!("web/index.html").as_slice(),
            )),
            "style.css" => Some((
                "text/css; charset=utf-8",
                include_bytes!("web/style.css").as_slice(),
            )),
            "app.js" => Some((
                "text/javascript; charset=utf-8",
                include_bytes!("web/app.js").as_slice(),
            )),
            "icon.svg" => Some(("image/svg+xml", include_bytes!("web/icon.svg").as_slice())),
            _ => None,
        };
        if let Some((mime, bytes)) = asset {
            return Ok(Response {
                status: 200,
                mime,
                bytes: Arc::new(bytes.to_vec()),
            });
        }
        if path == "api/state" {
            let p = project::load(&server.project)?;
            return Ok(json_response(summary(&p, &server.project)?));
        }
        if let Some(revision) = path.strip_prefix("api/cleanup-plan/") {
            return Ok(json_response(crate::workfiles::cleanup_plan(
                &server.project,
                revision.parse()?,
            )?));
        }
        if let Some(id) = path.strip_prefix("api/job/") {
            let jobs = server.jobs.lock().unwrap();
            return Ok(json_response(
                jobs.entries
                    .get(&id.parse()?)
                    .context("job expired; request preview again")?
                    .state
                    .clone(),
            ));
        }
        if let Some(name) = path.strip_prefix("image/") {
            let jobs = server.jobs.lock().unwrap();
            let bytes = jobs
                .entries
                .values()
                .find_map(|j| j.images.get(name))
                .context("preview expired; request again")?
                .clone();
            return Ok(Response {
                status: 200,
                mime: "image/png",
                bytes,
            });
        }
    } else {
        match path {
            "api/preview" => {
                let preview: Preview = serde_json::from_slice(&request.body)?;
                ensure!(preview.client_id.len() <= 128, "invalid preview client ID");
                ensure!(
                    (64..=16384).contains(&preview.max_edge),
                    "preview max_edge must be 64..16384"
                );
                let p = project::load(&server.project)?;
                p.get_revision(preview.revision)?;
                if let Some(reference) = preview.reference {
                    p.get_revision(reference)?;
                }
                let id = enqueue_preview(server, preview)?;
                return Ok(json_response(json!({"job":id})));
            }
            "api/save" => {
                let save: Save = serde_json::from_slice(&request.body)?;
                let name = save.name.trim().to_string();
                project::tag_revision(&server.project, save.expect_revision, save.revision, name)?;
                return Ok(json_response(json!({"ok":true})));
            }
            "api/finalize" => {
                let selected: Finalize = serde_json::from_slice(&request.body)?;
                return Ok(json_response(crate::workfiles::finalize(
                    &server.project,
                    selected.expect_revision,
                    selected.revision,
                )?));
            }
            "api/restore" => {
                let restore: Restore = serde_json::from_slice(&request.body)?;
                project::restore(
                    &server.project,
                    restore.expect_revision,
                    restore.revision,
                    format!("restore revision {} from viewer", restore.revision),
                )?;
                return Ok(json_response(json!({"ok":true})));
            }
            "api/selection" => {
                let save: SelectionSave = serde_json::from_slice(&request.body)?;
                let basis = {
                    let jobs = server.jobs.lock().unwrap();
                    jobs.entries
                        .get(&save.job)
                        .context("preview expired")?
                        .state
                        .get("result")
                        .context("preview not ready")?
                        .clone()
                };
                let p = project::load(&server.project)?;
                let revision = basis["revision"]
                    .as_u64()
                    .context("invalid preview basis")?;
                let rev = p.get_revision(revision)?;
                ensure!(
                    basis["recipe_hash"] == rev.recipe_hash
                        && basis["source_hash"] == p.source.hash,
                    "preview basis mismatch"
                );
                let item = project::selections::Selection {
                    id: random_token()?,
                    revision,
                    source_hash: p.source.hash.clone(),
                    recipe_hash: rev.recipe_hash.clone(),
                    output_node: rev.recipe.output.clone(),
                    width: basis["width"].as_u64().context("missing width")? as u32,
                    height: basis["height"].as_u64().context("missing height")? as u32,
                    timestamp: 0,
                    mask: save.mask,
                    note: save.note,
                };
                let item = project::selections::save(&server.project, save.expect_revision, item)?;
                return Ok(json_response(json!({"ok":true,"selection":item})));
            }
            _ => {}
        }
    }
    bail!("unknown viewer route")
}
fn handle(mut stream: TcpStream, server: &Server) {
    let result = read_request(&mut stream).and_then(|request| route(request, server));
    let response = match result {
        Ok(response) => response,
        Err(e) => {
            let conflict = e.downcast_ref::<project::RevisionConflict>().is_some();
            let mut response =
                json_response(json!({"ok":false,"error":crate::api::error_json(&e)}));
            response.status = if conflict { 409 } else { 400 };
            response
        }
    };
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' blob:; connect-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'\r\n\r\n",
        response.status,
        if response.status == 200 {
            "OK"
        } else {
            "Error"
        },
        response.mime,
        response.bytes.len()
    );
    let _ = stream
        .write_all(headers.as_bytes())
        .and_then(|_| stream.write_all(&response.bytes));
}
fn update(jobs: &Mutex<Jobs>, id: u64, state: Value) {
    if let Some(job) = jobs.lock().unwrap().entries.get_mut(&id)
        && !job.cancel.load(Ordering::Relaxed)
    {
        job.state = state;
    }
}
fn render(
    work: &Work,
    path: &Path,
    engine: &mut Engine,
    jobs: &Mutex<Jobs>,
) -> Result<(Value, Images)> {
    let p = project::load(path)?;
    let rev = p.get_revision(work.preview.revision)?;
    let cancel = work.cancel.as_ref();
    ensure!(!cancel.load(Ordering::Relaxed), "render cancelled");
    update(
        jobs,
        work.id,
        json!({"status":"rendering","id":work.id,"phase":"解码与调色"}),
    );
    let frame = engine.project_render(path, &p, Some(rev.id), cancel, &mut |progress| {
        update(
            jobs,
            work.id,
            json!({"status":"rendering","id":work.id,"phase":"调色","progress":progress}),
        );
    })?;
    let pipeline = rev
        .color_pipeline
        .as_ref()
        .map(|p| p.resolved_at(project::base(path)));
    let reference = if let Some(reference) = work.preview.reference {
        engine.project_render(path, &p, Some(reference), cancel, &mut |progress| {
            update(
                jobs,
                work.id,
                json!({"status":"rendering","id":work.id,"phase":"对比版本","progress":progress}),
            );
        })?
    } else {
        ensure!(!cancel.load(Ordering::Relaxed), "render cancelled");
        update(
            jobs,
            work.id,
            json!({"status":"rendering","id":work.id,"phase":"原图"}),
        );
        engine.project_source(path, &p, Some(rev.id), cancel)?
    };
    let reference_rev = work
        .preview
        .reference
        .map(|id| p.get_revision(id))
        .transpose()?;
    let reference_pipeline = reference_rev
        .map(|r| {
            r.color_pipeline
                .as_ref()
                .map(|p| p.resolved_at(project::base(path)))
        })
        .unwrap_or_else(|| pipeline.clone());
    let mut images = BTreeMap::new();
    let mut reports = Vec::new();
    for (label, full, pipeline) in [
        ("graded", frame.as_ref(), pipeline.as_ref()),
        ("reference", reference.as_ref(), reference_pipeline.as_ref()),
    ] {
        ensure!(!cancel.load(Ordering::Relaxed), "render cancelled");
        update(
            jobs,
            work.id,
            json!({"status":"rendering","id":work.id,"phase":"生成 sRGB 预览"}),
        );
        let preview = full.resized(work.preview.max_edge)?;
        let pipeline = pipeline.map(|p| p.for_preview()).transpose()?;
        let name = format!("{}-{label}.png", work.id);
        let (bytes, report, transform) =
            tinge_engine::preview_png_bytes(&preview, pipeline.as_ref())?;
        ensure!(
            bytes.len() <= IMAGE_BUDGET / 2,
            "preview exceeds 64MiB; use a smaller max_edge"
        );
        reports.push(json!({"width":preview.width,"height":preview.height,"export":report,"transform":transform}));
        images.insert(name, Arc::new(bytes));
    }
    let result = json!({"revision":rev.id,"reference_revision":work.preview.reference,"source_hash":p.source.hash,"recipe_hash":rev.recipe_hash,"output_node":rev.recipe.output,
        "width":frame.width,"height":frame.height,"reference_width":reference.width,"reference_height":reference.height,
        "image":format!("image/{}-graded.png",work.id),"reference":format!("image/{}-reference.png",work.id),
        "graded":reports[0],"reference_report":reports[1],"preview_color":"sRGB SDR · ICC tagged · 8-bit"});
    Ok((result, images))
}
fn worker(path: PathBuf, jobs: Arc<Mutex<Jobs>>, queue: Arc<WorkQueue>) {
    let mut engine = Engine::new();
    loop {
        let Some(work) = queue.pop() else {
            engine.clear_cache();
            continue;
        };
        if work.cancel.load(Ordering::Relaxed) {
            continue;
        }
        let result = render(&work, &path, &mut engine, &jobs);
        let mut jobs = jobs.lock().unwrap();
        if let Some(job) = jobs.entries.get_mut(&work.id) {
            match result {
                _ if work.cancel.load(Ordering::Relaxed) => {
                    job.state = json!({"status":"cancelled","id":work.id});
                }
                Ok((result, images)) => {
                    job.state = json!({"status":"ready","id":work.id,"result":result});
                    job.images = images;
                }
                Err(e) => {
                    job.state =
                        json!({"status":"error","id":work.id,"error":crate::api::error_json(&e)})
                }
            }
        }
        // Remove cancelled/error bookkeeping before evicting expensive ready images.
        loop {
            let size: usize = jobs
                .entries
                .values()
                .flat_map(|j| j.images.values())
                .map(|b| b.len())
                .sum();
            if jobs.entries.len() <= 16 && size <= IMAGE_BUDGET {
                break;
            }
            let old = jobs
                .entries
                .iter()
                .filter(|(id, j)| {
                    **id != work.id
                        && matches!(
                            j.state["status"].as_str(),
                            Some("ready" | "error" | "cancelled")
                        )
                })
                .min_by_key(|(_, j)| (j.state["status"] == "ready", j.last_used))
                .map(|(id, _)| *id);
            if let Some(old) = old {
                jobs.entries.remove(&old);
            } else {
                break;
            }
        }
    }
}
fn open_browser(url: &str) -> Result<()> {
    #[cfg(target_os = "windows")]
    let mut command = std::process::Command::new("rundll32.exe");
    #[cfg(target_os = "windows")]
    command.arg("url.dll,FileProtocolHandler");
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut command = std::process::Command::new("xdg-open");
    command.arg(url).spawn().context("open default browser")?;
    Ok(())
}
pub fn serve(path: PathBuf, port: u16, open: bool) -> Result<()> {
    let path = std::fs::canonicalize(path)?;
    project::load(&path)?;
    let listener = TcpListener::bind(("127.0.0.1", port)).context("bind local viewer")?;
    let authority = listener.local_addr()?.to_string();
    let token = random_token()?;
    let url = format!("http://{authority}/{token}/");
    let jobs = Arc::new(Mutex::new(Jobs::default()));
    let queue = Arc::new(WorkQueue::default());
    let worker_queue = queue.clone();
    let worker_path = path.clone();
    let worker_jobs = jobs.clone();
    std::thread::Builder::new()
        .name("tinge-preview".into())
        .spawn(move || worker(worker_path, worker_jobs, worker_queue))?;
    let server = Arc::new(Server {
        project: path.clone(),
        token,
        authority,
        jobs,
        work: queue,
    });
    let (connections, incoming) = mpsc::sync_channel::<TcpStream>(32);
    let incoming = Arc::new(Mutex::new(incoming));
    for n in 0..8 {
        let server = server.clone();
        let incoming = incoming.clone();
        std::thread::Builder::new()
            .name(format!("tinge-http-{n}"))
            .spawn(move || {
                loop {
                    let stream = incoming.lock().unwrap().recv();
                    match stream {
                        Ok(stream) => handle(stream, &server),
                        Err(_) => break,
                    }
                }
            })?;
    }
    println!(
        "{}",
        json!({"ok":true,"event":"viewer_ready","data":{"url":url,"project":path,"preview_color":"sRGB SDR","full_resolve_parity":false}})
    );
    std::io::stdout().flush()?;
    if open && let Err(e) = open_browser(&url) {
        eprintln!(
            "{}",
            json!({"event":"browser_open_error","message":e.to_string(),"url":url})
        );
    }
    for stream in listener.incoming() {
        let stream = stream?;
        // A bounded pool prevents idle/malformed clients from creating unlimited threads.
        let _ = connections.try_send(stream);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn preview(client: &str, revision: u64) -> Preview {
        Preview {
            revision,
            reference: None,
            max_edge: 1600,
            client_id: client.into(),
        }
    }
    fn test_server() -> Server {
        Server {
            project: PathBuf::new(),
            token: "secret".into(),
            authority: "127.0.0.1:1234".into(),
            jobs: Default::default(),
            work: Default::default(),
        }
    }
    #[test]
    fn rapid_switches_keep_only_latest_request_per_tab() {
        let server = test_server();
        let other = enqueue_preview(&server, preview("other-tab", 0)).unwrap();
        let first = enqueue_preview(&server, preview("tab", 1)).unwrap();
        assert_eq!(first, enqueue_preview(&server, preview("tab", 1)).unwrap());
        for revision in 2..25 {
            enqueue_preview(&server, preview("tab", revision)).unwrap();
        }
        let jobs = server.jobs.lock().unwrap();
        assert_eq!(jobs.entries[&first].state["status"], "cancelled");
        assert!(!jobs.entries[&other].cancel.load(Ordering::Relaxed));
        assert_eq!(server.work.pending.lock().unwrap().len(), 2);
    }
    #[test]
    fn finished_previews_are_reused_and_cancel_obsolete_work() {
        let server = test_server();
        let cached = enqueue_preview(&server, preview("tab", 1)).unwrap();
        server
            .jobs
            .lock()
            .unwrap()
            .entries
            .get_mut(&cached)
            .unwrap()
            .state = json!({"status":"ready"});
        let obsolete = enqueue_preview(&server, preview("tab", 2)).unwrap();
        assert_eq!(cached, enqueue_preview(&server, preview("tab", 1)).unwrap());
        assert!(
            server.jobs.lock().unwrap().entries[&obsolete]
                .cancel
                .load(Ordering::Relaxed)
        );
        assert_eq!(
            cached,
            enqueue_preview(&server, preview("other", 1)).unwrap()
        );
        let mut different = preview("other", 1);
        different.reference = Some(0);
        assert_ne!(cached, enqueue_preview(&server, different).unwrap());
    }
    #[test]
    fn preview_queue_remains_bounded_for_independent_tabs() {
        let server = test_server();
        for n in 0..8 {
            enqueue_preview(&server, preview(&format!("tab-{n}"), n)).unwrap();
        }
        assert!(enqueue_preview(&server, preview("overflow", 9)).is_err());
        assert_eq!(server.work.pending.lock().unwrap().len(), 8);
        assert_eq!(server.jobs.lock().unwrap().entries.len(), 8);
    }
    #[test]
    fn viewer_requires_local_host_url_token_and_same_origin_mutations() {
        let work = Arc::new(WorkQueue::default());
        let server = Server {
            project: PathBuf::new(),
            token: "secret".into(),
            authority: "127.0.0.1:1234".into(),
            jobs: Default::default(),
            work,
        };
        let mut request = HttpRequest {
            method: "POST".into(),
            path: "/secret/api/save".into(),
            headers: BTreeMap::from([
                ("host".into(), "127.0.0.1:1234".into()),
                ("content-type".into(), "application/json".into()),
                ("x-tinge".into(), "secret".into()),
            ]),
            body: vec![],
        };
        assert!(authenticate(&request, &server).is_ok());
        request
            .headers
            .insert("origin".into(), "https://foreign.test".into());
        assert!(authenticate(&request, &server).is_err());
        request.headers.remove("origin");
        request.headers.remove("x-tinge");
        assert!(authenticate(&request, &server).is_err());
        request.method = "GET".into();
        request
            .headers
            .insert("host".into(), "attacker.test:1234".into());
        assert!(authenticate(&request, &server).is_err());
    }
}
