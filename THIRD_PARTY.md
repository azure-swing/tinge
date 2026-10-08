# Third-party components

VibeColor's own source is MIT licensed. Dependency versions are pinned in Cargo.lock.

- rawler 0.8: LGPL-2.1, [DNGLab source](https://github.com/dnglab/dnglab), Daniel Vogelbacher and contributors. Used for camera RAW decoding, sensor scaling/demosaicing, and camera metadata. Source is unmodified. The `raw` feature in vibecolor-io enables it by default; building/distributing a combined binary must meet the dependency's LGPL terms, including the relevant source and relinking rights. The complete Cargo sources and lockfile allow rebuilding and replacing this dependency.
- moxcms 0.8: BSD-3-Clause OR Apache-2.0, [source](https://github.com/awxkee/moxcms), Radzivon Bartoshyk and contributors. Used for actual ICC profile parsing, transforms, and encoding.
- ocio-rs / ocio-sys 0.2.1: BSD-3-Clause, [binding source](https://github.com/shaloong/ocio-rs). Bundled feature builds the packaged OpenColorIO 2.5.2 C++ source and links it statically. OpenColorIO is BSD-3-Clause, [upstream](https://github.com/AcademySoftwareFoundation/OpenColorIO). Native dependencies include Expat, yaml-cpp, Imath, pystring, minizip-ng and zlib with their own licenses. Distribution must include the exact native library and builtin ACES configuration notices as well as Rust crate licenses.
- image, png, tiff, exr and their dependencies: format codecs; see the exact package manifests/license files in the Cargo dependency sources.
- serde, serde_json, schemars, clap, rayon, tempfile, fs2, blake3, base64 and their dependencies: CLI, serialization, execution, storage and protocol infrastructure.

The cutout algorithms are implemented in Rust from the GrabCut (Rother/Kolmogorov/Blake, 2004) and closed-form matting (Levin/Lischinski/Weiss, 2007) formulations. No OpenCV/PyMatting runtime or pretrained model weights are bundled. Development references use OpenCV 4.12.0 and PyMatting 1.1.16 / SciPy 1.16.1; oracle versions and sources are recorded in the fixtures and reports. The Eileen Collins example photograph is NASA public-domain material distributed by scikit-image; download URL, SHA-256 and source attribution are in `artifacts/cutout-reference.json`.

This is an attribution overview, not a full bundled license inventory. Before publishing binary releases, package the exact dependency licenses and comply with each dependency's distribution requirements. Resolve and Lightroom are comparison targets; their code, private algorithms, models and assets are not bundled.
