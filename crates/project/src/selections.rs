//! Viewer annotations are separate from the grading graph, with an explicit output-space basis.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub id: String,
    pub revision: u64,
    pub source_hash: String,
    pub recipe_hash: String,
    pub output_node: String,
    pub width: u32,
    pub height: u32,
    pub timestamp: u64,
    pub mask: Mask,
    pub note: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    pub schema_version: u32,
    pub source_hash: String,
    pub items: Vec<Selection>,
}
pub fn path(project: &Path) -> PathBuf {
    let mut name = project.as_os_str().to_os_string();
    name.push(".selections.json");
    PathBuf::from(name)
}
/// Only geometry is accepted: annotations cannot import arbitrary files or color qualifiers.
pub fn validate_mask(mask: &Mask) -> Result<()> {
    mask.validate(0)?;
    fn geometry(mask: &Mask) -> bool {
        match mask {
            Mask::Ellipse { .. }
            | Mask::Rectangle { .. }
            | Mask::Polygon { .. }
            | Mask::Brush { .. } => true,
            Mask::Combine { masks, .. } => masks.iter().all(geometry),
            Mask::Invert { mask } => geometry(mask),
            _ => false,
        }
    }
    ensure!(geometry(mask), "viewer selection requires geometry masks");
    Ok(())
}
fn read_document(project: &Path, p: &Project) -> Result<Document> {
    let file = path(project);
    if !file.exists() {
        return Ok(Document {
            schema_version: 1,
            source_hash: p.source.hash.clone(),
            items: Vec::new(),
        });
    }
    ensure!(
        fs::metadata(&file)?.len() <= 16 * 1024 * 1024,
        "selection document exceeds 16MiB"
    );
    let doc: Document = serde_json::from_slice(&fs::read(file)?)?;
    ensure!(
        doc.schema_version == 1 && doc.source_hash == p.source.hash,
        "selection source/version mismatch"
    );
    ensure!(doc.items.len() <= 1024, "selection limit is 1024");
    let mut ids = std::collections::BTreeSet::new();
    for item in &doc.items {
        let rev = p.get_revision(item.revision)?;
        ensure!(ids.insert(&item.id), "duplicate selection id");
        ensure!(
            item.source_hash == p.source.hash
                && item.recipe_hash == rev.recipe_hash
                && item.output_node == rev.recipe.output,
            "selection basis mismatch"
        );
        ensure!(
            item.width > 0 && item.height > 0 && item.note.len() <= 4096,
            "invalid selection metadata"
        );
        validate_mask(&item.mask)?;
    }
    Ok(doc)
}
pub fn load(project: &Path) -> Result<Document> {
    let _lock = lock(project)?;
    let p = read(project)?;
    read_document(project, &p)
}
/// The caller renders the immutable revision to supply the actual output dimensions.
/// The project lock and expected head protect against edits between rendering and annotation.
pub fn save(project: &Path, expected: u64, mut item: Selection) -> Result<Selection> {
    let _lock = lock(project)?;
    let p = read(project)?;
    check_revision(&p, expected)?;
    let rev = p.get_revision(item.revision)?;
    ensure!(
        item.source_hash == p.source.hash
            && item.recipe_hash == rev.recipe_hash
            && item.output_node == rev.recipe.output,
        "selection basis mismatch"
    );
    ensure!(
        !item.id.is_empty() && item.id.len() <= 128,
        "invalid selection id"
    );
    ensure!(
        item.width > 0 && item.height > 0 && item.note.len() <= 4096,
        "invalid selection metadata"
    );
    validate_mask(&item.mask)?;
    item.timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let mut doc = read_document(project, &p)?;
    ensure!(doc.items.len() < 1024, "selection limit is 1024");
    ensure!(
        !doc.items.iter().any(|s| s.id == item.id),
        "selection id already exists"
    );
    doc.items.push(item.clone());
    let bytes = serde_json::to_vec_pretty(&doc)?;
    ensure!(
        bytes.len() <= 16 * 1024 * 1024,
        "selection document exceeds 16MiB"
    );
    vibecolor_io::atomic_bytes(&path(project), &bytes, true)?;
    Ok(item)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, PathBuf, Selection) {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("input.png");
        let path = dir.path().join("p.vcolor");
        vibecolor_io::export(
            &vibecolor_core::Frame::new(2, 1, vec![[0.2, 0.4, 0.6, 1.0]; 2]).unwrap(),
            &input,
            Default::default(),
        )
        .unwrap();
        let p = init(&path, &input, None).unwrap();
        let item = Selection {
            id: "region".into(),
            revision: 0,
            source_hash: p.source.hash.clone(),
            recipe_hash: p.head().unwrap().recipe_hash.clone(),
            output_node: "source".into(),
            width: 2,
            height: 1,
            timestamp: 0,
            mask: Mask::Rectangle {
                min: [0.0, 0.0],
                max: [0.5, 1.0],
                feather: 0.0,
            },
            note: "天空".into(),
        };
        (dir, path, item)
    }
    #[test]
    fn annotations_do_not_grade_and_survive_version_restore_and_reopen() {
        let (_dir, path, item) = fixture();
        let before = fs::read(&path).unwrap();
        assert!(load(&path).unwrap().items.is_empty());
        let saved = save(&path, 0, item).unwrap();
        assert_eq!(fs::read(&path).unwrap(), before);
        assert_eq!(load(&path).unwrap().items[0].recipe_hash, saved.recipe_hash);
        super::super::restore(&path, 0, 0, "restore".into()).unwrap();
        let read = load(&path).unwrap();
        assert_eq!(read.items[0].revision, 0);
        assert_eq!(read.items[0].output_node, "source");
        assert_eq!(read.items[0].note, "天空");
        let p = tag_revision(&path, 1, 0, "original".into()).unwrap();
        assert_eq!(p.tags["original"], 0);
        assert_eq!(p.revision, 2);
        assert_eq!(p.history.len(), 3);
    }
    #[test]
    fn stale_head_wrong_basis_duplicate_id_and_file_masks_fail_atomically() {
        let (_dir, path, mut item) = fixture();
        let e = save(&path, 1, item.clone()).unwrap_err();
        assert!(e.downcast_ref::<RevisionConflict>().is_some());
        assert!(!super::path(&path).exists());
        item.recipe_hash = "wrong".into();
        assert!(save(&path, 0, item.clone()).is_err());
        item.recipe_hash = super::super::load(&path)
            .unwrap()
            .head()
            .unwrap()
            .recipe_hash
            .clone();
        let mut asset = item.clone();
        asset.mask = Mask::Bitmap {
            path: "private.png".into(),
        };
        assert!(save(&path, 0, asset).is_err());
        save(&path, 0, item.clone()).unwrap();
        let before = fs::read(super::path(&path)).unwrap();
        assert!(save(&path, 0, item).is_err());
        assert_eq!(before, fs::read(super::path(&path)).unwrap());
    }
}
