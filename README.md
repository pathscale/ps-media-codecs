# ps-media-codecs

A small, pure-Rust integration starter for bounded Opus audio and VP9 video.
The public adapter currently covers 48 kHz mono/stereo Opus in 20 ms frames and
VP9 profile 0 (8-bit 4:2:0) up to 1920×1080. VP9 superframes are limited to two
coded frames per packet. Input packet limits and declared VP9 dimensions are
checked before decode allocation; YUV encoder planes and Opus frame buffers
are checked before codec calls.

The codec implementations are consumed as ordinary crates.io dependencies with
caret ranges. The adapter does not vendor codec sources or add a C/C++ library.
The adapter is Apache-2.0; upstream license texts and exact source archive
hashes are recorded in
[`UPSTREAM.md`](UPSTREAM.md).

This is an integration starter, not a production-readiness claim. In
particular, the VP9 encoder source describes its current encoder as “Floor 3,
C3” and acknowledges that it is younger than libvpx. The library adapter has
been compiled. The executable probe passed five stereo 20 ms Opus frames and
four 64×48 VP9 frames. The exported packets also decoded successfully in an
independent browser through WebCodecs: 4,800 audio samples per channel and four
video frames with visible/display dimensions 64×48. Formatting and all-target
clippy with warnings denied passed. These small sequences do not establish
general conformance, reverse decoding, real-time throughput or Chuzz support.

The VP9 adapter does not expose the upstream encoder's process-environment
controls as crate settings. `Vp9Encoder::new()` and every `encode_frame()`
call reject the presence of any environment variable whose name begins with
`VP9_`, without printing its name or value or changing the process environment.
This check is performed before upstream encoder construction and frame
encoding. However, `rusty_vp9` reads many inherited `VP9_*` variables during
configuration and encoding. These can change activity
modeling, interpolation filters, trellis and transform search, partition and
motion-search decisions, reference-chain behavior, and dispatch budgeting;
debug/profiling switches can also add stderr output. Therefore the adapter's
fixed `qindex` and `speed` settings do not by themselves guarantee identical
encoder behavior across differently configured processes. Because the
environment is process-wide and upstream reads some controls after the adapter
check, concurrent mutation of `VP9_*` variables while an encoder is in use is
outside this guard's guarantee. Keep those variables absent and the process
environment stable for the encoder's lifetime; a complete library-level
guarantee requires upstream to remove the implicit reads. The executable probe
also refuses to run when any `VP9_*` variable is set. File-output variables
mentioned in the upstream source are confined to its test-only code and are not
part of the normal downstream library path.

The retained upstream VP9 archive contains one embedded `keyframe.vp9`
fixture, but it is profile 1 / RGB and does not exercise this adapter's
profile-0 4:2:0 path. The Opus crate's local
roundtrip/oracle checks are useful code-level evidence, but they do not replace
an independent decoder comparison for this adapter.

## Repeat the executable and browser checks

Run `cargo run --example codec_probe` for the local encode/decode probe. To
export synthetic packets, pass `-- --export-dir` followed by the absolute path
to `browser-probe/data` in this checkout. Serve `browser-probe` on a loopback
HTTP origin, open it in a WebCodecs-capable browser, and click **Run browser
decode**. The page checks actual decoded samples and visible/display frame
dimensions, allows coded-frame padding, and closes every output frame. It does
not capture, play or upload media. Generated packet JSON is ignored by Git.
