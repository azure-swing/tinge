//! Only registered, unchanged project work files are eligible for recycling.
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
use vibecolor_project as project;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Preview,
    Draft,
    Export,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    hash: String,
    bytes: u64,
    revision: u64,
    role: Role,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u32,
    source_hash: String,
    final_revision: Option<u64>,
    entries: Vec<Entry>,
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}
fn canonical_project(path: &Path) -> Result<PathBuf> {
    Ok(fs::canonicalize(path)?)
}
struct Lock(fs::File);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
fn lock(path: &Path) -> Result<Lock> {
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(sidecar(path, ".workfiles.lock"))?;
    file.lock_exclusive()?;
    Ok(Lock(file))
}
fn read(path: &Path, source: &str) -> Result<Document> {
    let file = sidecar(path, ".workfiles.json");
    if !file.exists() {
        return Ok(Document {
            version: 1,
            source_hash: source.into(),
            ..Default::default()
        });
    }
    ensure!(
        fs::metadata(&file)?.len() <= 4 * 1024 * 1024,
        "workfile registry exceeds 4MiB"
    );
    let doc: Document = serde_json::from_slice(&fs::read(file)?)?;
    ensure!(
        doc.version == 1 && doc.source_hash == source,
        "workfile registry belongs to a different project source"
    );
    ensure!(
        doc.entries.len() <= 4096,
        "workfile registry exceeds 4096 entries"
    );
    Ok(doc)
}
fn save(path: &Path, doc: &Document) -> Result<()> {
    vibecolor_io::atomic_bytes(
        &sidecar(path, ".workfiles.json"),
        &serde_json::to_vec(doc)?,
        true,
    )
}

pub fn preview_path(path: &Path, revision: u64, edge: u32, compare: bool) -> Result<PathBuf> {
    let path = canonical_project(path)?;
    let p = project::load(&path)?;
    let rev = p.get_revision(revision)?;
    let folder = sidecar(&path, ".work");
    fs::create_dir_all(&folder)?;
    Ok(folder.join(format!(
        "{}-r{}-{}-{}.png",
        if compare { "compare" } else { "preview" },
        revision,
        &rev.recipe_hash[..16],
        edge
    )))
}
pub fn edit_path(path: &Path) -> Result<PathBuf> {
    let path = canonical_project(path)?;
    let folder = sidecar(&path, ".work");
    fs::create_dir_all(&folder)?;
    Ok(folder.join(format!("edit-{}.png", crate::web::random_token()?)))
}

fn candidate_path(path: &Path, p: &project::Project, stored: &str) -> Result<PathBuf> {
    let relative = Path::new(stored);
    ensure!(
        !relative.is_absolute()
            && relative
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
        "workfile path must stay inside project directory"
    );
    let file = project::base(path).join(relative);
    // Reject links/junctions in any component before resolving the target.
    let mut cursor = project::base(path).to_path_buf();
    for part in relative.components() {
        cursor.push(part.as_os_str());
        let meta = fs::symlink_metadata(&cursor)?;
        ensure!(
            !meta.file_type().is_symlink(),
            "linked workfile is not eligible"
        );
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            ensure!(
                meta.file_attributes() & 0x400 == 0,
                "reparse workfile is not eligible"
            );
        }
    }
    ensure!(file.is_file(), "only regular work files can be recycled");
    let file = fs::canonicalize(file)?;
    ensure!(
        file.starts_with(project::base(path)),
        "workfile resolves outside project directory"
    );
    crate::api::protect_project(path, p, &file)?;
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    ensure!(
        ["png", "jpg", "jpeg", "tif", "tiff", "exr"].contains(&ext.as_str()),
        "only registered image work files can be recycled"
    );
    Ok(file)
}

