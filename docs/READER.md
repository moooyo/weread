# Reader and Offline Input

## Ownership

`src/reader.rs` owns plain-text measurement, pagination, stable character anchors,
and reader gestures. It has no Slint, SQL, network, or Kindle ioctl dependency.
`src/input.rs` owns the touch keyboard's editable query, pinyin composition,
offline candidates, language modes, and submission behavior. Slint emits intents
and displays these models; it does not simulate content or authentication.

## Pagination contract

`Layout` uses logical pixels. Callers must subtract page margins, headers,
footers, and any visible Scribe note panel before passing its content width and
height. The platform layer applies the Kindle's factor of two exactly once.
Changing a menu overlay must not change this layout.

`TextMetrics::from_font` reads a TTF, OTF, or TTC face through owned_ttf_parser. The selected
collection index must match the face used for display. A standalone SC font uses
index zero. A system Noto CJK collection may require selecting its SC face.
`TextMetrics::default()` is an explicitly approximate CJK fallback for an absent
font, not visual acceptance evidence. It estimates Latin advances and does not
justify a claim of exact typography.

The cached face retains the source bytes and reads horizontal advances from
font tables on demand, scaling font units by `units_per_em`. Traditional
horizontal `kern` subtables supply pair kerning when present. It does not
pre-parse every CJK glyph outline or rasterize glyphs, avoiding startup work and
memory proportional to all of a large CJK font's outlined shapes.
`from_font_owned(Vec<u8>, index)` also permits the caller to transfer a loaded
font buffer without an extra byte copy. Complex GPOS shaping remains outside
this plain-text measurement module.

`Paginator::new(text, &layout, &metrics)` returns at least one page, including for
empty text. Page and line ranges are half-open offsets measured in Unicode
scalars in the original chapter body. They are never UTF-8 byte offsets. CRLF
and paragraph separators remain covered by page ranges. A line's `start..end`
selects its original visible text; page ranges include intervening separators.

`new_with_first_page_height(text, &layout, &metrics, first_page_height)` reserves
space for the chapter heading on the first page while later pages use the full
`layout.height`. Its `reflow_with_first_page_height` counterpart keeps a source
anchor under the same geometry rule.

Wrapping uses measured glyph advances and kerning, preserves Latin words where
they fit, avoids starting a wrapped line with closing punctuation, and avoids
ending one with opening punctuation. Combining marks stay with their base
character. A pathological one-character-wide layout may exceed its measure
to avoid stranding closing punctuation, and always makes progress.

`seek` and `reflow` retain a source character anchor rather than an old page
number. This permits restoring reading position after changing font size,
line spacing, margins, or the Scribe note panel. `Line::offset_at_x` resolves
a horizontal coordinate to a nearby source boundary. Applications can combine
that result with `text_range` for persistent selections and annotations.

`Line::paragraph_end`, `width`, and `gap_after` expose the information a renderer
needs for paragraph endings, optional justification, and paragraph spacing.
`Layout::justify` does not itself mutate strings by inserting spaces. The UI
renderer must apply the corresponding visual spacing consistently when using
coordinate-based selection.

This is a Unicode-safe plain-text paginator, not a complete EPUB/CSS, bidi,
complex-script shaping, or grapheme-cluster layout engine. EPUB parsing and
Kindle pen or pinch event delivery require their own accepted implementations.
It does not invent a fixed number of pages or assume a source offers EPUB files.

## Inline book images

`Paginator::new_with_images(text, &layout, &metrics, first_page_height, images)`
inserts verified `InlineImage` occurrences at Unicode scalar boundaries in the
canonical chapter text. Existing constructors retain their original text-only
geometry. Each `Line` also exposes its page-relative `y` and allocated `height`,
including the effect of preceding image slots and paragraph spacing.

