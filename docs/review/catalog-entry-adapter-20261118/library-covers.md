# Offline Library cover fixture

The default Scribe cover grid contains fifteen public demo books in shelf order,
`demo-0` through `demo-14`. This fixture gives every book a different grayscale
PNG; a repeating four-band pattern encodes its ordinal plus one. It retains the
normal titles, badges, reading progress, and chapter data. The three
`article:demo-article-*` records are outside the shelf. Publication and chapter
pictures, the launcher Icon, and full-size sleep covers are not Library targets.
The unseeded demo has no book-cover URLs and is a fallback control only.

The [seed example](../../../tools/offline-ui-fixture/examples/seed-library-covers.rs)
creates a new data directory. It generates the images without network access,
uses the production `Store` and `FileCache::put_cover` APIs, reads each thumbnail
back, and writes the book/URL/key/PNG-digest bindings to `library-covers.json`.
All URLs use `example.invalid`. The thumbnail-key constructor is private; a
production screenshot checks that the test encoding agrees with the actual
Library read path. Only the separate test package gains an example.

From the source root on `test-env`, with the locked dependencies already cached:

```sh
set -eu
work=$(mktemp -d)
export CARGO_TARGET_DIR=/path/to/this-tasks-private-cargo-target
export CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=6 SLINT_SCALE_FACTOR=1
cargo build --locked --offline --bin weread
cargo build --locked --offline --manifest-path tools/offline-ui-fixture/Cargo.toml --example seed-library-covers
"$CARGO_TARGET_DIR/debug/examples/seed-library-covers" --data-dir "$work/seed"
tar -C "$work/seed" -czf "$work/seed.tar.gz" demo library-covers.json
sha256sum "$work/seed.tar.gz"
mkdir "$work/warm-data"
tar -C "$work/warm-data" -xzf "$work/seed.tar.gz"
for run in warm fallback; do
  xvfb-run -a -s '-screen 0 1280x1400x24' "$CARGO_TARGET_DIR/debug/weread" \
    --device scribe --demo --offline --screen library --data-dir "$work/$run-data" \
    --capture "$work/$run.png"
done
python3 tools/offline-ui-fixture/check-library-covers.py --fixture "$work/seed" \
  --frame "$work/warm.png" --metadata "$work/warm.metadata.json" \
  --fallback-frame "$work/fallback.png" --report "$work/pixel-check.json"
```

The [checker](../../../tools/offline-ui-fixture/check-library-covers.py) requires
the production desktop capture metadata and a 930 by 1240, scale-one PNG. For
each predetermined cell it compares interior pixels with the central 151 by 210
crop of the 400 by 560 thumbnail and all fourteen alternatives. In the actual
scale-one render the image is centered at its intrinsic size and clipped by
the tile. The pattern repeats so that each center crop remains unique. The
checker excludes badges and progress bars and allows
one pixel of raster rounding inside that cell. A report includes the thresholds,
per-book error and separation, and the exact rejected IDs for four controls:
the real fallback render, one missing cover, two swapped covers and a gray frame.
It does not accept a Kindle scanout or an unknown geometry.

Freeze one closed seed archive and restore that same archive for both arms; the
demo records ordinary timestamps. A newly generated SQLite tree is not a
byte-identical replacement. A successful final host frame establishes the
rendered, nonempty target set. It does not timestamp the first qualifying frame,
the formal document entry, the display submission or panel completion, catalog
notifications, native Library restoration, or acquisition of online covers.

## Verified on 2026-11-18

The host run used `test-env`, Xvfb, the production desktop `weread` built from
Git `ee97decd839b27f820ba2c843f98f76f62d59de5`, and the test example above.
The source archive was compared with the extracted product inputs after a worker
restart. A damaged pre-restart ELF was rejected; only the rebuilt ELF below
produced the reported images. No Kindle was used.

| Input | SHA-256 |
| --- | --- |
| Production host ELF | `267037f759d26565960bfad4eaab9b5f3def307adaffcb6173f53b04ed71d9ef` |
| Seed example source | `bf096fc4314c115c165e3397fc961481e6823f054b7653fcac28c0905557c716` |
| Pixel checker source | `f8095f181bb821ba0fc0adad16b4feb7ba5c1f51e9fc36b0cdaea420409ee2d6` |
| Isolated seed archive (27,418 bytes) | `446e1afb4b17d0110e353004dc34c010dd570d986e87ce1001a7a38ef6ff3847` |
| [Ordered book and thumbnail bindings](library-cover-bindings.json) | `6b6f86e535ace6b88e0e8f21b7fe60877f320f77dd054ae3d2b7f04d596be01f` |

The [unaltered warm frame](library-cover-frames/warm.png) matched **15/15**
correct thumbnail interiors with zero mean absolute error. The closest
incorrect thumbnail was at least 42.29 grayscale levels farther away on
average. All four [pixel-check controls](library-cover-frames/pixel-check.json)
rejected exactly their expected IDs, including the [actual unseeded product
frame](library-cover-frames/fallback.png). The one-cell and swapped-cell
controls in that report are composites of actual product pixels.

A separate application launch extracted the same archive with only the cached
PNG identified for `demo-7` excluded. The [resulting product frame](library-cover-frames/missing.png)
matched the other fourteen books, and the unchanged [checker rejected only
`demo-7`](library-cover-frames/missing-check.json), as required. Every run used
a separate data directory; the pristine seed tree still matched the frozen
archive after the checks. The untracked operator artifact
`artifacts/catalog-entry/public-library-cover-seed.tar.gz` holds the frozen
snapshot for a future, independently authorized comparison.
