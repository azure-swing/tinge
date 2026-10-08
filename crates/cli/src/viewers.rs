use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

struct Viewer {
    child: Child,
    ready: Value,
}
#[derive(Default)]
pub struct Viewers {
    children: BTreeMap<PathBuf, Viewer>,
}
impl Viewers {
    pub fn open(&mut self, project: &Path, port: u16) -> Result<Value> {
        let path = std::fs::canonicalize(project)?;
        crate::api::project_info(&path, None, false, 0, None)?;
        if let Some(viewer) = self.children.get_mut(&path)
            && viewer.child.try_wait()?.is_none()
        {
            let mut ready = viewer.ready.clone();
            ready["reused"] = json!(true);
            return Ok(ready);
        }
        let mut command = Command::new(std::env::current_exe()?);
        command
            .arg("view")
            .arg(&path)
            .args(["--port", &port.to_string(), "--no-open"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = command.spawn().context("start managed viewer")?;
        let stdout = child.stdout.take().unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
            let _ = sender.send(result);
        });
        let ready = (|| -> Result<Value> {
            let line = receiver
                .recv_timeout(Duration::from_secs(15))
                .context("viewer startup timed out")??;
            let response: Value =
                serde_json::from_str(&line).context("viewer did not return a startup URL")?;
            ensure!(response["event"] == "viewer_ready", "viewer startup failed");
            let mut ready = response["data"].clone();
            ready["pid"] = json!(child.id());
            ready["reused"] = json!(false);
            Ok(ready)
        })();
        match ready {
            Ok(ready) => {
                self.children.insert(
                    path,
                    Viewer {
                        child,
                        ready: ready.clone(),
                    },
                );
                Ok(ready)
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(error)
            }
        }
    }
    pub fn status(&mut self) -> Result<Value> {
        let mut viewers = Vec::new();
        for viewer in self.children.values_mut() {
            if viewer.child.try_wait()?.is_none() {
                viewers.push(viewer.ready.clone());
            }
        }
        self.children
            .retain(|_, viewer| viewer.child.try_wait().ok().flatten().is_none());
        Ok(json!({"viewers":viewers}))
    }
    pub fn close(&mut self, project: &Path) -> Result<Value> {
        let path = std::fs::canonicalize(project)?;
        if let Some(mut viewer) = self.children.remove(&path) {
            if viewer.child.try_wait()?.is_none() {
                viewer.child.kill()?;
            }
            viewer.child.wait()?;
            return Ok(json!({"closed":true,"project":path}));
        }
        Ok(json!({"closed":false,"project":path}))
    }
}
impl Drop for Viewers {
    fn drop(&mut self) {
        for viewer in self.children.values_mut() {
            let _ = viewer.child.kill();
            let _ = viewer.child.wait();
        }
    }
}