`InlineImage.width` and `height` describe the bitmap's intrinsic logical size.
The caller converts cached physical pixel dimensions to logical dimensions.
The paginator scales down to the text measure and the page's available body
height while retaining the bitmap's aspect ratio. It reserves a separate
`IMAGE_CAPTION_HEIGHT` of 24 logical pixels. `PlacedImage.width` is the fitted
bitmap width; `PlacedImage.height` includes the caption, and `y` is the top of
the entire slot. The UI centers the bitmap and uses `height - 24` for its image
area, with the caption below. Valid target-device bodies are taller than this
caption area; pathological tiny layouts remain bounded but cannot show a full
image and caption.

An image that does not fit after existing content moves to the next page rather
than shrinking into an unusably small remainder. Tall images fit the complete
body of that page. Multiple occurrences at the same anchor keep input order.
An image inside a text line forces a safe split and a complete rewrap of the
following text. The shortened line before the image is not justified across
the full measure, and continuation text does not receive another first-line
indent. Paragraph delimiters remain part of the original character ranges;
an image before a delimiter does not invent an extra blank line. Invalid or
unmapped image geometry is omitted rather than relocating it speculatively.

Image-only pages can have identical start and end character offsets. A text
anchor denotes the position before every image at that boundary, so `seek`
selects the earliest page containing an image at the requested anchor. Buttons
and `next_page` still reach each successive image-only page. `page_for_image`
and `seek_image` resolve an occurrence identity explicitly;
`reflow_with_images` retains the visible occurrence when reflowing an image-only
page. A persisted character offset alone cannot distinguish several images at
the same boundary; a controller needing exact restoration also keeps the
occurrence identity.

`article::image_anchors(html, canonical_plaintext)` supplies conservative source
alignment. It retains each raw decoded image source and a distinct occurrence
identity. Its anchor is `Option<u32>`: preceding DOM text must match the canonical
text monotonically through Unicode character maps. Optional captions can be
included or absent, but unresolved ambiguity or a failed required prefix yields
`None`. Callers omit such an image from inline layout instead of guessing its
position. Resource authorization and source-to-asset matching remain the source
adapter and asset store's responsibility.

Remote tests cover mid-line and leading/trailing images, multiple images,
very tall geometry, image-only navigation and reflow, stable text-only layout,
Unicode and CRLF boundaries without character loss, text/image non-overlap,
raw source references, repeated text, optional captions, and conservative
alignment failure. These controlled fixtures do not establish live upstream
resource availability or physical e-ink rendering quality.

## Reader gestures

`GestureRecognizer` uses logical coordinates and caller-supplied monotonic
milliseconds. Its default tap zones are 30% previous, 40% menu, and 30% next.
The mirrored setting swaps tap directions. In swipe-only mode all short taps
open the menu. A horizontal movement of at least 64 logical pixels, dominant
over its vertical movement, turns a page. Vertical motion does not turn a page.

A press held for at least 500 ms with no more than 16 logical pixels of motion
emits a selection intent. The UI/controller decides which text lies under the
point. `cancel` discards a touch interrupted by a route change or suspension.
These are deterministic gesture policies, not physical-touch acceptance results.

The current controller updates an active long-press selection at an 80 ms
cadence while the contact remains held. The live underline follows the moving
endpoint across lines and can contract or reverse with the drag. Visual
selection and a saved annotation are separate states: changing an underline
does not by itself establish that a note was persisted. Native handoff and
return preserve the editing and selection context rather than treating an
unfinished selection as a new navigation request.

## Reader presentation

The Scribe Reader defaults to the body view with `notes_panel=false`. Opening
the optional notes panel is an explicit action; its width remains part of the
pagination and anchor-preserving reflow contract above.

Software dark mode is applied at the application's RGB-to-grayscale rendering
boundary. It does not enable hardware framebuffer inversion, which remains
unsupported. The native firmware UI owns its own presentation during a
handoff. Current software dark rendering still requires acceptance in the
complete physical-device flow.

## Managed sleep and native handoff

Managed sleep uses a saved local Reader checkpoint and a new application
process on wake. The checkpoint identifies the account context, book, chapter,
and source character anchor, with content checks before the anchor is reused.
It is a continuation of the local reading session, not a new cloud-position
choice or a claim that arbitrary replaced content can use the same offset.

