---
name: photo-workflow
description: Edit, mask, preview or export local photos and RAW images with Tinge. Use for local image processing or existing Tinge projects, not general color advice, image generation or video editing.
---

Use Tinge's named MCP tools with their visible schemas; no `command` or generic
request wrapper. Ask for a missing local source path or ambiguous target.
An attachment alone is not a known local path. No connection means no edit.

Interactive photo work defaults to the web viewer. Once the project is known or
created, call `tinge_viewer_open` and open its returned URL in the host browser
before the first adjustment; no separate request to show the webpage is needed.
Reuse an already open viewer/tab for that project, and keep its serving session
alive. Verify the page displays the intended project/revision; an inline image
or a returned URL alone does not mean the interactive page was shown. If the host
cannot open a browser, keep the viewer running, provide its URL and explain the
display limitation. Skip this for explicit no-webpage/batch-export requests or
read-only metadata inspection. Enabling the plugin without a photo/project does
not itself launch a viewer. See [viewer lifecycle](references/viewer-workflow.md).

Prefer the shortest useful loop:
- New image: `tinge_init`, then open the viewer. Use `tinge_grade` for an explicit
  one-off export without UI. Inspect metadata, RAW settings or capabilities only
  if uncertain.
- Existing project: `tinge_project_info` once for the revision; omit the recipe
  for basic grading. History is opt-in. After a successful edit, reuse the
  returned revision; re-read on conflict, reconnect or known external changes.
- Basic grading: `tinge_adjust` with observed `expect_revision`; combine exposure,
  contrast, saturation, vibrance, white balance and tones in one call. Values are
  absolute; omitted/null controls stay unchanged. Reuse `adjustment_id` (default
  `basic`) to update without stacking; a new ID appends a group. Use
  `_inline_image:true` when evaluating pixels. Do not also request preview,
  compare or stats unless they answer a separate need.
- Advanced graphs/masks: load `tinge_edit_preview` and read `include_recipe:true`
  only when needed. Preserve the existing graph. A conflicting/bypassed basic
  group needs reconciliation or a deliberately new adjustment ID.
- Heavy work: set `background:true` on the same tool, keep its returned job ID,
  and check `tinge_job_status` without tight polling. New mutations get new
  idempotency keys; identical retries reuse the key and arguments in that session.
- Source statistics: `tinge_analyze`; project recipe statistics: `tinge_stats`.
  `tinge_lut_bake` takes a project and optional revision. Direct recipe processing
  belongs to advanced `tinge_grade`, `tinge_validate` and graph editing.
- Export: `tinge_render` for the requested revision/path/color space. The running
  viewer follows new revisions automatically; do not reopen it after each edit.
  Scopes, full diagnostics and history remain optional.

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
