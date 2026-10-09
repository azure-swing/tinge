---
name: photo-workflow
description: Edit, mask, preview or export local photos and RAW images with Tinge. Use for local image processing or existing Tinge projects, not general color advice, image generation or video editing.
---

Use Tinge's named MCP tools with their visible schemas; no `command` or generic
request wrapper. Ask for a missing local source path or ambiguous target.
An attachment alone is not a known local path. No connection means no edit.

Prefer the shortest useful loop:
- New image: `tinge_init` for revisioned work, or `tinge_grade` for a one-off
  recipe/export. Inspect metadata, RAW settings or capabilities only if uncertain.
- Existing project: `tinge_project_info` with `include_recipe:true` once to learn
  the graph and revision. History is opt-in. After a successful edit, reuse the
  returned revision; re-read on conflict, reconnect or known external changes.
- Ordinary edits: `tinge_edit_preview`, observed `expect_revision`, managed output
  (omit `output`), and `_inline_image:true` when evaluating pixels. Do not also
  request preview, compare or stats unless they answer a separate need.
- Heavy work: use the corresponding `tinge_submit_*`, keep its actual job ID,
  and check `tinge_job_status` without tight polling. New mutations get new
  idempotency keys; identical retries reuse the key and arguments in that session.
- Export: `tinge_render` for the requested revision/path/color space. Viewer,
  scopes, full diagnostics and history are optional, not prerequisites.

A failed preview or cancelled job may still have committed: inspect `commit`,
`committed`, `revision` and `preview_error` before retrying. On conflict, reconcile
with the new graph; do not just replace the expected revision. Jobs and keys are
session-local; reconnect requires checking project/output state.

Saved marks: read `tinge_selections` when relevant, verify source/recipe hashes,
output node and full dimensions before using the mask. Export does not authorize
cleanup: only use `tinge_cleanup_plan` then `tinge_finalize` for an explicitly
chosen final revision and authorized recycling. `temporary:true` means draft.
Overwrite only within the user's scope; protect source/project/assets/history.

Load only the reference needed: [operator units and masks](references/operators.md),
[RAW](references/raw-workflow.md), [color pipeline](references/color-pipeline.md),
[viewer selections](references/viewer-workflow.md), or
[cleanup and partial failures](references/storage-workflow.md).

Report the changed revision and preview/export, adding only actionable warnings.
Keep routine replies brief. Do not claim queued work is finished or an unread
image was visually inspected. `_response:"full"` keeps full structured diagnostics;
pixels and analysis have separate switches.
