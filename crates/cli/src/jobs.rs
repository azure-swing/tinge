use crate::api::{Request, Session, error_json};
use anyhow::{Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
    },
    time::Duration,
};

#[derive(Debug)]
pub struct JobError {
    pub code: &'static str,
    pub message: String,
}
impl fmt::Display for JobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}
impl std::error::Error for JobError {}
fn failure(code: &'static str, message: &str) -> anyhow::Error {
    JobError {
        code,
        message: message.into(),
    }
    .into()
}

struct Record {
    state: Value,
    cancel: Arc<AtomicBool>,
    key: Option<String>,
    digest: String,
    resources: BTreeMap<String, std::path::PathBuf>,
}
#[derive(Default)]
struct Registry {
    next: u64,
    records: BTreeMap<u64, Record>,
    receipts: BTreeMap<String, (String, Value)>,
}
struct Work {
    id: u64,
    request: Option<Request>,
    cancel: Arc<AtomicBool>,
}
pub struct Jobs {
    registry: Arc<Mutex<Registry>>,
    sender: SyncSender<Work>,
    budget: Arc<AtomicUsize>,
    idle: Arc<AtomicUsize>,
    cached: Arc<AtomicUsize>,
    clear_requested: Arc<AtomicBool>,
}
fn terminal(state: &Value) -> bool {
    matches!(
        state["status"].as_str(),
        Some("completed" | "failed" | "cancelled")
    )
}

