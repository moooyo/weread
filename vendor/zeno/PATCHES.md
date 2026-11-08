# Fallible fill-raster resource admission

The original Zeno 0.3.3 source, revision and Apache-2.0 OR MIT licenses are
preserved. `SOURCE_PROVENANCE.json` records the modified source identities.
No package-version upgrade or application-license decision is made.

An opt-in `Mask::render_into_with_budget` guards actual Fill commands, line and
curve transitions, row heads, linked Cell visits, requested Vec capacity, and
output samples. Limits can only tighten the 1 Mi Cell, 16 MiB requested-storage
and 64 Mi logical-access envelopes. Accepted reset/growth/work requests are not
refunded. Checked geometric capacity growth pays spare/delta storage, resident
and old/new temporary spans, and potential relocation work before reserve,
push, or link changes. The first refusal is sticky and stops the real producer.

Current Swash Outline rendering enables this budget and stages its bounded
image target. Only successful fill rendering commits the caller's Image;
refusal preserves its old data, placement, source and content. Original
unconfigured APIs retain their behavior. Stroke, Bitmap/PNG, allocator RSS,
arbitrary iterator code, kernel behavior and wall-clock deadlines receive no
new acceptance claim.

Nine small genuine regressions are registered in the existing Swash test
binary. This source-integration record is not a test or compilation receipt;
complete verification and final artifact identity are recorded separately for
the matching canonical candidate.
