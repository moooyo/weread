# WeRead native Library tile

This research helper targets the normal WeRead card on the first-generation Scribe, firmware 5.19.5. It does not open a file URI, use a memorized point, write to the catalog, read book labels, or save screenshots. It refuses an unrecognized stock view, an absent or ambiguous public cover, a held pointer button, wrong geometry or a changed target. A successful XTest click is software input to the genuine native card, not a physical finger or an optical observation.

## Reviewed inputs

The upstream copy is [BiliComics, commit `9175002`](https://github.com/moooyo/bilicomics/tree/9175002958174ff236e0f4c33072dc278189e88e/docs/physical/startup-optimization-20261119/resume/rebuilt). `native-target.c` supplies the X image reader, bilingual footer OCR, active-tab check, pointer restoration, XTest pulse, `XSync` and `CLOCK_MONOTONIC` brackets. The `FixtureDisplay` used by `native_tile_test.py` is from **that commit's** `test-native-target.py`; it paints synthetic Xvfb pixels, not Kindle images. [Aidoku's public locator at `1976df8`](https://github.com/moooyo/aidokuForKindle/tree/1976df8f66a20f84cbb07c1601d0252beae1ee0f/docs/review/startup-native-20261121/input) is a design reference for rechecking the public artwork after moving the pointer; no Aidoku kit, trained target or private frames are inputs.

`native_tile_prepare.py` consumes the original C source and **this project's** `assets/branding/weread-cover-thumbnail.png` (256 by 384). It requires the reviewed source sections; changes to their boundaries stop generation. It restricts OCR to the stock Home/Library footer, recognizes the entire public artwork rather than matching any book title, checks 1860 by 2480 geometry and disables core dumps. Four regions independently check the WeRead letters, reader frame and penguin against dark and light points. The active program compares the full target region between frames and reconfirms uniqueness immediately before pressing. The state record is created only after those checks.

## Build without a Kindle

On the Linux verification host, start with a **new, private** `WORK` directory, a trusted WeRead checkout `WEREAD`, and a Git checkout `BILI` containing the public revision below. Do not copy another project's runtime stage. All artifacts are verified and packaged **before** a separate operator considers requesting a device lease. Do not run these commands on a Kindle.

```sh
set -eu
umask 077
mkdir -m 700 "$WORK"
revision=9175002958174ff236e0f4c33072dc278189e88e
prefix=docs/physical/startup-optimization-20261119/resume/rebuilt
for file in fetch-runtime.sh build-target.sh pack-kit.py test-native-target.py; do
    git -C "$BILI" show "$revision:$prefix/$file" >"$WORK/$file"
done
git -C "$BILI" show "$revision:$prefix/native-target.c" >"$WORK/public-source.c"
cp "$WEREAD"/tools/startup-observer/native_tile{.sh,_artwork.c,_prepare.py,_test.py,_package.py} "$WORK/"
cp "$WEREAD/assets/branding/weread-cover-thumbnail.png" "$WORK/"
ubuntu_run=/opt/kindle-toolchain/ubuntu-run
"$ubuntu_run" "$WORK" bash fetch-runtime.sh
"$ubuntu_run" --build "$WORK" sh -ec '
  sh -n native_tile.sh
  python3 -B native_tile_prepare.py public-source.c weread-cover-thumbnail.png /work
  sh build-target.sh'
"$ubuntu_run" "$WORK" sh -ec '
  mkdir -p test-mnt/us; touch test-mnt/us/.synthetic-media
  mount --bind /work/test-mnt /mnt
  export WEREAD_XVFB_FIXTURE=1
  xvfb-run -a -s "-screen 0 1860x2480x24 -nolisten tcp" python3 -B native_tile_test.py'
"$ubuntu_run" "$WORK" sh -ec 'python3 -B pack-kit.py --paired; python3 -B native_tile_package.py'
```

Use Bash for the host commands above. `fetch-runtime.sh` uses the Ubuntu archive keyring and signed Noble ARM distribution indexes to download the SDK, loader, X11/XTest/Tesseract libraries and English/Simplified Chinese models into this new work directory. Its selected runtime package versions are recorded in `apt-arm/selected-runtime.tsv`; archive revisions may advance. The shared `ubuntu-run` supplies Zig, QEMU, Xvfb, Pillow and an isolated mount/PID namespace. Only the test namespace's `/mnt` is overlaid, and the fixture checks the marker inode before creating a fake stage. The resulting `native-tile-qualified.tar.gz` contains regular USTAR files under `tile/` including both ARM binaries, their actual loader closure, the models, and `native_tile.sh`. The packer explicitly compares the ARM executables with those tested. No build source, fixture picture, user content or credential is needed in the device package.

## Owned-stage interface

The outer authorized transaction provides `/mnt/us/.weread-render-$label`, a 16-lowercase-hex public label in its `owner` file, the `tile/` subtree, exclusive access, bounded operation timeouts, an independently verified normal catalog identity, and orderly app shutdown and native recovery. Run `sh "$stage/tile/native_tile.sh" OP "$label"`:

- `inspect` reads stock tabs only and reports `home|library X Y` privately, with the original pointer in root coordinates.
- `prepare` saves `tile-original` before any navigation, enters Library only if necessary, and checks for exactly one public WeRead cover. It sets `tile-prepared`; it does not click the application card. No scroll or Home-recents fallback is permitted.
- `tap` creates an exclusive `tile-gate` before its last OCR pass. This earlier marker is suitable for arming an observer but **is not a click**. The ARM input program creates `tile-dispatch-intent` exclusively after validating the exact owner, active tab, unique unchanged target and pointer, immediately before generating button-down. It also requires the same label in the regular `observer-active` file, a nonblocking exclusive lock attempt rejected as busy on that file, and no `render-cancel` entry. This last pre-press liveness check is not a completion receipt. Failure without the intent means no WeRead button-down, although the pointer or native tab may have moved. If the operation is still alive, wait for its bounded exit before using absence as evidence. Any existing or partial intent without a valid receipt means **possibly dispatched**: no retry, no substitution, no inferred release. On success stdout and `tile-release` contain the same JSON; the two `up_*_ns` fields bracket the XTest button release and button-state acknowledgement on `CLOCK_MONOTONIC`, in nanoseconds. Do not use the down bracket or the shell clock instead.
- `restore` first requires native ownership and restores and revalidates the original tab and pointer. It does not terminate an application, clear a foreign lock, or attest to stock `activeView`. The outer controller must first close only the proven owned visit and then verify the original foreground, view, lights, input roster and installations before removing its stage and releasing the device. A visible stock shell alone is not proof of recovery after an uncertain press.