impl Jobs {
    pub fn new(budget: usize, idle_seconds: u64) -> Result<Self> {
        let mut seed = [0; 8];
        getrandom::fill(&mut seed).map_err(|e| anyhow::anyhow!("OS random source: {e}"))?;
        let registry = Arc::new(Mutex::new(Registry {
            next: u64::from_le_bytes(seed) & ((1 << 52) - 1),
            ..Default::default()
        }));
        let (sender, receiver) = mpsc::sync_channel::<Work>(8);
        let budget = Arc::new(AtomicUsize::new(budget));
        let idle = Arc::new(AtomicUsize::new(idle_seconds as usize));
        let cached = Arc::new(AtomicUsize::new(0));
        let clear_requested = Arc::new(AtomicBool::new(false));
        let (worker_cached, worker_clear) = (cached.clone(), clear_requested.clone());
        let (worker_registry, worker_budget, worker_idle) =
            (registry.clone(), budget.clone(), idle.clone());
        std::thread::spawn(move || {
            let mut session = Session::default();
            loop {
                if worker_clear.swap(false, Ordering::Relaxed) {
                    session.clear_cached_data();
                    worker_cached.store(0, Ordering::Relaxed);
                }
                let work = match receiver.recv_timeout(Duration::from_secs(
                    worker_idle.load(Ordering::Relaxed) as u64,
                )) {
                    Ok(work) => work,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        session.clear_cached_data();
                        worker_cached.store(0, Ordering::Relaxed);
                        continue;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                let Some(request) = work.request else {
                    continue;
                };
                {
                    let mut r = worker_registry.lock().unwrap();
                    let Some(record) = r.records.get_mut(&work.id) else {
                        continue;
                    };
                    if work.cancel.load(Ordering::Relaxed) {
                        record.state["status"] = json!("cancelled");
                        continue;
                    }
                    record.state["status"] = json!("running");
                }
                let budget = worker_budget.load(Ordering::Relaxed);
                if session.engine.cache_budget_bytes != budget {
                    session.clear_cached_data();
                }
                session.engine.cache_budget_bytes = budget;
                session.cancel = work.cancel.clone();
                let (events, id) = (worker_registry.clone(), work.id);
                session.observer = Some(Arc::new(move |event| {
                    if let Some(record) = events.lock().unwrap().records.get_mut(&id) {
                        if event.get("committed").is_some() {
                            record.state["commit"] = event;
                        } else {
                            record.state["progress"] = event;
                        }
                    }
                }));
                let result = session.run(request);
                worker_cached.store(session.engine.cache_usage_bytes(), Ordering::Relaxed);
                session.observer = None;
                let mut r = worker_registry.lock().unwrap();
                let record = r.records.get_mut(&work.id).unwrap();
                match result {
                    Ok(data) => {
                        if serde_json::to_vec(&data)
                            .map_or(true, |bytes| bytes.len() > 4 * 1024 * 1024)
                        {
                            record.state["status"] = json!("failed");
                            record.state["error"] = json!({"code":"result_too_large","message":"job result exceeds 4MiB; request smaller diagnostics"});
                            session.previews.clear();
                            continue;
                        }
                        let failed = data.get("preview_error").is_some()
                            || data
                                .get("failed")
                                .and_then(Value::as_array)
                                .is_some_and(|files| !files.is_empty())
                            || data
                                .get("registry_error")
                                .is_some_and(|error| !error.is_null());
                        record.state["status"] = json!(if failed { "failed" } else { "completed" });
                        for uri in resource_uris(&data) {
                            if let Some(path) = session.previews.get(&uri) {
                                record.resources.insert(uri, path.clone());
                            }
                        }
                        record.state["result"] = data;
                    }
                    Err(error) => {
                        record.state["status"] = json!(if work.cancel.load(Ordering::Relaxed) {
                            "cancelled"
                        } else {
                            "failed"
                        });
                        record.state["error"] = error_json(&error);
                    }
                }
                // Result records own the referenced paths. The worker need not
                // retain another unbounded index of every generated preview.
                session.previews.clear();
            }
        });
        Ok(Self {
            registry,
            sender,
            budget,
            idle,
            cached,
            clear_requested,
        })
    }
    pub fn configure(&self, budget: usize, idle_seconds: u64) {
        self.budget.store(budget, Ordering::Relaxed);
        self.idle.store(idle_seconds as usize, Ordering::Relaxed);
        self.clear_cache();
    }
    pub fn cache_usage_bytes(&self) -> usize {
        self.cached.load(Ordering::Relaxed)
    }
    pub fn clear_cache(&self) {
        self.clear_requested.store(true, Ordering::Relaxed);
        let _ = self.sender.try_send(Work {
            id: 0,
            request: None,
            cancel: Arc::new(AtomicBool::new(false)),
        });
    }
    pub fn submit(&self, request: Request, key: Option<String>) -> Result<Value> {
        ensure!(
            matches!(
                request,
                Request::Preview { .. }
                    | Request::Render { .. }
                    | Request::EditPreview { .. }
                    | Request::Stats { .. }
                    | Request::Analyze { .. }
                    | Request::Compare { .. }
                    | Request::Grade { .. }
                    | Request::Finalize { .. }
            ),
            "job_submit accepts preview/render/edit_preview/stats/analyze/compare/grade/finalize; nested jobs and service controls are rejected"
        );
        if let Some(key) = &key {
            ensure!(
                !key.is_empty() && key.len() <= 128,
                "idempotency key must be 1..128 bytes"
            );
        }
        let encoded = serde_json::to_vec(&request)?;
        ensure!(encoded.len() <= 1024 * 1024, "job request exceeds 1MiB");
        let digest = blake3::hash(&encoded).to_hex().to_string();
        let mut registry = self.registry.lock().unwrap();
        if let Some(key) = &key {
            if let Some((previous, receipt)) = registry.receipts.get(key) {
                if previous != &digest {
                    return Err(failure(
                        "idempotency_conflict",
                        "idempotency key already names a different request",
                    ));
                }
                let mut receipt = receipt.clone();
                receipt["reused"] = json!(true);
                return Ok(receipt);
            }
            if let Some((&id, record)) = registry
                .records
                .iter()
                .find(|(_, record)| record.key.as_ref() == Some(key))
            {
                if record.digest != digest {
                    return Err(failure(
                        "idempotency_conflict",
                        "idempotency key already names a different request",
                    ));
                }
                return Ok(json!({"job":id,"status":record.state["status"],"reused":true}));
            }
        }
        // Keep small tombstones when dropping heavy terminal results. Old
        // retries receive an expired receipt, never another mutation.
        if key.is_some()
            && registry.receipts.len()
                + registry
                    .records
                    .values()
                    .filter(|r| r.key.is_some())
                    .count()
                >= 4096
        {
            return Err(failure(
                "job_capacity",
                "4096 idempotency receipts reached in this session",
            ));
        }
        while registry.records.len() >= 64 {
            let evict = registry
                .records
                .iter()
                .find(|(_, record)| terminal(&record.state))
                .map(|(&id, _)| id);
            if let Some(id) = evict {
                let record = registry.records.remove(&id).unwrap();
                if let Some(key) = record.key {
                    registry.receipts.insert(key, (record.digest, json!({"job":id,"status":"expired","original_status":record.state["status"],"commit":record.state.get("commit"),"result_retained":false})));
                }
            } else {
                return Err(failure(
                    "job_capacity",
                    "64 retained jobs reached; reconnect for a new job session after inspecting completed results",
                ));
            }
        }
        registry.next += 1;
        let id = registry.next;
        let cancel = Arc::new(AtomicBool::new(false));
        registry.records.insert(
            id,
            Record {
                state: json!({"job":id,"status":"queued"}),
                cancel: cancel.clone(),
                key,
                digest,
                resources: Default::default(),
            },
        );
        if let Err(error) = self.sender.try_send(Work {
            id,
            request: Some(request),
            cancel,
        }) {
            registry.records.remove(&id);
            return Err(failure(
                "queue_full",
                &format!("background queue unavailable: {error}"),
            ));
        }
        Ok(json!({"job":id,"status":"queued","reused":false}))
    }
    pub fn status(&self, id: u64) -> Result<Value> {
        let registry = self.registry.lock().unwrap();
        registry
            .records
            .get(&id)
            .map(|record| record.state.clone())
            .or_else(|| {
                registry
                    .receipts
                    .values()
                    .find(|(_, receipt)| receipt["job"] == id)
                    .map(|(_, receipt)| receipt.clone())
            })
            .ok_or_else(|| failure("job_not_found", "job not found in this server session"))
    }
    pub fn resources(&self, id: u64) -> BTreeMap<String, std::path::PathBuf> {
        self.registry
            .lock()
            .unwrap()
            .records
            .get(&id)
            .map(|record| record.resources.clone())
            .unwrap_or_default()
    }
    pub fn cancel(&self, id: u64) -> Result<Value> {
        let mut registry = self.registry.lock().unwrap();
        let record = registry
            .records
            .get_mut(&id)
            .ok_or_else(|| failure("job_not_found", "job not found in this server session"))?;
        if !terminal(&record.state) {
            record.cancel.store(true, Ordering::Relaxed);
            record.state["cancel_requested"] = json!(true);
            if record.state["status"] == "queued" {
                record.state["status"] = json!("cancelled");
            }
        }
        Ok(record.state.clone())
    }
}
impl Drop for Jobs {
    fn drop(&mut self) {
        for record in self.registry.lock().unwrap().records.values() {
            record.cancel.store(true, Ordering::Relaxed);
        }
    }
}
pub fn resource_uris(data: &Value) -> Vec<String> {
    let mut found = Vec::new();
    fn walk(data: &Value, out: &mut Vec<String>) {
        match data {
            Value::Object(map) => {
                if let Some(uri) = map.get("resource_uri").and_then(Value::as_str) {
                    out.push(uri.into());
                }
                for value in map.values() {
                    walk(value, out);
                }
            }
            Value::Array(values) => {
                for value in values {
                    walk(value, out);
                }
            }
            _ => {}
        }
    }
    walk(data, &mut found);
    found
}
