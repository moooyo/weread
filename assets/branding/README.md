# WeRead launcher cover

The operator accepted this artwork on 2026-11-17 (Asia/Shanghai): the `WeRead`
title above a Kindle-style reader displaying a playful, single-eye-winking
penguin. The cover has no subtitle. This is the application's Kindle Library
entry artwork, not a replacement for books in the user's reading library or
the current-book sleep cover.

![Accepted WeRead launcher cover](weread-cover-thumbnail.png)

## Assets

- `weread-cover.png`: the accepted 1024 x 1536 lossless grayscale master.
- `weread-cover-thumbnail.png`: the 256 x 384 grayscale Library thumbnail.
- `weread-cover.prompt.txt`: the final image-edit prompt.

The artwork was generated with the upstream `gpt-image-2.5-flare` model through
the imagegen CLI. Post-processing converted the result to 8-bit grayscale and
resized the thumbnail with Lanczos sampling. The earlier edit targets and
unselected concepts remain operator-local under ignored `output/imagegen/`.
No account metadata or credentials are part of these assets. Accepting the
artwork does not select a project license or imply endorsement by Amazon.

## Launcher integration

`packaging/kindle/document-launcher.sh` embeds the thumbnail as a single, unwrapped
`# Icon: data:image/png;base64,...` header on its third line. The development
builder copies that script to `documents/weread.sh`; the archive already
includes it and `weread/launch.sh` as explicit executable inputs. No external image path or runtime
image-generation dependency is required on the device.

The reviewed SH_Integration parser reads only the first six lines, accepts the
`# Icon: ` field, decodes its data URI into the script's `.sdr` directory, and
passes that file to the native catalog as the thumbnail. Keep the metadata near
the top, unwrapped, and byte-identical to `weread-cover-thumbnail.png` after
base64 decoding. Update the image and header together when changing the cover.

The contract was reviewed in
[SH_Integration `adb7ef5c570a25c76fd64e94837ae0f590bd3b9c`](https://github.com/KindleModding/sh_integration/tree/adb7ef5c570a25c76fd64e94837ae0f590bd3b9c):
`utils/utils.c` reads the header and `extractor/main.c` extracts and registers
the icon. This is the submodule revision pinned by jb.sh v2.2.1, used by the
existing firmware-emulator setup. The project reuses the metadata protocol,
not the upstream implementation. Source review and packaging verification do
not establish display behavior on a physical Kindle.

See [Kindle packaging](../../packaging/kindle/README.md#application-entry-cover)
for the installation boundary and verification status.