pub fn register(path: &Path, file: &Path, revision: u64, role: Role) -> Result<Value> {
    let path = canonical_project(path)?;
    let p = project::load(&path)?;
    p.get_revision(revision)?;
    let file = fs::canonicalize(file)?;
    let relative = file
        .strip_prefix(project::base(&path))
        .context("workfiles must be stored inside the project directory")?
        .to_string_lossy()
        .into_owned();
    candidate_path(&path, &p, &relative)?;
    let hash = vibecolor_io::hash_file(&file)?;
    ensure!(
        hash != p.source.hash,
        "source originals cannot be registered as workfiles"
    );
    let entry = Entry {
        path: relative.clone(),
        hash,
        bytes: fs::metadata(&file)?.len(),
        revision,
        role,
    };
    let _lock = lock(&path)?;
    let mut doc = read(&path, &p.source.hash)?;
    doc.entries.retain(|e| e.path != relative);
    ensure!(doc.entries.len() < 4096, "workfile registry is full");
    doc.entries.push(entry);
    save(&path, &doc)?;
    Ok(json!({"registered":true,"path":file,"role":role,"revision":revision}))
}

fn plan(path: &Path, p: &project::Project, doc: &Document, revision: u64) -> Value {
    let mut eligible = Vec::new();
    let mut retained = Vec::new();
    let mut skipped = Vec::new();
    for entry in &doc.entries {
        if entry.hash == p.source.hash {
            skipped.push(json!({"path":entry.path,"reason":"source original is protected"}));
            continue;
        }
        if entry.role == Role::Export || (entry.role == Role::Draft && entry.revision == revision) {
            retained.push(json!({"path":entry.path,"revision":entry.revision,"role":entry.role}));
            continue;
        }
        match candidate_path(path, p, &entry.path).and_then(|file| {
            ensure!(
                vibecolor_io::hash_file(&file)? == entry.hash,
                "file changed since registration"
            );
            Ok(file)
        }) {
            Ok(file) => eligible.push(
                json!({"path":file,"stored_path":entry.path,"bytes":entry.bytes,"hash":entry.hash}),
            ),
            Err(error) => skipped.push(json!({"path":entry.path,"reason":error.to_string()})),
        }
    }
    let bytes: u64 = eligible.iter().map(|e| e["bytes"].as_u64().unwrap()).sum();
    json!({"revision":revision,"final_revision":doc.final_revision,"files":eligible,"bytes":bytes,"retained":retained,"skipped":skipped,"destination":"system_recycle_bin","history_preserved":true})
}
pub fn cleanup_plan(path: &Path, revision: u64) -> Result<Value> {
    let path = canonical_project(path)?;
    let p = project::load(&path)?;
    p.get_revision(revision)?;
    let _lock = lock(&path)?;
    Ok(plan(&path, &p, &read(&path, &p.source.hash)?, revision))
}
#[cfg(test)]
pub fn final_revision(path: &Path) -> Option<u64> {
    let path = canonical_project(path).ok()?;
    let p = project::load(&path).ok()?;
    final_revision_for(&path, &p)
}
pub fn final_revision_for(path: &Path, p: &project::Project) -> Option<u64> {
    let path = canonical_project(path).ok()?;
    let _lock = lock(&path).ok()?;
    read(&path, &p.source.hash).ok()?.final_revision
}