A successful managed shutdown saves application state and acknowledges the
current private launcher generation. The supervisor waits for application
termination, releases the application's display and input ownership, and
returns control to the stock firmware UI. A supported wake and unlock path may
then start a fresh process and restore a valid local checkpoint. An explicit
application exit takes priority over an overlapping sleep request; a failed
save must not reopen an older checkpoint as if it were the current session.
USB takeover ends the launch and is a separate acceptance gate.

Wi-Fi settings and quick settings use a managed native handoff with an explicit
return context and editing/selection protection. The application releases its
input ownership before native interaction. Returning requires supported native
lifecycle evidence; guessed `closed` or `dismiss` behavior is not a return
signal. Opening failure or an interface that never becomes mapped before the
opening timeout retains native ownership and exits without restarting the
application. A normal mapped-to-unmapped close restores the application through
the managed return path after the required context checks.

While a target panel or one of its derived windows remains visible, the native
observer uses X window observations only. It reads `activeApp` and `activeView`
after all target and derived windows have closed, when those values can inform
the return-context check. This avoids repeated LIPC queries while the panel is
still visible without weakening the native-ownership boundary.

The full application-to-native-to-application interaction, software dark mode,
the latest continuous selection, and wake restoration to the same reading
position remain pending physical acceptance together.

## Offline pinyin

The dictionary contains all 65,105 AOSP PinyinIME source character readings and
phrases, normalized into `assets/pinyin/dictionary.tsv`. Its precise upstream
revision, data transformation, SHA-256 values, and Apache-2.0 notice are recorded
in [the dictionary readme](../assets/pinyin/README.md). The dictionary's license
does not select this application's project license.

`PinyinInput` keeps committed `query` separate from ASCII `composition`.
Candidates use historical source frequency, exact words, a bounded beam of
word combinations for continuous pinyin, incomplete-syllable completion, and
leading-word choices that preserve an unconverted suffix. Lowercase `v`
represents umlaut-u. Five candidates are exposed per page, with explicit
previous and next candidate intents. Unknown spelling remains editable and is
retained on submission rather than silently discarded.

The engine supports Chinese, English, numeric/symbol, uppercase, backspace,
space-to-choose, punctuation, and submit keys. Backspace removes composition
before committed text. A direct `set_query` supports physical keyboard or
desktop-native input without forcing it through pinyin conversion.

The bundled source is a historical unigram dictionary, not an adaptive language
model. It has no learned personal vocabulary, cloud prediction, or modern book
title corpus. Ambiguous words may require candidate paging or component entry;
all such operations remain available offline.

## Verification

The reader and input modules were checked on the designated `test-env` Linux
host in an isolated Rust crate with owned_ttf_parser. Coverage includes character-range
continuity, Unicode and CRLF preservation, punctuation boundaries, combining
marks, empty and tiny geometry, anchor-preserving reflow, touch thresholds,
dictionary substance, continuous-pinyin suffix retention, backspace ordering,
English and symbol modes, candidate paging, and non-destructive submission.

The optional font integration test accepts `WEREAD_TEST_FONT` and
`WEREAD_TEST_FONT_INDEX`, allowing an explicitly identified real font to drive
measurement. Without that environment variable it exercises no external font.
These results do not establish physical Kindle display quality, pressure or
pinch input, framebuffer refresh, sleep behavior, or live WeRead content access.

Current remote test totals, timings, and report paths are maintained in
[the verification record](VERIFICATION.md). The desktop UI and Clipboard checks used binary
`e11d74c23e309f2aa4e1daa6c4e33fe3d611f747d7e5c904a40bd2537eb24b4c`.
That binary predates the latest checkpoint regression fix; Reader source was
unchanged by that fix. These results remain distinct from complete
physical-device acceptance of the current iteration.

The latest ARM candidate is
`2b39140b11d3ac717953a01e38fc00d32434d30d18e3c7f9221e01d121f23609`.
Remote `file` and `readelf` inspection identified a stripped, statically linked
ELF32 ARM EABI5 executable with no dynamic section. This binary and the latest
launcher shell have run in the Scribe 2022 firmware 5.20.1 guest for the
specific cases recorded below. The candidate has not been deployed or run on
the physical Kindle. Static artifact inspection and emulator results do not
establish physical-device behavior or compatibility.

