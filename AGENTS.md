# Working on Tinge

Tinge is the plugin name; `vibecolor` is the native executable. Read README.md
and docs/status.md for implemented features and known limits.

## Code map

- `crates/color`: RGB colorimetry and transfer functions.
- `crates/core`: Frame, recipe DAG, masks, operations and scopes.
- `crates/ocio`: native OpenColorIO integration and CDL exchange.
- `crates/io`: decode, RAW, metadata and atomic image exports.
- `crates/project`: frozen assets, revisions, transactions and selections.
- `crates/engine`: rendering, caches, cancellation and LUT baking.
- `crates/cli/src/api.rs`: shared requests and session dispatch.
- `crates/cli/src/tools.rs` and `protocol.rs`: named MCP tools and transports.
- `crates/cli/src/outcome.rs`: shared partial-failure classification.
- `crates/cli/src/jobs.rs`, `workfiles.rs`, `web.rs`: background work,
  registered-file cleanup and loopback viewer.

Search first-party code first: `rg <pattern> crates docs scripts plugin tests`.
`vendor/` contains upstream sources; consult vendor/PATCHES.md before changes.
`target/` and `artifacts/` are generated and ignored.

## Invariants

- Preserve scene-linear sRGB/D65 float32, straight-alpha frame semantics.
- Keep project revision checks, OS locks, immutable asset hashes and atomic
  writes. A failed preview does not roll back a committed edit.
- Propagate partial failures through `outcome::failed`, retaining commit/export
  receipts. A workfile warning alone does not mean the export failed.
- Keep MCP tools explicitly named with complete schemas and truthful safety
  annotations. CLI/JSONL generic dispatch is not an MCP tool.
- Treat result verbosity, optional analysis and inline images independently.
- Cache keys must include all inputs that change computed values. Cache budgets
  do not bound peak process memory.
- Source/project/assets/history protection also applies to overwrite and cleanup.

## Verification

Use Rust 1.95 with rustfmt/clippy and a C++17/CMake toolchain (MSVC on Windows).

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --release --locked
python scripts/plugin-acceptance.py --executable target/release/vibecolor.exe
python scripts/agent-acceptance.py --executable target/release/vibecolor.exe
node --test scripts/viewer-client.test.cjs
```

On Linux omit `.exe`. Python/Node are development checks only. If changing
serialized request/recipe/edit types, regenerate schemas with
`scripts/export-schemas.ps1` and inspect the schema diff. Add regression tests
for observable bugs, including process-level tests for transport behavior.
Do not claim real-model tool-selection, camera-corpus or host compatibility
acceptance from structural tests alone.