pub fn finalize(path: &Path, expected: u64, revision: u64) -> Result<Value> {
    finalize_with(path, expected, revision, recycle)
}
fn finalize_with(
    path: &Path,
    expected: u64,
    revision: u64,
    mut recycle_file: impl FnMut(&Path) -> Result<()>,
) -> Result<Value> {
    let path = canonical_project(path)?;
    let _lock = lock(&path)?;
    project::with_locked_project(&path, |p| {
        if p.revision != expected {
            return Err(project::RevisionConflict {
                expected,
                actual: p.revision,
            }
            .into());
        }
        p.get_revision(revision)?;
        let mut doc = read(&path, &p.source.hash)?;
        let planned = plan(&path, p, &doc, revision);
        // Persist the user's selected version before attempting fallible OS work.
        doc.final_revision = Some(revision);
        save(&path, &doc)?;
        let mut moved = Vec::new();
        let mut failed = Vec::new();
        let mut bytes = 0u64;
        for item in planned["files"].as_array().unwrap() {
            let stored = item["stored_path"].as_str().unwrap();
            let result = candidate_path(&path, p, stored).and_then(|file| {
                ensure!(
                    vibecolor_io::hash_file(&file)? == item["hash"].as_str().unwrap(),
                    "file changed before recycling"
                );
                recycle_file(&file)?;
                ensure!(!file.exists(), "system did not recycle file");
                Ok(())
            });
            match result {
                Ok(()) => {
                    bytes += item["bytes"].as_u64().unwrap();
                    moved.push(item["path"].clone());
                    doc.entries.retain(|e| e.path != stored);
                }
                Err(error) => failed.push(json!({"path":item["path"],"reason":error.to_string()})),
            }
        }
        // Missing records are dropped; modified files stay protected and indexed.
        doc.entries
            .retain(|e| project::base(&path).join(&e.path).exists());
        let registry_error = save(&path, &doc).err().map(|e| e.to_string());
        Ok(
            json!({"finalized":true,"revision":revision,"project_head":p.revision,"recycled":moved,"recycled_bytes":bytes,"failed":failed,"skipped":planned["skipped"],"retained":planned["retained"],"registry_error":registry_error,"history_preserved":true,"destination":"system_recycle_bin"}),
        )
    })
}

