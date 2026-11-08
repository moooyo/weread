# Physical outline-mask allocation admission

## Pinned source

This overlay applies only to the already resolved Swash 0.2.10 package from
`https://github.com/dfrg/swash`, VCS revision
`7773843df0d63cd468db61a29c152b5e7a99d4ab`. Its published crate SHA-256 is
`6c2499c2d826531388872b2268718aed907a39bd785ab0dcfe57fab26283f92e`.
Keep the original `LICENSE-APACHE` and `LICENSE-MIT` files and the original
`Apache-2.0 OR MIT` package notice. This does not select the project license.

## Current producer and change

The pinned Slint software renderer 1.17.1 `fonts/vectorfont.rs` uses
`Render::new(&[Source::Outline]).format(Format::Alpha)` before its later viewport
clip and glyph-cache admission. The patch changes only this outline-mask
allocation boundary. Bitmap, PNG, strike, and ColorOutline behavior are not
changed.

The same Zeno `Mask` first computes and reports its actual physical dimensions.
Checked width/height, pixel count, stride, requested bytes, and operation work
admission then precede `try_reserve_exact` and `resize`. Rendering uses that same
inspected mask and its admitted stride. Alpha is one byte per pixel; Zeno's
Subpixel and CustomSubpixel formats are four bytes per pixel. The shared layout
math also prices three-channel buffers without treating Subpixel as RGB3.

The project envelope is 2,048 pixels per side, 4 Mi pixels, 16 MiB per requested
buffer, and 64 MiB of requested allocation work per render operation. Public
configuration can tighten these ceilings. Failed attempts do not refund work.
These are requested-buffer policies, not total allocator RSS, glyph CPU time,
physical-device acceptance, or a global DPI limit.

Resource refusal returns the existing `false`/`None` no-glyph result and records
one typed `RasterLimitError` for inspection. It does not update the caller's
image, font metrics, family, weight, or normalized coordinates. The software
renderer already skips a missing glyph result; no UI or frame contract changes.

## Prepared regression scope

Six small unit cases exercise the real outline builder: exact/one-less Alpha
limits, the four-point UPEM-16 mask at the menu's 30 physical pixels, preserved
caller image and metrics, Alpha/RGBA-Subpixel byte counts, checked overflow and
hard ceilings, and normal Noto bytes/placement against the original known-small
Zeno oracle. The large mask never reaches an allocation or the oracle.

The unit fixture reuses the repository's original tiny font generator and
existing Noto compressed assets. The only requested test-manifest addition is
the already resolved `flate2` 1.1.10 dev dependency. The operator owns complete
package copying, Cargo manifest/lock integration, and notice updates.

No execution was performed while preparing the fixed overlay. This source
record is not a package-test or compilation receipt. Complete verification is
recorded separately for the matching canonical candidate; the previous
candidate's results do not verify this patch.
