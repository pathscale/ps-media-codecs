# Browser probe fixtures

The repository contains small Opus and VP9 packet sequences under
`browser-probe/data/`. They are synthetic output of `examples/codec_probe.rs`
(sine tones for Opus, a generated gradient for VP9). No third-party media is
involved, so there is no source-media redistribution question.

They are source inputs for `browser-probe/index.html`, not successful test
results or broad conformance coverage. Regenerate them with
`cargo run --example codec_probe -- --export-dir <absolute path to browser-probe/data>`.

Open items:

- Add a documented consumer with explicit expected decoded behavior.
- Check malformed-input handling against independent vectors.
