# Third-party sources and release review

This document records third-party notices. It does not choose a license for this project or grant a release approval.

| Component | Source and applicable notice |
| --- | --- |
| Kindle backend | Vendored from the exact revision in `vendor/slint-backend-kindle/UPSTREAM_REVISION`; MIT OR Apache-2.0. Original licenses and local patch notes are retained beside the source. |
| ZIP parser | The already resolved `zip` 4.6.1 source is retained under `vendor/zip` for a bounded metadata-constructor patch. MIT; the original `LICENSE`, exact crate checksum/upstream revision, and local changes are recorded beside that source. This does not select the application's license. |
| Chinese interface and reading fonts | Noto CJK; SIL Open Font License 1.1. `assets/fonts/OFL.txt`, `manifest.json`, and `docs/FONTS.md` contain the full notice, revisions, hashes, and reproducible preparation procedure. |
| Offline pinyin dictionary | AOSP PinyinIME; Apache-2.0. `assets/pinyin/` contains the original notice, exact revision, data format, and reproducible conversion script. |
| WeRead protocol reference | `moooyo/weread.koplugin` at the revision recorded in `docs/API_REFERENCE.md`; the reference identifies AGPL-3.0 and `Copyright (C) 2026 finlater and contributors`. Review obligations for translated or derived implementation before distribution. |
| Slint 1.17.1 | The inspected crate declares `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`. The already resolved Core 1.17.1 is preserved under `vendor/i-slint-core` with a native-edit admission patch; its published checksum, exact revision, original declaration, and source changes are retained beside the source. Select an applicable distribution model before publication. |
| Swash 0.2.10 | The already resolved package is preserved under `vendor/swash` with a physical outline-mask admission patch. Apache-2.0 OR MIT; both original license files, the exact published checksum/revision, and patch/source provenance are retained. Its package tests reuse the already locked flate2 1.1.10. No dependency-version upgrade or application-license choice is made. |
| Zeno 0.3.3 | The already resolved renderer dependency is preserved under `vendor/zeno` with opt-in fallible fill-raster resource admission used by Swash Outline rendering. Apache-2.0 OR MIT; both original licenses, source revision/checksum and patch notes are retained. Its unused benchmark dev dependencies are not added to the application; genuine integration regressions run in the existing Swash test binary. |
| Other Rust dependencies | Exact development versions are recorded in `Cargo.lock`. Their individual licenses must be reviewed together with the intended project distribution model. |
| Native X11 handoff runtime | The project-owned frame guard uses the device's existing X11 and glibc runtime through a separate native process. The development archive includes no Amazon firmware modules, X11 shared libraries, or system loader. Existing X11 headers are build inputs; this adds no Rust dependency-version change or application-license selection. Firmware source and ABI research remain private and are not package inputs. |

The demo contains interface metadata and public-domain classical Chinese texts. It is separate from real accounts and downloaded content. The demo illustration is generated locally by project code. No upstream book files, real account tokens, personal notes, or private research archives are included in source or development bundles.
