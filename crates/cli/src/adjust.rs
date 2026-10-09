//! A small, revision-checked editing surface for the ordinary grading loop.
use crate::api::Request;
use anyhow::{Context, Result, ensure};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;
use tinge_core::{Node, Operation};
use tinge_project::{self as project, Edit};

fn adjustment_id() -> String {
    "basic".into()
}
fn edge() -> u32 {
    1600
}
fn label() -> String {
    "basic adjustment".into()
}

/// Absolute controls; omitted/null values preserve the previous adjustment.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Adjust {
    pub project: PathBuf,
    pub expect_revision: u64,
    /// Reuse this ID to update the same group; a new ID appends another group.
    #[serde(default = "adjustment_id")]
    pub adjustment_id: String,
    /// Exposure in stops; zero is neutral.
    #[serde(default)]
    #[schemars(range(min = -24, max = 24))]
    pub exposure: Option<f32>,
    #[serde(default)]
    #[schemars(range(min = 0, max = 4))]
    pub contrast: Option<f32>,
    #[serde(default)]
    #[schemars(range(min = 0, max = 4))]
    pub saturation: Option<f32>,
    #[serde(default)]
    #[schemars(range(min = -1, max = 1))]
    pub vibrance: Option<f32>,
    /// Scene-linear RGB gains, each 0.001..100; [1,1,1] is neutral.
    #[serde(default)]
    pub white_balance: Option<[f32; 3]>,
    #[serde(default)]
    #[schemars(range(min = -1, max = 1))]
    pub shadows: Option<f32>,
    #[serde(default)]
    #[schemars(range(min = -1, max = 1))]
    pub highlights: Option<f32>,
    #[serde(default)]
    #[schemars(range(min = -1, max = 1))]
    pub whites: Option<f32>,
    #[serde(default)]
    #[schemars(range(min = -1, max = 1))]
    pub blacks: Option<f32>,
    #[serde(default = "edge")]
    #[schemars(range(min = 1, max = 16384))]
    pub max_edge: u32,
    #[serde(default = "label")]
    pub label: String,
}

impl Adjust {
    pub fn prepare(self) -> Result<Request> {
        ensure!(
            !self.adjustment_id.is_empty()
                && self.adjustment_id.len() <= 64
                && self
                    .adjustment_id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-'),
            "adjustment_id must be 1..64 ASCII letters, digits, underscores or hyphens"
        );
        ensure!(
            (1..=16384).contains(&self.max_edge),
            "max_edge must be 1..16384"
        );
        ensure!(
            [
                self.exposure,
                self.contrast,
                self.saturation,
                self.vibrance,
                self.shadows,
                self.highlights,
                self.whites,
                self.blacks
            ]
            .iter()
            .any(Option::is_some)
                || self.white_balance.is_some(),
            "provide at least one adjustment"
        );
        let p = project::load(&self.project)?;
        if p.revision != self.expect_revision {
            return Err(project::RevisionConflict {
                expected: self.expect_revision,
                actual: p.revision,
            }
            .into());
        }
        let recipe = &p.head()?.recipe;
        let ids = ["wb", "primary", "tone"]
            .map(|stage| format!("__tinge_{}_{}", self.adjustment_id, stage));
        let existing = ids
            .each_ref()
            .map(|id| recipe.nodes.iter().find(|n| &n.id == id));
        let count = existing.iter().filter(|n| n.is_some()).count();
        ensure!(
            count == 0 || count == 3,
            "adjustment group is incomplete; reconcile it with edit_preview or choose a new adjustment_id"
        );
        let new_group = count == 0;
        let mut nodes = if new_group {
            let operations: [Operation; 3] = [
                serde_json::from_value(json!({"type":"white_balance","gains":[1,1,1]}))?,
                serde_json::from_value(json!({"type":"primary"}))?,
                serde_json::from_value(json!({"type":"tone"}))?,
            ];
            operations
                .into_iter()
                .enumerate()
                .map(|(i, op)| Node {
                    id: ids[i].clone(),
                    inputs: vec![if i == 0 {
                        recipe.output.clone()
                    } else {
                        ids[i - 1].clone()
                    }],
                    op,
                    mask: None,
                    mix: 1.0,
                    enabled: true,
                })
                .collect::<Vec<_>>()
        } else {
            let nodes = existing.map(|n| n.unwrap().clone());
            ensure!(
                matches!(nodes[0].op, Operation::WhiteBalance { .. })
                    && matches!(nodes[1].op, Operation::Primary { .. })
                    && matches!(nodes[2].op, Operation::Tone { .. })
                    && nodes[0].inputs.len() == 1
                    && nodes[1].inputs == [ids[0].clone()]
                    && nodes[2].inputs == [ids[1].clone()],
                "adjustment group conflicts with the existing graph; reconcile it with edit_preview or choose a new adjustment_id"
            );
            let mut pending = vec![recipe.output.as_str()];
            let mut visited = std::collections::BTreeSet::new();
            while let Some(id) = pending.pop() {
                if !visited.insert(id) {
                    continue;
                }
                if let Some(node) = recipe.nodes.iter().find(|n| n.id == id) {
                    pending.extend(node.inputs.iter().map(String::as_str));
                }
            }
            ensure!(
                visited.contains(ids[2].as_str()),
                "adjustment group is bypassed by the current output; reconcile it or choose a new adjustment_id"
            );
            nodes.into()
        };
        if let Operation::WhiteBalance { gains } = &mut nodes[0].op
            && let Some(value) = self.white_balance
        {
            *gains = value;
        }
        if let Operation::Primary {
            exposure,
            contrast,
            saturation,
            vibrance,
            ..
        } = &mut nodes[1].op
        {
            for (target, value) in [
                (exposure, self.exposure),
                (contrast, self.contrast),
                (saturation, self.saturation),
                (vibrance, self.vibrance),
            ] {
                if let Some(value) = value {
                    *target = value;
                }
            }
        }
        if let Operation::Tone {
            shadows,
            highlights,
            whites,
            blacks,
        } = &mut nodes[2].op
        {
            for (target, value) in [
                (shadows, self.shadows),
                (highlights, self.highlights),
                (whites, self.whites),
                (blacks, self.blacks),
            ] {
                if let Some(value) = value {
                    *target = value;
                }
            }
        }
        for node in &nodes {
            node.op.validate().context("invalid basic adjustment")?;
        }
        let mut edits: Vec<_> = nodes
            .into_iter()
            .map(|node| Edit::UpsertNode { node })
            .collect();
        if new_group {
            edits.push(Edit::SetOutput { id: ids[2].clone() });
        }
        // The transaction rechecks the revision under the project OS lock.
        // Reuse the ordinary commit/preview path so partial failures keep receipts.
        Ok(Request::EditPreview {
            project: self.project,
            expect_revision: self.expect_revision,
            edits,
            output: None,
            label: self.label,
            asset_base: None,
            max_edge: self.max_edge,
            overwrite: false,
        })
    }
}
