# Bundled Chinese Fonts

Recorded: 2026-11-03 (Asia/Shanghai).

The application bundles Simplified Chinese faces from Noto CJK for interface text, book titles, and reading text. Seven resources provide Sans Regular (400), Medium (500), and Bold (700), and Serif Light (300), Regular (400), Medium (500), and Bold (700). They are compressed with gzip for storage and decompressed before registration with Slint and the reader's font metrics implementation. The actual Medium face supplies the interface's 500-weight text; Serif Light and Medium supply the corresponding reader weight choices.

## Sources and license

The source collections came from Debian's `fonts-noto-cjk` and `fonts-noto-cjk-extra` packages, both version `1:20240730+repack1-1`. The additional weights use the same reviewed upstream font releases as the original four resources.

| Source package | Selected weights | Archive SHA-256 |
| --- | --- | --- |
| [fonts-noto-cjk](https://deb.debian.org/debian/pool/main/f/fonts-noto-cjk/fonts-noto-cjk_20240730+repack1-1_all.deb) | Sans/Serif Regular and Bold | `f5dc28a754e17327d99f0a612134d92c8dd6187314ae967cb77f25df60860139` |
| [fonts-noto-cjk-extra](https://deb.debian.org/debian/pool/main/f/fonts-noto-cjk/fonts-noto-cjk-extra_20240730+repack1-1_all.deb) | Sans Medium; Serif Light and Medium | `16081e6982d2c8de4d773cf124c09ea25e6e1e472cd7fc62a58a52a1c241b41a` |

| Family | Upstream release | Reviewed tag revision | Embedded copyright |
| --- | --- | --- | --- |
| Noto Sans CJK SC | [Sans 2.004](https://github.com/notofonts/noto-cjk/tree/Sans2.004/Sans) | `523d033d6cb47f4a80c58a35753646f5c3608a78` | Copyright 2014-2021 Adobe |
| Noto Serif CJK SC | [Serif 2.003](https://github.com/notofonts/noto-cjk/tree/Serif2.003/Serif) | `9b0f1436e455d902de067a2501422e5dc71ad16b` | Copyright 2017-2024 Adobe |

The font resources are distributed under SIL Open Font License 1.1. The complete font license and retained copyright notices are included in [OFL.txt](../assets/fonts/OFL.txt). License and copyright entries also remain inside the font name tables. These notices cover the bundled fonts; the application's project license remains an independent, unresolved decision.

The original upstream license files are also preserved without modification:

| Resource | Original file at the reviewed revision | Local copy | SHA-256 |
| --- | --- | --- | --- |
| Sans 2.004 | [Repository-root LICENSE](https://github.com/notofonts/noto-cjk/blob/523d033d6cb47f4a80c58a35753646f5c3608a78/LICENSE) | [NotoSansCJK-LICENSE.txt](../assets/fonts/NotoSansCJK-LICENSE.txt) | `6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2` |
| Serif 2.003 | [Serif/LICENSE](https://github.com/notofonts/noto-cjk/blob/9b0f1436e455d902de067a2501422e5dc71ad16b/Serif/LICENSE) | [NotoSerifCJK-LICENSE.txt](../assets/fonts/NotoSerifCJK-LICENSE.txt) | `6a73f9541c2de74158c0e7cf6b0a58ef774f5a780bf191f2d7ec9cc53efe2bf2` |

Both pinned files contain OFL's general Reserved Font Name definition and condition 3. Neither file declares a specific Reserved Font Name. Remote inspection of all seven original SC faces' name-table copyright, trademark, and license entries and CFF Notice fields found Adobe copyright notices and the statement `Noto is a trademark of Google Inc.`, with no additional Reserved Font Name declaration in those inspected locations. The original, exact font notices are reproduced in [NOTICES.txt](../assets/fonts/NOTICES.txt), including the missing space in the upstream Sans Medium CFF Notice. The original four compressed resources and all three existing license files retain their recorded bytes.

[manifest.json](../assets/fonts/manifest.json) records both package archive hashes, the source TTC SHA-256, selected face index, embedded version, resulting OTF SHA-256, compressed resource SHA-256, exact byte counts, glyph counts, and Unicode coverage for every resource. Family and style names are read from typographic name IDs 16/17, falling back to legacy IDs 1/2. Medium and Light have a style suffix in their legacy family names but share their regular face's typographic family. Those original names remain intact. Source `OS/2.usWeightClass` values must match the declared 300/400/500/700 weights, and runtime registration checks that Fontique exposes each additional weight under the same actual family as Regular.

| Resource | Compressed bytes | Decompressed bytes | Retained Unicode codepoints |
| --- | ---: | ---: | ---: |
| `NotoSansSC-Regular.otf.gz` | 6,419,672 | 7,547,448 | 33,126 |
| `NotoSansSC-Medium.otf.gz` | 6,447,212 | 7,519,256 | 33,126 |
| `NotoSansSC-Bold.otf.gz` | 6,498,293 | 7,551,936 | 33,126 |
| `NotoSerifSC-Light.otf.gz` | 8,405,942 | 10,186,696 | 33,093 |
| `NotoSerifSC-Regular.otf.gz` | 8,528,062 | 10,301,996 | 33,093 |
| `NotoSerifSC-Medium.otf.gz` | 8,650,669 | 10,393,872 | 33,093 |
| `NotoSerifSC-Bold.otf.gz` | 8,913,231 | 10,602,916 | 33,093 |

The compressed bundle is 53,863,081 bytes (51.37 MiB); decompressed font data totals 64,104,120 bytes (61.13 MiB). The three additional resources add 23,503,823 compressed bytes and 28,099,824 decompressed bytes. This size retains the declared source character coverage in each weight rather than reducing the bundle to a fixed list of common characters.

## Coverage and transformation

`prepare.py` requires the recorded TTC hashes and locates each Simplified Chinese face by its typographic family and style names. It verifies a unique match and the actual source weight before extraction. It retains every source-mapped character in the requested Unicode ranges, including Latin, combining marks, punctuation, Greek, Cyrillic, CJK radicals, kana, Bopomofo, CJK Unified Ideographs and Extension A, supplementary CJK extension ranges, CJK compatibility ideographs, and variation-selector ranges when represented by the source font.

The exact retained ranges are included in the manifest. Requesting a Unicode range preserves the characters available in the source; it does not supply characters missing from that source. An imported font may be needed for characters outside the source fonts' coverage.

The subset preserves default Simplified Chinese glyph forms and the `ccmp`, `liga`, `clig`, `kern`, and `calt` layout features. It removes regional substitution features and vertical-layout alternatives because the application's current layout is horizontal; vertical cover titles are composed as individual horizontal glyphs. Hinting instructions are removed for the software-rendered bundle. Glyph outlines, Unicode mappings, family names, version records, and license entries remain intact.

The compressed resources use an empty gzip filename and `mtime=0`. Only compressed font files are stored in the repository; the extracted OTF files are temporary preparation output. Keeping full source CJK coverage in seven faces requires more space than a subset restricted to interface strings. The manifest's byte counts are the source of truth for the bundle size. Regular remains the default pagination metrics face. The application font records also retain metrics for every registered bundled weight so reading layout can select the same actual weight as rendering. Custom font records accumulate metrics for their registered weights under each actual family.

The optional LXGW WenKai face is not included in this bundle. The application can discover supported user-provided TTF and OTF files in its data directory's `fonts` folder.

The resource candidate retains the 64 MiB per-file limit and adds checked
initialization admission: 16 raw faces per file, 128 faces overall, 128 MiB actual
input-read work, and 256 MiB original-blob plus full-metric-copy forecasts.
Application name/record/axis forecasts have an 8 MiB allowance; raw variation
axes are limited to 64 and final families to 4 KiB. Selected possible family-name
spans are charged before temporary Fontique decoding. Directory enumeration
admits at most 256 entries and 1 MiB of encoded paths before retaining them.
Failed or overwritten work does not refund possibly retained sources.

Every file is validated and its complete metrics/choices are prepared in a
nonshared collection before shared registration. Rejection skips the entire
file and preserves previous choices. The seven bundled payloads total
64,104,120 input bytes and forecast 128,208,240 blob/metric bytes when registered
and copied once; the defaults also admit one additional 64 MiB single-face font.
These are logical project policies, not total renderer RSS or wall-clock caps.
The caret/font changes passed the corrected 818-library/23-binary checkpoint,
including all 20 Main font cases after per-test-thread software platform setup.
The independent 64 × 1024 × 1024 raw-name scan-work allowance also charges each
root/directory/table/record search before visiting it. These results and later
source corrections retain their boundaries in the
[resource record](RESOURCE_LEAK_REVIEW.md).

## Reproduction

Preparation was run through `ssh test-env` using Python 3.13.5 and fontTools 4.57.0. No local application validation was run.

Install both exact source packages and fontTools 4.57.0 in the remote preparation environment, then use these commands from the repository directory in PowerShell. Preserve the existing license files; they already cover the additional weights from the same releases.

```powershell
scp assets/fonts/prepare.py test-env:/tmp/weread-fonts-prepare.py
ssh test-env 'python3 -I /tmp/weread-fonts-prepare.py --output-dir /tmp/weread-fonts'
scp "test-env:/tmp/weread-fonts/*.otf.gz" assets/fonts/
scp test-env:/tmp/weread-fonts/manifest.json assets/fonts/
```

Alternatively, extract the reviewed `fonts-noto-cjk-extra` archive with `dpkg-deb -x` and pass its `usr/share/fonts/opentype/noto` directory to `--extra-source-dir`; the original package can stay installed in the default source directory. The script refuses a different source package version or any source TTC whose recorded hash differs.

The preparation script reloads every extracted OTF and checks that its Unicode map exactly matches the source map intersected with the declared ranges. It verifies the retained family, style, and weight, and requires OFL metadata. A separate remote inspection verified all seven compressed and decompressed SHA-256 values, families, and embedded OFL records, and reproduced the original four output hashes. Subsequent application rendering checks and any Kindle acceptance evidence must be recorded separately.