The current launcher source SHA-256 is
`0226f613b834862c229bf72e987ef441a2f523c84081d6cab46ee5bc39465ed4`.
Its isolated fixture coverage includes normal exit, sleep, USB, native-query
timing, and generation regressions. Opening
failure or an interface that never becomes mapped before the opening timeout
preserved native ownership and exited without restarting the application.
These controlled fixtures do not establish native interaction on a VM or
physical device; artifact details belong in [the verification record](VERIFICATION.md).

### Physical Scribe evidence

The identified physical device is a first-generation Scribe released in 2022,
running firmware 5.19.5. Checks used isolated demo data and application offline
mode. Binary
`61fa9afd1b5b914d82ee1e20c47cd962891c7e29a028695581ea21e82e3c5c90`
showed the Library, Reader page turns and font-size changes, and explicit exit.
A note was saved through the UI and confirmed in local persistence. Its earlier
sleep behavior exited with status 143 and restored the stock UI and touch;
this evidence does not establish the later checkpoint-based wake continuation.
The user explicitly skipped USB testing in this round.

The intermediate `9a13abe6...` candidate introduced the startup screen,
body-first Reader view, and sleep-recovery work. User feedback found its drag
underlining insufficient, so it is not acceptance of continuous drag selection.
The user separately confirmed that native Wi-Fi and quick-settings views could
be shown and closed using their independent LIPC switches. The complete managed
round trip from the application on the physical device remains an outstanding
gate.

### Firmware-emulator evidence

#### Earlier candidate

In an earlier Scribe 2022 firmware 5.20.1 VM run, binary
`daae495cbca99690e6a6bf745b2ba447cb1293dbfb859c0d2ce92a8a9903c3b3`
received QMP-held press-and-move input. The changed-region bounding-box width (px)
progressed from 49 to 341 to 827, reached 1,604, and contracted to 438 on reverse
movement. Cross-line growth was evidenced separately by an increase in
bounding-box height; the horizontal extent is not an underlined-pixel count or
total underline length. Light and dark Reader body text masks agreed at 1.0.
These are emulator observations of selection and rendering, separate from
physical touch and panel acceptance.

In that earlier native-interaction attempt, no QMP touch events were observed
in native XInput. Although quick settings mapped and the application released
input, `activeView` was empty after native navigation, so the return guard kept
the application from reclaiming ownership. That attempt did not pass. It is a
historical failed case, not evidence that native QMP input is generally
unsupported; the current candidate was exercised separately below.

#### Current 2b firmware-emulator run

Binary `2b39140b11d3ac717953a01e38fc00d32434d30d18e3c7f9221e01d121f23609`
and launcher source
`0226f613b834862c229bf72e987ef441a2f523c84081d6cab46ee5bc39465ed4`
ran in the Scribe 2022 firmware 5.20.1 guest. No XInput, input-grab, or driver
changes were used for this run. The dark startup screen was recorded in the
`startup-06` capture.

New held press-and-move captures for this binary measured changed-region
bounding-box widths of 49, 341, and 827 px, reaching 1,604 px in the cross-line
capture and contracting to 438 px on reverse movement. These measurements are
bounding-box widths, not underlined-pixel counts or total underline lengths.
Switching to light mode through the application UI produced a Reader body text
mask agreement of 1.0 against dark mode.

The Reader-to-native-Quick-to-Reader path completed through an actual QMP tap
on the mapped native panel's close control. The application advanced from
generation 1 to 2; the Reader body ROI pixel difference was 0, and the sampled
dark background was 0 before and after the return. An early close tap at about
five seconds had no effect, while the same-coordinate click worked after the
panel finished drawing. That early tap does not establish unsupported native
QMP input or an application input-restoration defect.

