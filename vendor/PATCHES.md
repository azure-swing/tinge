# Local OpenColorIO Rust binding patch

`ocio-rs` and `ocio-sys` are vendored from their published 0.2.1 crates.
Their original licenses and the OCIO 2.5.2 source licenses are preserved.

VibeColor adds `Config::archive_bytes()` and a length-bearing C bridge function.
The upstream string archive binding reads ZIP data as a NUL-terminated C string;
this truncates binary OCIOZ archives. The new API copies the exact byte count,
preserves native exception reporting, and exports no borrowed storage to callers.
The original string API remains for compatibility but is not used by VibeColor.