#[cfg(not(windows))]
fn recycle(_path: &Path) -> Result<()> {
    anyhow::bail!("system recycle bin is not implemented on this platform; files were retained")
}
#[cfg(windows)]
fn recycle(path: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::{
            Storage::FileSystem::GetDriveTypeW,
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize,
            },
            UI::Shell::{
                FOF_ALLOWUNDO, FOF_NO_CONNECTED_ELEMENTS, FOF_NOCONFIRMATION, FOF_NOERRORUI,
                FOF_SILENT, FOF_WANTNUKEWARNING, FOFX_EARLYFAILURE, FOFX_RECYCLEONDELETE,
                FileOperation, IFileOperation, IShellItem, SHCreateItemFromParsingName,
                SHQUERYRBINFO, SHQueryRecycleBinW,
            },
        },
        core::PCWSTR,
    };
    // The shell requires STA. This short-lived thread is only created on finalization.
    let path = path.to_path_buf();
    std::thread::spawn(move || -> Result<()> {
        struct Apartment;
        impl Drop for Apartment {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .context("initialize recycling apartment")?;
            let _apartment = Apartment;
            let drive = match path.components().next() {
                Some(std::path::Component::Prefix(prefix)) => match prefix.kind() {
                    std::path::Prefix::Disk(drive) | std::path::Prefix::VerbatimDisk(drive) => {
                        drive
                    }
                    _ => {
                        anyhow::bail!("network workfiles are retained; local recycle bin required")
                    }
                },
                _ => anyhow::bail!("absolute drive path required for recycling"),
            };
            let root = [drive as u16, ':' as u16, '\\' as u16, 0];
            ensure!(
                GetDriveTypeW(PCWSTR(root.as_ptr())) == 3,
                "workfile drive has no supported fixed-drive recycle bin; file retained"
            );
            let mut bin = SHQUERYRBINFO {
                cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
                ..Default::default()
            };
            SHQueryRecycleBinW(PCWSTR(root.as_ptr()), &mut bin)
                .context("recycle bin unavailable; file retained")?;
            let operation: IFileOperation =
                CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER)
                    .context("create recycling operation")?;
            operation
                .SetOperationFlags(
                    FOF_ALLOWUNDO
                        | FOF_SILENT
                        | FOF_NOCONFIRMATION
                        | FOF_NOERRORUI
                        | FOF_WANTNUKEWARNING
                        | FOFX_RECYCLEONDELETE
                        | FOFX_EARLYFAILURE
                        | FOF_NO_CONNECTED_ELEMENTS,
                )
                .context("set recycle-only operation flags")?;
            // Shell parsing names use the drive form; canonical Win32 paths
            // carry a verbatim prefix that some Shell implementations reject.
            let parsing = path.to_string_lossy();
            let parsing = parsing.strip_prefix(r"\\?\").unwrap_or(&parsing);
            let text = std::ffi::OsStr::new(parsing)
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(text.as_ptr()), None)
                .context("resolve recycling item")?;
            operation
                .DeleteItem(&item, None)
                .context("queue recycling item")?;
            operation
                .PerformOperations()
                .context("move workfile to recycle bin")?;
            ensure!(
                !operation.GetAnyOperationsAborted()?.as_bool(),
                "system recycling was aborted; file retained where possible"
            );
        }
        Ok(())
    })
    .join()
    .map_err(|_| anyhow::anyhow!("recycle thread failed"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.png");
        let frame = vibecolor_core::Frame::new(3, 2, vec![[0.2, 0.3, 0.4, 1.0]; 6]).unwrap();
        vibecolor_io::export(&frame, &source, Default::default()).unwrap();
        let path = dir.path().join("test.vcolor");
        project::init(&path, &source, None).unwrap();
        project::tag(&path, 0, "draft".into()).unwrap();
        (dir, path)
    }
    #[test]
    fn recycling_only_moves_unchanged_registered_work_and_keeps_history() {
        let (dir, path) = fixture();
        for (name, rev, role) in [
            ("preview.png", 1, Role::Preview),
            ("draft.png", 0, Role::Draft),
            ("final.png", 1, Role::Draft),
            ("export.png", 0, Role::Export),
            ("changed.png", 0, Role::Preview),
        ] {
            fs::write(dir.path().join(name), b"image").unwrap();
            register(&path, &dir.path().join(name), rev, role).unwrap();
        }
        fs::write(dir.path().join("changed.png"), b"user changes").unwrap();
        fs::write(dir.path().join("unregistered.png"), b"keep").unwrap();
        let frozen = project::base(&path).join(&project::load(&path).unwrap().source.path);
        assert!(register(&path, &path, 1, Role::Preview).is_err());
        assert!(register(&path, &frozen, 1, Role::Preview).is_err());
        assert!(register(&path, &dir.path().join("source.png"), 1, Role::Preview).is_err());
        assert!(finalize_with(&path, 0, 1, |_| panic!("conflict must not move files")).is_err());
        let before = fs::read(&path).unwrap();
        let bin = dir.path().join("fake-bin");
        fs::create_dir(&bin).unwrap();
        let done = finalize_with(&path, 1, 1, |file| {
            fs::rename(file, bin.join(file.file_name().unwrap()))?;
            Ok(())
        })
        .unwrap();
        assert_eq!(done["recycled"].as_array().unwrap().len(), 2);
        for name in [
            "final.png",
            "export.png",
            "changed.png",
            "unregistered.png",
            "source.png",
        ] {
            assert!(dir.path().join(name).exists());
        }
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(frozen.exists());
        assert_eq!(final_revision(&path), Some(1));
        assert_eq!(
            finalize_with(&path, 1, 1, |_| panic!("retry must not move files")).unwrap()["recycled_bytes"],
            0
        );
    }
    #[test]
    fn failed_recycling_preserves_file_and_final_selection() {
        let (dir, path) = fixture();
        let file = dir.path().join("preview.png");
        fs::write(&file, b"keep").unwrap();
        register(&path, &file, 1, Role::Preview).unwrap();
        let done = finalize_with(&path, 1, 1, |_| anyhow::bail!("bin unavailable")).unwrap();
        assert_eq!(done["failed"].as_array().unwrap().len(), 1);
        assert!(file.exists());
        assert_eq!(final_revision(&path), Some(1));
    }
}