The sleep case used font-size setting 38, wide margins, and page 2 of 2 of the
`Lantingji Xu` sample. An actual powerd `powerButton` request with value 1 entered
`screenSaver`; the resume acknowledgement matched generation 2 and the saved
checkpoint contained `chapter_index=1` and Unicode anchor 150. A powerd `wakeUp`
request with value 1 produced native wake events and resumed generation 3. The
original Reader text ROI pixel difference was 0 after wake.

The Settings Wi-Fi row opened the native Wi-Fi list. A QMP tap on Done closed
the native view and returned to Settings in generation 4 from generation 3,
with a Settings content ROI pixel difference of 0. No network setting was
changed by this case.

Finally, Settings Exit and its confirmation ended the application, removed the
display lock, and returned to native `KPP_HOME` with `awesome` in state `S` and
`statusbar` running. The application log contained four native geometry lines
and no display-feedback error. No process exit status was recorded, so this
observation is not assigned a numerical exit code.

The Quick return, Wi-Fi return, and saved-anchor wake cases passed their stated
software-level observations. [The verification record](VERIFICATION.md) maintains
the reports, exit observation, acceptance summary, and capture locations.
USB was not exercised in this round. The current candidate still requires
integrated acceptance on the identified device, tracked in
[the physical testing record](PHYSICAL_TESTING.md).

The guest retains the VM-only startup error-handling change documented in
[the Scribe emulator environment record](SCRIBE_EMULATOR.md#current-environment-recovery-after-the-six-case-run).
The historical six native cases ran without that change and retain their
original environment attribution. The current VM observations do not revise
those earlier conditions or establish the physical Scribe's behavior.

## Article HTML flow

`src/article.rs` extracts an `ArticleDocument` from real article HTML. It selects
the WeChat content container when present, then visits DOM nodes once. Nested
`div` and `p` elements do not cause duplicate text extraction. Headings,
paragraphs, block quotes, explicit breaks, figures, captions, and images retain
their order. The application supplies the article title and account metadata
separately. Empty or unusable HTML can fall back to source-provided plain text.

Scripts, styles, hidden nodes, and embedded audio or video are omitted. The
document exposes a media count so the application can direct the reader to the
original article's phone QR entry. The parser does not execute content or fetch
resources. Image references accept HTTPS URLs, including protocol-relative
references normalized to HTTPS; data, file, script, credential-bearing, and
unencrypted HTTP URLs are not emitted.

`images()` returns distinct `ArticleImage { id, url, caption }` values, and
`image(id_or_url)` resolves a requested figure. IDs derive from the complete URL
so repeated references share the same local resource. `paginate` and
`paginate_with_first_page_height` return pages of `PageBlock` values ready for
the Slint article flow. The latter reserves the first-page title area while
retaining full capacity on later pages.

Article font settings are supplied independently as `ArticleLayout`, an alias
of the generic text `Layout`. Text lines are measured with `TextMetrics` and
wrapped through the same Unicode-safe paginator used for books. Heading sizing
and quote insets match the article component's rendering contract. Long quotes
can split between pages, with padding applied once per visible quote fragment.
Paragraph spacing is included in each block's logical height; the UI's flow
layout must therefore use zero additional spacing between these measured blocks.

`paginate_with_options` accepts explicit image and quote geometry. The application
uses PW placeholder/loaded totals of 208/248 logical pixels and Scribe totals of
254/334, including caption and gap. A frame moves past an insufficient title page
before it is resized; a regular page may fit the frame down while preserving the
caption budget. Nonfinal text lines carry measured letter spacing for justification,
and quote fragments use the supplied serif metrics with per-fragment padding.
`PageBlock.loaded` is true only when the caller's loaded set contains its image ID
or full URL. Parsing or pagination never marks an image loaded or downloads it.
The controller must fetch and decode the image, place the successful result in its
local cache, and request a full image-region refresh.

Article module checks on `test-env` cover nested extraction, text/image order,
lazy-image state, unsafe image references, hidden and executable nodes, content
container selection, malformed HTML fallback, heading and quote styles, and
first-page and continuation capacities. These are controlled HTML fixtures,
not evidence of a live WeChat account or physical Kindle behavior.
