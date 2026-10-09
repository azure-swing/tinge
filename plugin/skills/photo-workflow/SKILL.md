---
name: photo-workflow
description: Edit, compare, mask, preview or export local photos and RAW images using Tinge and its revisioned color recipes. Use for actual local image processing or an existing Tinge project; general color advice, image generation and video editing do not require this workflow.
---

Use Tinge's bundled native engine through its named MCP tools. Tools have complete static input schemas;
do not send a `command`, generic request, or batch. Explicit user instructions
take precedence over this workflow. If the MCP connection is missing, explain
the missing local connection instead of claiming an edit was made.

Inputs are a local image/project path, the desired visual change, and an export
destination when exporting. Ask for a missing source path or ambiguous target.
An attached image alone is not a known local path. Do not invent paths, camera
support, neutral colors, revision numbers, output color spaces, or job IDs.

1. For a new image, use `tinge_inspect` (and `tinge_raw_plan` if needed),
   then `tinge_init` to create a new project when revisioned editing is wanted.
   For an existing project, use `tinge_project_info` with `include_recipe:true`
   and a small history page to obtain the current revision and graph.
2. Plan edits from the user's intended look. Use `tinge_selections` for saved
   marks and verify their source, recipe, output dimensions and coordinate basis
   before using them as masks. Load [operator guidance](references/operators.md)
   only for unfamiliar controls, units or constraints. Inspect capabilities when
   support is uncertain; implemented controls do not imply Resolve/Lightroom parity.
3. Use `tinge_edit_preview` for synchronous edits, or
   `tinge_submit_edit_preview` with an `idempotency_key` for background work.
   Pass the observed `expect_revision`. Each new edit gets a new key; an identical
   retry in the same session reuses the key and identical arguments. Omit `output`
   for a managed preview. Previews write files; only overwrite explicit files when
   replacement is within the user's authorized scope.
4. Read `tinge_job_status` using the returned ID until a terminal result;
   avoid tight polling. `tinge_job_cancel` requests cancellation but cannot
   undo a commit. A failed preview or cancelled job can still have committed edits.
   Check `commit`, `committed`, `revision` and `preview_error` before another edit.
   On revision conflict, re-read the project and reassess against the new recipe;
   do not blindly substitute a new revision number. After reconnect/restart, keys
   and jobs are gone: inspect project and output state before retrying a write.
5. View the returned preview resource using `resources/read` or request
   `_inline_image:true` when pixels are needed. Use `tinge_compare` for a saved
   before/after PNG, or `tinge_viewer_open` for zooming, marks and history UI.
   The viewer is optional and all editing/exporting works without it. Request
   `tinge_stats`/scopes only when helpful; `_response:"full"` includes histograms.
6. Use `tinge_render` or `tinge_submit_render` for the requested revision,
   destination, bit depth and color space. Export does not finalize or clean up.
   Only mark an export `temporary:true` when it is a disposable draft. RAW/color
   details: load [RAW](references/raw-workflow.md) or
   [color pipeline](references/color-pipeline.md) only as needed.
7. When the user has explicitly selected a final revision and authorized cleanup,
   use `tinge_cleanup_plan`, inspect the files, then `tinge_finalize` (or
   its background counterpart) with the current expected revision. Do not infer
   finality from the latest head or from an export request. Recycling is destructive
   even though Windows may allow recovery. Partial failure leaves finalization
   committed; report failed/skipped/retained files. Load
   [storage guidance](references/storage-workflow.md) for cleanup details.

Report the actual changed revision, preview/export path and relevant warnings.
Do not claim a visual result was inspected unless you read the image, or claim
success when only a queued receipt was returned. For partial success, state the
successful commit and the failed stage separately. Preview URIs and viewers are
session-local; source/project/assets remain local and no cloud transfer is needed.
