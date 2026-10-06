# Codec source provenance

This crate links the following published Rust implementations through
caret-versioned Cargo dependencies. The complete source archives used for this
initial adapter review are retained under the local `lanes/upstream` audit
directory; they are not vendored here.

| Crate | Range | Source | Reviewed archive SHA-256 | License file retained here |
| --- | --- | --- | --- | --- |
| `rusty-opus` | `^1.0.0` | <https://crates.io/crates/rusty-opus/1.0.0> | `c6f45b2cc805b83960083f25b77df0c678cae5ac89a3138e6994aed8c62e2cba` | `LICENSES-rusty-opus-COPYING` |
| `rusty_vp9` | `^0.1.1` | <https://crates.io/crates/rusty_vp9/0.1.1> | `b4f7698cee9ea00ee8ee104766c47fced2fb60486a2ce6e854ab453533dbd63b` | `LICENSES-rusty-vp9-Apache-2.0` |

The reviewed manifests declare no production dependencies for either codec.
The Opus manifest has development-only dependencies, including an exact-pinned
allocator dev dependency; it is not part of the downstream production graph.
Both codec implementations contain Rust `unsafe` SIMD kernels with scalar
fallbacks. No C ABI, FFI, `build.rs`, or native library dependency was found in
the published source reviewed for this starter.

The Opus package carries BSD-3-Clause terms and its included patent-license
references. The VP9 package carries Apache-2.0 terms. Preserve both complete
license files when redistributing this adapter or its binaries.
