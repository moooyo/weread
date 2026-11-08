//! Real App TextInput producers, clipboard snapshots, IME events and native Undo.
//! The event bridge is the pinned core's existing InternalKeyEvent/KeyEventType,
//! reexported solely because public WindowEvent cannot represent IME events.

use super::auth::{JUMP_EDIT_LIMITS, JumpEditLimits, jump_text_edit_admitted};
use super::underline_projection_tests::isolated;
use super::*;
use crate::input::{SEARCH_INPUT_SCALARS, THOUGHT_INPUT_SCALARS};
use slint::{
    InternalKeyEvent, KeyEventType,
    platform::{
        Clipboard, Key, Platform as SlintPlatform, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
    private_unstable_api::re_exports::{
        FocusReason, InputEventResult, InputType, ItemRc, KeyEvent, KeyEventResult, LogicalPoint,
        MouseCursor, MouseEvent, TextInput, WindowInner,
    },
};
use std::{
    cell::{Cell, RefMut},
    ops::{ControlFlow, Range},
    pin::Pin,
};

const BOOK: &str = "native-input-book-a";
const OTHER_BOOK: &str = "native-input-book-b";
const BODY: &str = "A\u{1f9ea}\u{4e2d}B native input fixture.";

slint::slint! {
    export component NativeGateWindow inherits Window {
        in-out property <string> value;
        in-out property <bool> gate-read-only: false;
        in-out property <bool> gate-enabled: true;
        callback edited(string);
        TextInput {
            width: parent.width;
            height: parent.height;
            text <=> root.value;
            read-only: root.gate-read-only;
            enabled: root.gate-enabled;
            single-line: true;
            font-family: "sans-serif";
            font-size: 18px;
            edited => { root.edited(self.text); }
        }
    }
}

#[derive(Default)]
struct ClipboardState {
    default: Option<String>,
    selection: Option<String>,
    after_default_read: Option<String>,
    after_default_write: Option<Box<dyn Fn()>>,
    default_reads: usize,
    selection_reads: usize,
}

struct TestUiPlatform(Rc<RefCell<ClipboardState>>);

impl SlintPlatform for TestUiPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }

    fn clipboard_text(&self, clipboard: Clipboard) -> Option<String> {
        let mut state = self.0.borrow_mut();
        match clipboard {
            Clipboard::DefaultClipboard => {
                state.default_reads += 1;
                let snapshot = state.default.clone();
                if let Some(next) = state.after_default_read.take() {
                    state.default = Some(next);
                }
                snapshot
            }
            Clipboard::SelectionClipboard => {
                state.selection_reads += 1;
                state.selection.clone()
            }
            _ => None,
        }
    }

    fn set_clipboard_text(&self, text: &str, clipboard: Clipboard) {
        let callback = {
            let mut state = self.0.borrow_mut();
            match clipboard {
                Clipboard::DefaultClipboard => {
                    state.default = Some(text.into());
                    state.after_default_write.take()
                }
                Clipboard::SelectionClipboard => {
                    state.selection = Some(text.into());
                    None
                }
                _ => None,
            }
        };
        if let Some(callback) = callback {
            callback();
        }
    }
}

struct Fixture {
    application: Option<Rc<RefCell<App>>>,
    ui: Option<AppWindow>,
    runtime: Option<tokio::runtime::Runtime>,
    clipboard: Rc<RefCell<ClipboardState>>,
    edits: Rc<Cell<usize>>,
    _directory: tempfile::TempDir,
}

impl Fixture {
    fn new(query: &str) -> Result<Self> {
        let clipboard = Rc::new(RefCell::new(ClipboardState::default()));
        slint::platform::set_platform(Box::new(TestUiPlatform(clipboard.clone())))?;
        let directory = tempfile::tempdir()?;
        let profile = DeviceProfile::Paperwhite6;
        let platform = Arc::new(Platform::new(profile, false)?);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let ui = AppWindow::new()?;
        crate::platform::configure_window(ui.window(), &platform)?;
        let application = App::attach(
            &ui,
            AppOptions {
                data_dir: directory.path().to_path_buf(),
                profile,
                demo: true,
                offline: true,
                initial_screen: "library".into(),
                fonts: vec![FontRecord {
                    id: "song".into(),
                    label: "Native edit fixture font".into(),
                    family: "sans-serif".into(),
                    metrics: TextMetrics::default(),
                    weights: BTreeMap::new(),
                    available: true,
                }],
            },
            platform,
            &runtime,
        )?;
        let edits = Rc::new(Cell::new(0));
        // Count only signals emitted by the actual generated native editor.
        // Each observer still dispatches through the real application callback.
        macro_rules! observe {
            ($global:ty, $section:literal, $edit:literal) => {
                let weak = Rc::downgrade(&application);
                let observed = edits.clone();
                ui.global::<$global>().on_event(move |action, value| {
                    if action.as_str() == $edit {
                        observed.set(observed.get() + 1);
                    }
                    if let Some(application) = weak.upgrade() {
                        application
                            .borrow_mut()
                            .dispatch($section, action.as_str(), value.as_str())
                            .expect("The native editor's real App callback failed");
                    }
                });
            };
        }
        observe!(SearchModel, "search", "edit-query");
        observe!(ReaderModel, "reader", "reader.thought-draft");
        observe!(LoginModel, "login", "edit-key");
        observe!(ShellModel, "shell", "dialog-input");
        {
            let mut state = application.borrow_mut();
            for book_id in [BOOK, OTHER_BOOK] {
                let mut book = Book::new(book_id, "Native edit fixture book");
                book.source = "local".into();
                book.on_shelf = true;
                book.total_chapters = 1;
                book.cached_chapters = 1;
                state.store.upsert_book(&book)?;
                state.store.upsert_chapter(&Chapter {
                    id: format!("{book_id}-chapter"),
                    book_id: book_id.into(),
                    index: 0,
                    title: "Native edit fixture chapter".into(),
                    cached: true,
                    body: Some(BODY.into()),
                    content_status: ContentStatus::Full,
                    ..Default::default()
                })?;
            }
            state.load_local_reader(BOOK, 0, None)?;
            state.query = PinyinInput::new(query);
            state.search_purpose = "books".into();
            state.search_scope = "shelf".into();
            state.navigate(Route::Search, true)?;
            state.render()?;
            anyhow::ensure!(
                state.demo && state.force_offline && !state.jobs.configured(),
                "The native edit fixture became connected"
            );
        }
        ui.show()?;
        Ok(Self {
            application: Some(application),
            ui: Some(ui),
            runtime: Some(runtime),
            clipboard,
            edits,
            _directory: directory,
        })
    }

    fn app(&self) -> RefMut<'_, App> {
        self.application.as_ref().unwrap().borrow_mut()
    }

    fn ui(&self) -> &AppWindow {
        self.ui.as_ref().unwrap()
    }

    fn paste_text(&self, text: &str) {
        self.clipboard.borrow_mut().default = Some(text.into());
    }

    fn selection_text(&self, text: &str) {
        self.clipboard.borrow_mut().selection = Some(text.into());
    }

    fn search_text(&self, text: &str) -> Result<()> {
        let mut state = self.app();
        assert!(state.query.set_query(text));
        state.render_search()
    }

    fn reinstall_admission(&self) {
        let weak = Rc::downgrade(self.application.as_ref().unwrap());
        self.ui()
            .window()
            .on_text_input_edit_admission(move |edit| {
                let Some(application) = weak.upgrade() else {
                    return false;
                };
                let Ok(state) = application.try_borrow() else {
                    return false;
                };
                state.native_text_input_admitted(edit)
            });
    }

    fn release_application(&mut self) -> Result<()> {
        if let Some(application) = self.application.take() {
            application.borrow_mut().terminal_shutdown()?;
            let weak = Rc::downgrade(&application);
            drop(application);
            assert!(
                weak.upgrade().is_none(),
                "A native edit handler retained App"
            );
        }
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        self.release_application()?;
        let ui = self.ui.take().unwrap();
        let weak = ui.as_weak();
        ui.hide()?;
        drop(ui);
        slint::platform::update_timers_and_animations();
        assert!(
            weak.upgrade().is_none(),
            "A native edit fixture retained its window"
        );
        self.runtime
            .take()
            .unwrap()
            .shutdown_timeout(Duration::from_secs(1));
        Ok(())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(application) = self.application.take() {
            let _ = application.borrow_mut().terminal_shutdown();
        }
        if let Some(ui) = self.ui.take() {
            let _ = ui.hide();
        }
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_timeout(Duration::from_secs(1));
        }
    }
}

#[derive(Clone, Copy)]
enum EditorKind {
    Search,
    Thought,
    Password,
    Numeric,
}

struct NativeEditor {
    item: ItemRc,
    adapter: Rc<dyn WindowAdapter>,
}

impl NativeEditor {
    fn find(window: &slint::Window, kind: EditorKind) -> Self {
        let inner = WindowInner::from_pub(window);
        let root = ItemRc::new_root(inner.component());
        let item = root
            .visit_descendants(|item| {
                let Some(input) = item.downcast::<TextInput>() else {
                    return ControlFlow::Continue(());
                };
                let input = input.as_pin_ref();
                let matches = !input.read_only()
                    && input.enabled()
                    && match kind {
                        EditorKind::Search => {
                            input.single_line() && input.input_type() == InputType::Text
                        }
                        EditorKind::Thought => {
                            !input.single_line() && input.input_type() == InputType::Text
                        }
                        EditorKind::Password => input.input_type() == InputType::Password,
                        EditorKind::Numeric => input.input_type() == InputType::Number,
                    };
                if matches && item.is_visible() {
                    ControlFlow::Break(item.clone())
                } else {
                    ControlFlow::Continue(())
                }
            })
            .expect("The actual AppWindow native editor is missing");
        let adapter = inner.window_adapter();
        inner.set_focus_item(&item, true, FocusReason::PointerClick);
        Self { item, adapter }
    }

    fn with<T>(&self, operation: impl FnOnce(Pin<&TextInput>) -> T) -> T {
        let input = self.item.downcast::<TextInput>().unwrap();
        operation(input.as_pin_ref())
    }

    fn text(&self) -> slint::SharedString {
        self.with(|input| input.text())
    }

    fn snapshot(
        &self,
    ) -> (
        slint::SharedString,
        slint::SharedString,
        i32,
        i32,
        Range<usize>,
    ) {
        self.with(|input| {
            (
                input.text(),
                input.preedit_text(),
                input.anchor_position_byte_offset(),
                input.cursor_position_byte_offset(),
                input.visual_representation(None).selection_range,
            )
        })
    }

    fn select(&self, range: Range<usize>) {
        self.with(|input| {
            input.set_selection_offsets(
                &self.adapter,
                &self.item,
                range.start as i32,
                range.end as i32,
            )
        });
    }

    fn select_end(&self) {
        let end = self.text().len();
        self.select(end..end);
    }

    fn paste(&self) {
        self.with(|input| input.paste(&self.adapter, &self.item));
    }

    fn cut(&self) {
        self.with(|input| input.cut(&self.adapter, &self.item));
    }

    fn delete_selection(&self) {
        self.with(|input| {
            input.delete_selection(
                &self.adapter,
                &self.item,
                slint::private_unstable_api::re_exports::TextChangeNotify::TriggerCallbacks,
            )
        });
    }

    fn undo(&self) {
        self.with(|input| input.undo(&self.adapter, &self.item));
    }

    fn redo(&self) {
        self.with(|input| input.redo(&self.adapter, &self.item));
    }

    fn key(&self, text: &str) {
        self.adapter
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.into() });
        self.adapter
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text: text.into() });
    }

    fn shortcut(&self, text: &str) {
        self.adapter
            .window()
            .dispatch_event(WindowEvent::KeyPressed {
                text: Key::Control.into(),
            });
        self.key(text);
        self.adapter
            .window()
            .dispatch_event(WindowEvent::KeyReleased {
                text: Key::Control.into(),
            });
    }

    fn key_code(&self, key: Key) {
        let text: slint::SharedString = key.into();
        self.key(text.as_str());
    }

    fn ime(&self, commit: &str, preedit: &str, replacement: Option<Range<i32>>) -> KeyEventResult {
        let mut key_event = KeyEvent::default();
        key_event.text = commit.into();
        WindowInner::from_pub(self.adapter.window()).process_key_input(InternalKeyEvent {
            key_event,
            event_type: if preedit.is_empty() {
                KeyEventType::CommitComposition
            } else {
                KeyEventType::UpdateComposition
            },
            replacement_range: replacement,
            preedit_text: preedit.into(),
            preedit_selection: (!preedit.is_empty()).then_some(0..preedit.len() as i32),
            ..Default::default()
        })
    }

    fn middle_release(&self) -> InputEventResult {
        let mut cursor = MouseCursor::Text;
        self.item.borrow().as_ref().input_event(
            &MouseEvent::Released {
                position: LogicalPoint::new(0., 0.),
                button: slint::platform::PointerEventButton::Middle,
                click_count: 0,
                touch_finger_id: 0,
            },
            &self.adapter,
            &self.item,
            &mut cursor,
        )
    }
}

fn accepted_suffix(fixture: &Fixture, editor: &NativeEditor) {
    editor.select_end();
    fixture.paste_text("B");
    editor.paste();
    assert_eq!(editor.text().as_str(), "AB");
    assert_eq!(fixture.app().query.query, "AB");
}

fn repeated_paste_rejection() -> Result<()> {
    let fixture = Fixture::new("A")?;
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Search);
    accepted_suffix(&fixture, &editor);
    editor.select(1..2);
    assert!(matches!(
        editor.ime("", "\u{4e2d}", None),
        KeyEventResult::EventAccepted
    ));
    let before = editor.snapshot();
    let (body, selection, account, request) = {
        let state = fixture.app();
        (
            state.reader.body.clone(),
            state.reader.selection,
            state.jobs.session(),
            state.search_request,
        )
    };
    let edits = fixture.edits.get();
    fixture.paste_text(&"x".repeat(SEARCH_INPUT_SCALARS + 1));
    for _ in 0..16 {
        editor.paste();
        assert_eq!(editor.snapshot(), before);
    }
    assert_eq!(fixture.edits.get(), edits);
    {
        let state = fixture.app();
        assert_eq!(state.query.query, "AB");
        assert_eq!(state.reader.body, body);
        assert_eq!(state.reader.selection, selection);
        assert_eq!(state.jobs.session(), account);
        assert_eq!(state.search_request, request);
    }
    assert!(matches!(
        editor.ime("", "", None),
        KeyEventResult::EventAccepted
    ));
    editor.undo();
    assert_eq!(
        editor.text().as_str(),
        "A",
        "A rejected payload entered native Undo"
    );
    editor.redo();
    assert_eq!(editor.text().as_str(), "AB");
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_rejected_native_pastes_preserve_composition_and_accepted_undo() -> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_rejected_native_pastes_preserve_composition_and_accepted_undo",
        repeated_paste_rejection,
    )
}

fn one_clipboard_snapshot() -> Result<()> {
    let fixture = Fixture::new("A")?;
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Search);
    editor.select_end();
    fixture.paste_text("B\n\u{1f9ea}");
    fixture.clipboard.borrow_mut().after_default_read = Some("x".repeat(SEARCH_INPUT_SCALARS + 1));
    editor.paste();
    assert_eq!(fixture.clipboard.borrow().default_reads, 1);
    assert_eq!(editor.text().as_str(), "AB \u{1f9ea}");
    assert_eq!(fixture.app().query.query, "AB \u{1f9ea}");
    assert_eq!(fixture.edits.get(), 1);
    assert_eq!(
        fixture.clipboard.borrow().default.as_ref().unwrap().len(),
        SEARCH_INPUT_SCALARS + 1
    );
    editor.undo();
    assert_eq!(editor.text().as_str(), "A");
    editor.redo();
    assert_eq!(editor.text().as_str(), "AB \u{1f9ea}");
    assert_eq!(fixture.clipboard.borrow().default_reads, 1);
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_native_paste_admits_and_inserts_the_same_single_clipboard_snapshot() -> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_native_paste_admits_and_inserts_the_same_single_clipboard_snapshot",
        one_clipboard_snapshot,
    )
}

fn key_and_control_paste() -> Result<()> {
    let prefix = "q".repeat(SEARCH_INPUT_SCALARS - 1);
    let fixture = Fixture::new(&prefix)?;
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Search);
    editor.select_end();
    editor.key("\u{1f9ea}");
    let complete = format!("{prefix}\u{1f9ea}");
    assert_eq!(editor.text().as_str(), complete);
    let before = editor.snapshot();
    let edits = fixture.edits.get();
    editor.key("X");
    fixture.paste_text("Y");
    editor.shortcut("v");
    assert_eq!(fixture.clipboard.borrow().default_reads, 1);
    assert_eq!(editor.snapshot(), before);
    assert_eq!(fixture.edits.get(), edits);
    editor.shortcut("z");
    assert_eq!(editor.text().as_str(), prefix);
    editor.redo();
    assert_eq!(editor.text().as_str(), complete);
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_native_unicode_key_and_control_paste_reject_before_undo() -> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_native_unicode_key_and_control_paste_reject_before_undo",
        key_and_control_paste,
    )
}

fn atomic_ime_admission() -> Result<()> {
    let prefix = "s".repeat(SEARCH_INPUT_SCALARS - 2);
    let fixture = Fixture::new(&prefix)?;
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Search);
    editor.select_end();
    fixture.paste_text("A");
    editor.paste();
    let complete = format!("{prefix}A");
    editor.select(prefix.len()..complete.len());
    assert!(matches!(
        editor.ime("", "\u{4e2d}", None),
        KeyEventResult::EventAccepted
    ));
    let before = editor.snapshot();
    let edits = fixture.edits.get();
    for _ in 0..4 {
        assert!(matches!(
            editor.ime("xy", "z", None),
            KeyEventResult::EventIgnored
        ));
        // Empty commit does not erase the selection. Preedit is added at the
        // post-edit cursor, so that selection must not be deducted again.
        assert!(matches!(
            editor.ime("", "\u{4e2d}\u{4e2d}", None),
            KeyEventResult::EventIgnored
        ));
        assert_eq!(editor.snapshot(), before);
    }
    assert_eq!(fixture.edits.get(), edits);
    assert_eq!(fixture.app().query.query, complete);
    assert!(matches!(
        editor.ime("", "", None),
        KeyEventResult::EventAccepted
    ));
    editor.undo();
    assert_eq!(editor.text().as_str(), prefix);
    editor.redo();
    assert_eq!(editor.text().as_str(), complete);
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_ime_commit_and_preedit_admit_atomically_without_double_counting_selection() -> Result<()>
{
    isolated(
        "app::native_edit_tests::genuine_ime_commit_and_preedit_admit_atomically_without_double_counting_selection",
        atomic_ime_admission,
    )
}

fn unicode_ime_replacement() -> Result<()> {
    let original = "A\u{1f9ea}\u{4e2d}B";
    let fixture = Fixture::new(original)?;
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Search);
    editor.select(1..8);
    assert!(matches!(
        editor.ime("\u{754c}", "", None),
        KeyEventResult::EventAccepted
    ));
    assert_eq!(editor.text().as_str(), "A\u{754c}B");
    editor.undo();
    assert_eq!(editor.text().as_str(), "AB");
    editor.undo();
    assert_eq!(editor.text().as_str(), original);
    editor.redo();
    editor.redo();
    assert_eq!(editor.text().as_str(), "A\u{754c}B");
    editor.select(4..4);
    assert!(matches!(
        editor.ime("", "", Some(-3..0)),
        KeyEventResult::EventAccepted
    ));
    assert_eq!(editor.text().as_str(), "AB");
    editor.undo();
    assert_eq!(editor.text().as_str(), "A\u{754c}B");
    assert_eq!(fixture.app().reader.body, BODY);
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_ime_unicode_replacement_and_empty_commit_deletion_keep_native_history() -> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_ime_unicode_replacement_and_empty_commit_deletion_keep_native_history",
        unicode_ime_replacement,
    )
}

fn note_and_book_scope() -> Result<()> {
    let fixture = Fixture::new("A")?;
    let search = NativeEditor::find(fixture.ui().window(), EditorKind::Search);
    accepted_suffix(&fixture, &search);
    let account = fixture.app().jobs.session();
    {
        let mut state = fixture.app();
        state.load_local_reader(BOOK, 0, None)?;
        state.reader.selection = Some((0, 1));
        assert!(
            state
                .reader
                .draft
                .set_query(&"n".repeat(SEARCH_INPUT_SCALARS + 1))
        );
        state.render()?;
    }
    let before = search.snapshot();
    let edits = fixture.edits.get();
    fixture.paste_text("obsolete");
    search.paste();
    search.undo();
    assert_eq!(search.snapshot(), before);
    assert_eq!(fixture.edits.get(), edits);
    fixture
        .ui()
        .global::<ReaderModel>()
        .invoke_event("reader.write-thought".into(), "selection".into());
    let thought = NativeEditor::find(fixture.ui().window(), EditorKind::Thought);
    thought.select_end();
    fixture.paste_text("\u{4e2d}");
    thought.paste();
    assert_eq!(
        fixture.app().query.query.chars().count(),
        SEARCH_INPUT_SCALARS + 2
    );
    fixture.ui().global::<SearchModel>().invoke_event(
        "edit-query".into(),
        "m".repeat(THOUGHT_INPUT_SCALARS).into(),
    );
    thought.select_end();
    let before = thought.snapshot();
    let edits = fixture.edits.get();
    fixture.paste_text("X");
    thought.paste();
    assert_eq!(thought.snapshot(), before);
    assert_eq!(fixture.edits.get(), edits);
    fixture
        .ui()
        .global::<SearchModel>()
        .invoke_event("back".into(), "".into());
    {
        let mut state = fixture.app();
        state.load_local_reader(OTHER_BOOK, 0, None)?;
        state.reader.selection = Some((0, 1));
        state.render()?;
        assert!(state.reader.draft.query.is_empty());
        assert_eq!(state.jobs.session(), account);
    }
    fixture
        .ui()
        .global::<ReaderModel>()
        .invoke_event("reader.write-thought".into(), "selection".into());
    let other = NativeEditor::find(fixture.ui().window(), EditorKind::Thought);
    other.select_end();
    fixture.paste_text("OK");
    other.paste();
    assert_eq!(fixture.app().query.query, "OK");
    assert_eq!(fixture.app().reader.book_id, OTHER_BOOK);
    assert_eq!(fixture.app().reader.body, BODY);
    other.undo();
    assert!(other.text().is_empty());
    drop(other);
    drop(thought);
    drop(search);
    fixture.finish()
}

#[test]
fn genuine_native_search_note_and_book_switches_apply_the_current_input_scope() -> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_native_search_note_and_book_switches_apply_the_current_input_scope",
        note_and_book_scope,
    )
}

fn numeric_dialog_scope() -> Result<()> {
    let query = "q".repeat(SEARCH_INPUT_SCALARS);
    let fixture = Fixture::new(&query)?;
    fixture
        .ui()
        .global::<SearchModel>()
        .invoke_event("jump".into(), "".into());
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Numeric);
    editor.select(0..editor.text().len());
    fixture.paste_text("42");
    editor.paste();
    assert_eq!(editor.text().as_str(), "42");
    let before = editor.snapshot();
    let edits = fixture.edits.get();
    editor.key("X");
    assert_eq!(editor.snapshot(), before);
    assert_eq!(fixture.edits.get(), edits);
    // Native paste has never applied Number's ordinary-key type filter.
    // Admission must preserve that distinction, not invent a new paste policy.
    editor.select(0..0);
    fixture.paste_text("z");
    editor.paste();
    assert_eq!(editor.text().as_str(), "z42");
    editor.undo();
    assert_eq!(editor.text().as_str(), "42");
    editor.redo();
    assert_eq!(editor.text().as_str(), "z42");
    fixture
        .ui()
        .global::<ShellModel>()
        .invoke_event("dialog-secondary".into(), "".into());
    let before = editor.snapshot();
    let edits = fixture.edits.get();
    fixture.paste_text("7");
    editor.paste();
    editor.undo();
    editor.redo();
    assert_eq!(editor.snapshot(), before);
    assert_eq!(fixture.edits.get(), edits);
    assert_eq!(fixture.app().query.query, query);
    assert!(fixture.app().dialog_action.is_empty());
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_native_numeric_dialog_preserves_its_existing_policy_and_rejects_after_close()
-> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_native_numeric_dialog_preserves_its_existing_policy_and_rejects_after_close",
        numeric_dialog_scope,
    )
}

fn password_scope() -> Result<()> {
    let fixture = Fixture::new("Search draft")?;
    {
        let mut state = fixture.app();
        state.navigate(Route::Login, true)?;
        state.render()?;
    }
    fixture
        .ui()
        .global::<LoginModel>()
        .invoke_event("step".into(), "key".into());
    fixture
        .ui()
        .global::<LoginModel>()
        .invoke_event("edit-key".into(), "A".into());
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Password);
    editor.select_end();
    fixture.paste_text("B");
    editor.paste();
    assert_eq!(fixture.app().login_key, "AB");
    let before = editor.snapshot();
    let edits = fixture.edits.get();
    fixture.paste_text(&"x".repeat(16_385));
    for _ in 0..4 {
        editor.paste();
        assert_eq!(editor.snapshot(), before);
    }
    assert_eq!(fixture.edits.get(), edits);
    editor.undo();
    assert_eq!(fixture.app().login_key, "A");
    editor.redo();
    assert_eq!(fixture.app().login_key, "AB");
    {
        let mut state = fixture.app();
        state.navigate(Route::Search, true)?;
        state.render()?;
    }
    let before = editor.snapshot();
    let edits = fixture.edits.get();
    fixture.paste_text("obsolete");
    editor.paste();
    editor.undo();
    assert_eq!(editor.snapshot(), before);
    assert_eq!(fixture.edits.get(), edits);
    assert_eq!(fixture.app().login_key, "AB");
    assert_eq!(fixture.app().query.query, "Search draft");
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_native_password_rejections_keep_valid_undo_and_current_login_scope() -> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_native_password_rejections_keep_valid_undo_and_current_login_scope",
        password_scope,
    )
}

fn disabled_and_readonly_gates() -> Result<()> {
    let fixture = Fixture::new("A")?;
    // The product Search input has constant read-only/enabled defaults. Bind
    // legal public gate properties in a native WindowItem, then associate that
    // root with the real AppWindow adapter and its existing weak App policy.
    let gates = NativeGateWindow::new()?;
    let platform = fixture.app().platform.clone();
    crate::platform::configure_window(gates.window(), &platform)?;
    gates.set_value("A".into());
    let weak_ui = fixture.ui().as_weak();
    gates.on_edited(move |text| {
        if let Some(ui) = weak_ui.upgrade() {
            ui.global::<SearchModel>()
                .invoke_event("edit-query".into(), text);
        }
    });
    let inner = WindowInner::from_pub(fixture.ui().window());
    let original = inner.component();
    let gated_root = WindowInner::from_pub(gates.window()).component();
    inner.set_component(&gated_root);
    fixture.reinstall_admission();
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Search);
    accepted_suffix(&fixture, &editor);
    fixture.paste_text("X");
    let before = editor.snapshot();
    let edits = fixture.edits.get();
    gates.set_gate_read_only(true);
    assert!(editor.with(|input| input.read_only()));
    editor.paste();
    editor.undo();
    editor.redo();
    assert_eq!(editor.snapshot(), before);
    gates.set_gate_read_only(false);
    gates.set_gate_enabled(false);
    assert!(!editor.with(|input| input.enabled()));
    editor.paste();
    editor.undo();
    editor.redo();
    assert_eq!(editor.snapshot(), before);
    assert_eq!(fixture.edits.get(), edits);
    assert_eq!(fixture.app().query.query, "AB");
    gates.set_gate_enabled(true);
    assert!(!editor.with(|input| input.read_only()));
    assert!(editor.with(|input| input.enabled()));
    editor.undo();
    assert_eq!(editor.text().as_str(), "A");
    editor.redo();
    assert_eq!(editor.text().as_str(), "AB");
    drop(editor);
    inner.set_component(&original);
    fixture.reinstall_admission();
    drop(gated_root);
    drop(original);
    gates.hide()?;
    drop(gates);
    fixture.finish()
}

#[test]
fn genuine_native_disabled_and_readonly_producers_preserve_accepted_history() -> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_native_disabled_and_readonly_producers_preserve_accepted_history",
        disabled_and_readonly_gates,
    )
}

fn historical_note_gate() -> Result<()> {
    let fixture = Fixture::new("Search draft")?;
    {
        let mut state = fixture.app();
        state.load_local_reader(BOOK, 0, None)?;
        state.reader.selection = Some((0, 1));
        assert!(state.reader.draft.set_query("draft"));
        state.render()?;
    }
    fixture
        .ui()
        .global::<ReaderModel>()
        .invoke_event("reader.write-thought".into(), "selection".into());
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Thought);
    editor.select_end();
    fixture.paste_text("B");
    editor.paste();
    assert_eq!(fixture.app().query.query, "draftB");
    fixture
        .ui()
        .global::<SearchModel>()
        .invoke_event("back".into(), "".into());
    let historical = "h".repeat(THOUGHT_INPUT_SCALARS + 1);
    fixture.app().store.save_annotation(&Annotation {
        id: "native-history-note".into(),
        book_id: BOOK.into(),
        chapter_index: 0,
        start: 0,
        end: 1,
        quote: "A".into(),
        note: historical.clone(),
        created_at: 1,
    })?;
    fixture
        .ui()
        .global::<ReaderModel>()
        .invoke_event("reader.thought".into(), "mine:native-history-note".into());
    assert!(fixture.app().reader.thought_edit_blocked);
    let before = editor.snapshot();
    let draft = fixture.app().reader.draft.query.clone();
    let edits = fixture.edits.get();
    fixture.paste_text("obsolete");
    editor.paste();
    editor.undo();
    editor.redo();
    assert_eq!(editor.snapshot(), before);
    assert_eq!(fixture.edits.get(), edits);
    assert_eq!(fixture.app().reader.draft.query, draft);
    fixture
        .ui()
        .global::<ReaderModel>()
        .invoke_event("reader.thought-close".into(), "".into());
    fixture
        .ui()
        .global::<ReaderModel>()
        .invoke_event("reader.press".into(), "0.2,8".into());
    let generation = fixture.ui().global::<ReaderModel>().get_press_generation();
    fixture
        .ui()
        .global::<ReaderModel>()
        .invoke_event("reader.hold".into(), generation.to_string().into());
    fixture
        .ui()
        .global::<ReaderModel>()
        .invoke_event("reader.release".into(), "0.2,8".into());
    fixture
        .ui()
        .global::<ReaderModel>()
        .invoke_event("reader.write-thought".into(), "selection".into());
    assert!(!fixture.app().reader.thought_edit_blocked);
    assert!(fixture.app().reader.thought_id.is_empty());
    assert!(fixture.app().reader.draft.query.is_empty());
    let current = NativeEditor::find(fixture.ui().window(), EditorKind::Thought);
    current.select_end();
    fixture.paste_text("OK");
    current.paste();
    assert_eq!(fixture.app().query.query, "OK");
    let saved = fixture.app().store.annotations(BOOK)?;
    assert_eq!(
        saved
            .iter()
            .find(|note| note.id == "native-history-note")
            .unwrap()
            .note,
        historical
    );
    current.undo();
    assert!(current.text().is_empty());
    drop(current);
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_native_historical_readonly_note_rejects_stale_editor_and_restores_new_selection()
-> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_native_historical_readonly_note_rejects_stale_editor_and_restores_new_selection",
        historical_note_gate,
    )
}

fn lifecycle_and_borrow_gates() -> Result<()> {
    let fixture = Fixture::new("A")?;
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Search);
    accepted_suffix(&fixture, &editor);
    fixture.paste_text("X");
    let before = editor.snapshot();
    let edits = fixture.edits.get();
    {
        let _borrow = fixture.app();
        editor.paste();
        editor.undo();
        editor.redo();
    }
    assert_eq!(editor.snapshot(), before);
    fixture.app().managed_sleep_blocked = true;
    editor.paste();
    editor.undo();
    fixture.app().managed_sleep_blocked = false;
    fixture.app().closing.set(true);
    editor.paste();
    editor.undo();
    fixture.app().closing.set(false);
    fixture.app().immediate_job_error = Some(anyhow::anyhow!("Synthetic native edit gate"));
    editor.paste();
    editor.undo();
    fixture.app().immediate_job_error = None;
    assert_eq!(editor.snapshot(), before);
    assert_eq!(fixture.edits.get(), edits);
    editor.undo();
    assert_eq!(
        editor.text().as_str(),
        "A",
        "A denied Undo advanced the history"
    );
    editor.redo();
    assert_eq!(editor.text().as_str(), "AB");
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_native_borrow_sleep_close_and_error_rejections_do_not_advance_history() -> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_native_borrow_sleep_close_and_error_rejections_do_not_advance_history",
        lifecycle_and_borrow_gates,
    )
}

fn expired_weak_policy() -> Result<()> {
    let mut fixture = Fixture::new("A")?;
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Search);
    accepted_suffix(&fixture, &editor);
    fixture.release_application()?;
    fixture.paste_text("X");
    let before = editor.snapshot();
    let edits = fixture.edits.get();
    for _ in 0..4 {
        editor.paste();
        editor.undo();
        editor.redo();
    }
    assert_eq!(editor.snapshot(), before);
    assert_eq!(fixture.edits.get(), edits);
    // Explicit registration is a caller action; an expired weak predicate must
    // never clear itself and silently change the policy to default-allow.
    fixture.ui().window().on_text_input_edit_admission(|_| true);
    editor.undo();
    assert_eq!(editor.text().as_str(), "A");
    editor.redo();
    assert_eq!(editor.text().as_str(), "AB");
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_native_expired_weak_admission_stays_closed_and_retains_accepted_history() -> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_native_expired_weak_admission_stays_closed_and_retains_accepted_history",
        expired_weak_policy,
    )
}

fn recursive_clear_policy() -> Result<()> {
    let fixture = Fixture::new("A")?;
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Search);
    editor.select_end();
    fixture.paste_text("B");
    fixture.clipboard.borrow_mut().after_default_read = Some("x".repeat(SEARCH_INPUT_SCALARS + 1));
    let calls = Rc::new(Cell::new(0));
    let observed = calls.clone();
    let weak_app = Rc::downgrade(fixture.application.as_ref().unwrap());
    let weak_ui = fixture.ui().as_weak();
    let weak_item = editor.item.downgrade();
    fixture
        .ui()
        .window()
        .on_text_input_edit_admission(move |edit| {
            observed.set(observed.get() + 1);
            let Some(application) = weak_app.upgrade() else {
                return false;
            };
            let admitted = application
                .try_borrow()
                .is_ok_and(|state| state.native_text_input_admitted(edit));
            let Some(ui) = weak_ui.upgrade() else {
                return false;
            };
            ui.window().clear_text_input_edit_admission();
            if let Some(item) = weak_item.upgrade() {
                let input = item.downcast::<TextInput>().unwrap();
                let adapter = WindowInner::from_pub(ui.window()).window_adapter();
                input.as_pin_ref().paste(&adapter, &item);
            }
            admitted
        });
    editor.paste();
    assert_eq!(calls.get(), 1);
    assert_eq!(
        fixture.clipboard.borrow().default_reads,
        1,
        "Recursive paste fetched a second snapshot"
    );
    assert_eq!(editor.text().as_str(), "AB");
    assert_eq!(fixture.edits.get(), 1);
    fixture.reinstall_admission();
    editor.paste();
    assert_eq!(editor.text().as_str(), "AB");
    assert_eq!(fixture.edits.get(), 1);
    editor.undo();
    assert_eq!(editor.text().as_str(), "A");
    editor.redo();
    assert_eq!(editor.text().as_str(), "AB");
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_native_recursive_paste_stays_denied_when_the_active_callback_clears_itself() -> Result<()>
{
    isolated(
        "app::native_edit_tests::genuine_native_recursive_paste_stays_denied_when_the_active_callback_clears_itself",
        recursive_clear_policy,
    )
}

fn middle_callback_text_change() -> Result<()> {
    let fixture = Fixture::new("A")?;
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Search);
    accepted_suffix(&fixture, &editor);
    fixture.search_text("")?;
    editor.select_end();
    assert!(matches!(
        editor.ime("", "\u{4e2d}", None),
        KeyEventResult::EventAccepted
    ));
    let callbacks = Rc::new(Cell::new(0));
    let observed = callbacks.clone();
    let weak_item = editor.item.downgrade();
    editor.with(|input| {
        input.cursor_position_changed.set_handler(move |_| {
            observed.set(observed.get() + 1);
            if let Some(item) = weak_item.upgrade() {
                let native = item.downcast::<TextInput>().unwrap();
                let native = native.as_pin_ref();
                native.text.set("zz".into());
                native.anchor_position_byte_offset.set(2);
                native.cursor_position_byte_offset.set(2);
            }
        })
    });
    fixture.selection_text(&"x".repeat(SEARCH_INPUT_SCALARS - 1));
    let edits = fixture.edits.get();
    editor.middle_release();
    assert_eq!(callbacks.get(), 1);
    assert_eq!(fixture.clipboard.borrow().selection_reads, 1);
    assert_eq!(
        editor.text().as_str(),
        "zz",
        "Old admission overwrote callback-owned text"
    );
    assert_eq!(
        editor.with(|input| input.preedit_text()).as_str(),
        "\u{4e2d}"
    );
    assert_eq!(fixture.edits.get(), edits);
    assert_eq!(fixture.app().query.query, "");
    editor.with(|input| input.cursor_position_changed.set_handler(|_| {}));
    fixture.search_text("AB")?;
    editor.select_end();
    assert!(matches!(
        editor.ime("", "", None),
        KeyEventResult::EventAccepted
    ));
    editor.undo();
    assert_eq!(
        editor.text().as_str(),
        "A",
        "The callback gap retained a rejected Undo payload"
    );
    editor.redo();
    assert_eq!(editor.text().as_str(), "AB");
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_middle_click_rechecks_callback_owned_text_before_preedit_or_undo_mutation() -> Result<()>
{
    isolated(
        "app::native_edit_tests::genuine_middle_click_rechecks_callback_owned_text_before_preedit_or_undo_mutation",
        middle_callback_text_change,
    )
}

fn middle_callback_selection_change() -> Result<()> {
    let fixture = Fixture::new("A")?;
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Search);
    accepted_suffix(&fixture, &editor);
    let weak_item = editor.item.downgrade();
    editor.with(|input| {
        input.cursor_position_changed.set_handler(move |_| {
            if let Some(item) = weak_item.upgrade() {
                let native = item.downcast::<TextInput>().unwrap();
                native.as_pin_ref().anchor_position_byte_offset.set(0);
                native.as_pin_ref().cursor_position_byte_offset.set(1);
            }
        })
    });
    fixture.selection_text("X");
    let edits = fixture.edits.get();
    editor.middle_release();
    assert_eq!(editor.text().as_str(), "AB");
    assert_eq!(
        editor.with(|input| input.selection_anchor_and_cursor()),
        (0, 1)
    );
    assert_eq!(fixture.edits.get(), edits);
    editor.with(|input| input.cursor_position_changed.set_handler(|_| {}));
    editor.undo();
    assert_eq!(editor.text().as_str(), "A");
    editor.redo();
    assert_eq!(editor.text().as_str(), "AB");
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_middle_click_rechecks_callback_selection_before_spending_admission() -> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_middle_click_rechecks_callback_selection_before_spending_admission",
        middle_callback_selection_change,
    )
}

fn middle_callback_root_change() -> Result<()> {
    let fixture = Fixture::new("")?;
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Search);
    let original = WindowInner::from_pub(fixture.ui().window()).component();
    let replacement = AppWindow::new()?;
    let platform = fixture.app().platform.clone();
    crate::platform::configure_window(replacement.window(), &platform)?;
    replacement
        .global::<ShellModel>()
        .set_route("search".into());
    replacement.global::<SearchModel>().set_query("NEW".into());
    let replacement_root =
        ItemRc::new_root(WindowInner::from_pub(replacement.window()).component());
    let replacement_weak = replacement_root.downgrade();
    let weak_ui = fixture.ui().as_weak();
    editor.with(|input| {
        input.cursor_position_changed.set_handler(move |_| {
            if let (Some(ui), Some(root)) = (weak_ui.upgrade(), replacement_weak.upgrade()) {
                WindowInner::from_pub(ui.window()).set_component(root.item_tree());
            }
        })
    });
    fixture.selection_text("abc");
    let edits = fixture.edits.get();
    editor.middle_release();
    assert_eq!(editor.text().as_str(), "");
    assert_eq!(
        replacement.global::<SearchModel>().get_query().as_str(),
        "NEW"
    );
    assert_eq!(fixture.edits.get(), edits);
    editor.with(|input| input.cursor_position_changed.set_handler(|_| {}));
    let current = NativeEditor::find(fixture.ui().window(), EditorKind::Search);
    current.select_end();
    fixture.paste_text("Q");
    current.paste();
    assert_eq!(
        current.text().as_str(),
        "NEW",
        "Root replacement silently defaulted to allow"
    );
    fixture.reinstall_admission();
    current.paste();
    assert_eq!(current.text().as_str(), "NEWQ");
    current.undo();
    assert_eq!(current.text().as_str(), "NEW");
    drop(current);
    WindowInner::from_pub(fixture.ui().window()).set_component(&original);
    fixture.reinstall_admission();
    drop(original);
    drop(replacement_root);
    replacement.hide()?;
    drop(replacement);
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_middle_click_root_replacement_preserves_new_context_and_requires_registration()
-> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_middle_click_root_replacement_preserves_new_context_and_requires_registration",
        middle_callback_root_change,
    )
}

fn middle_without_handler() -> Result<()> {
    let fixture = Fixture::new("")?;
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Search);
    fixture.ui().window().clear_text_input_edit_admission();
    let first = Rc::new(Cell::new(true));
    let once = first.clone();
    let weak_item = editor.item.downgrade();
    editor.with(|input| {
        input.cursor_position_changed.set_handler(move |_| {
            if !once.replace(false) {
                return;
            }
            if let Some(item) = weak_item.upgrade() {
                let native = item.downcast::<TextInput>().unwrap();
                let native = native.as_pin_ref();
                native.text.set("zz".into());
                native.anchor_position_byte_offset.set(2);
                native.cursor_position_byte_offset.set(2);
            }
        })
    });
    fixture.selection_text("abc");
    editor.middle_release();
    assert!(!first.get());
    assert_eq!(editor.text().as_str(), "zzabc");
    assert_eq!(fixture.app().query.query, "zzabc");
    editor.with(|input| input.cursor_position_changed.set_handler(|_| {}));
    editor.undo();
    assert_eq!(editor.text().as_str(), "zz");
    editor.redo();
    assert_eq!(editor.text().as_str(), "zzabc");
    fixture.reinstall_admission();
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_native_no_handler_middle_click_retains_upstream_callback_order_and_undo() -> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_native_no_handler_middle_click_retains_upstream_callback_order_and_undo",
        middle_without_handler,
    )
}

fn install_jump_limits(fixture: &Fixture, limits: JumpEditLimits) {
    let weak = Rc::downgrade(fixture.application.as_ref().unwrap());
    fixture
        .ui()
        .window()
        .on_text_input_edit_admission(move |edit| {
            let Some(application) = weak.upgrade() else {
                return false;
            };
            let Ok(state) = application.try_borrow() else {
                return false;
            };
            state.native_text_input_admitted(edit)
                && (!edit.is_numeric() || jump_text_edit_admitted(edit, limits))
        });
}

fn jump_low_limits() -> Result<()> {
    let fixture = Fixture::new("Jump draft")?;
    fixture
        .ui()
        .global::<SearchModel>()
        .invoke_event("jump".into(), "".into());
    install_jump_limits(
        &fixture,
        JumpEditLimits {
            scalars: 4,
            bytes: 8,
            work_bytes: 8,
        },
    );
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Numeric);
    editor.select(0..editor.text().len());
    fixture.paste_text("z42");
    editor.paste();
    assert_eq!(editor.text().as_str(), "z42");
    assert_eq!(
        fixture
            .ui()
            .global::<ShellModel>()
            .get_dialog_input()
            .as_str(),
        "z42"
    );
    editor.select_end();
    assert!(matches!(
        editor.ime("", "1", None),
        KeyEventResult::EventAccepted
    ));
    let before = editor.snapshot();
    let edits = fixture.edits.get();
    fixture.paste_text("XX");
    editor.paste();
    assert!(matches!(
        editor.ime("2", "3", None),
        KeyEventResult::EventIgnored
    ));
    assert_eq!(editor.snapshot(), before);
    assert_eq!(fixture.edits.get(), edits);
    assert!(matches!(
        editor.ime("", "", None),
        KeyEventResult::EventAccepted
    ));

    fixture.reinstall_admission();
    editor.select(0..editor.text().len());
    fixture.paste_text("\u{1f9ea}\u{1f9ea}");
    editor.paste();
    assert_eq!(editor.text().as_str(), "\u{1f9ea}\u{1f9ea}");
    install_jump_limits(
        &fixture,
        JumpEditLimits {
            scalars: 4,
            bytes: 8,
            work_bytes: 8,
        },
    );
    editor.select(0..8);
    let before = editor.snapshot();
    let edits = fixture.edits.get();
    // Both replacements fit the final scalar/byte limits. Their removed old
    // text still pays work before native selection, composition or Undo copies.
    fixture.paste_text("ab");
    editor.paste();
    assert!(matches!(
        editor.ime("", "1", Some(-8..0)),
        KeyEventResult::EventIgnored
    ));
    assert_eq!(editor.snapshot(), before);
    assert_eq!(fixture.edits.get(), edits);
    fixture.reinstall_admission();
    editor.undo();
    assert!(editor.text().is_empty());
    editor.undo();
    assert_eq!(editor.text().as_str(), "z42");

    editor.select(0..3);
    fixture
        .ui()
        .global::<ShellModel>()
        .invoke_event("dialog-secondary".into(), "".into());
    let before = editor.snapshot();
    let edits = fixture.edits.get();
    fixture.paste_text("sentinel");
    editor.cut();
    editor.delete_selection();
    assert_eq!(editor.snapshot(), before);
    assert_eq!(fixture.edits.get(), edits);
    assert_eq!(
        fixture.clipboard.borrow().default.as_deref(),
        Some("sentinel")
    );
    assert_eq!(fixture.app().query.query, "Jump draft");
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_jump_low_quota_preserves_paste_ime_undo_and_closed_dialog_context() -> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_jump_low_quota_preserves_paste_ime_undo_and_closed_dialog_context",
        jump_low_limits,
    )
}

fn jump_production_limits() -> Result<()> {
    let fixture = Fixture::new("Jump draft")?;
    fixture
        .ui()
        .global::<SearchModel>()
        .invoke_event("jump".into(), "".into());
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Numeric);
    let leading = format!("{}1", "0".repeat(190));
    assert!(leading.len() > 128 && leading.len() < JUMP_EDIT_LIMITS.scalars);
    editor.select(0..editor.text().len());
    fixture.paste_text(&leading);
    editor.paste();
    assert_eq!(editor.text().as_str(), leading);
    assert_eq!(editor.text().parse::<usize>()?, 1);

    let full = "1".repeat(JUMP_EDIT_LIMITS.scalars);
    editor.select(0..editor.text().len());
    fixture.paste_text(&full);
    editor.paste();
    assert_eq!(editor.text().as_str(), full);
    let before = editor.snapshot();
    let edits = fixture.edits.get();
    fixture.paste_text("2");
    editor.paste();
    assert_eq!(editor.snapshot(), before);
    assert_eq!(fixture.edits.get(), edits);

    let byte_full = "\u{1f9ea}".repeat(JUMP_EDIT_LIMITS.bytes / 4);
    editor.select(0..editor.text().len());
    fixture.paste_text(&byte_full);
    editor.paste();
    assert_eq!(editor.text().as_str(), byte_full);
    let before = editor.snapshot();
    let edits = fixture.edits.get();
    fixture.paste_text("\u{1f9ea}");
    editor.paste();
    assert_eq!(editor.snapshot(), before);
    assert_eq!(fixture.edits.get(), edits);
    editor.undo();
    assert!(
        editor.text().is_empty(),
        "Rejected jump bytes entered native Undo"
    );
    editor.undo();
    assert_eq!(editor.text().as_str(), full);

    editor.select(0..editor.text().len());
    fixture.paste_text(&leading);
    editor.paste();
    assert_eq!(
        fixture
            .ui()
            .global::<ShellModel>()
            .get_dialog_input()
            .as_str(),
        leading
    );
    fixture
        .ui()
        .global::<ShellModel>()
        .invoke_event("dialog-primary".into(), "".into());
    assert!(fixture.app().dialog_action.is_empty());
    assert_eq!(fixture.app().search_page, 0);
    assert_eq!(fixture.app().query.query, "Jump draft");
    assert_eq!(fixture.app().reader.body, BODY);
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_jump_uses_independent_scalar_byte_limits_and_keeps_long_valid_padding() -> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_jump_uses_independent_scalar_byte_limits_and_keeps_long_valid_padding",
        jump_production_limits,
    )
}

fn cut_and_delete_liveness() -> Result<()> {
    let mut fixture = Fixture::new("A")?;
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Search);
    accepted_suffix(&fixture, &editor);
    editor.select(0..1);
    fixture.paste_text("sentinel");
    let before = editor.snapshot();
    let edits = fixture.edits.get();
    let (body, account, request) = {
        let state = fixture.app();
        (
            state.reader.body.clone(),
            state.jobs.session(),
            state.search_request,
        )
    };
    {
        let _borrow = fixture.app();
        editor.cut();
        editor.delete_selection();
    }
    fixture.app().managed_sleep_blocked = true;
    editor.cut();
    editor.delete_selection();
    fixture.app().managed_sleep_blocked = false;
    fixture.app().closing.set(true);
    editor.shortcut("x");
    editor.delete_selection();
    fixture.app().closing.set(false);
    fixture.app().immediate_job_error = Some(anyhow::anyhow!("Synthetic deletion fence"));
    editor.cut();
    editor.delete_selection();
    fixture.app().immediate_job_error = None;
    assert_eq!(editor.snapshot(), before);
    assert_eq!(fixture.edits.get(), edits);
    assert_eq!(
        fixture.clipboard.borrow().default.as_deref(),
        Some("sentinel")
    );
    {
        let state = fixture.app();
        assert_eq!(state.query.query, "AB");
        assert_eq!(state.reader.body, body);
        assert_eq!(state.jobs.session(), account);
        assert_eq!(state.search_request, request);
    }
    editor.cut();
    assert_eq!(editor.text().as_str(), "B");
    assert_eq!(fixture.clipboard.borrow().default.as_deref(), Some("A"));
    editor.undo();
    assert_eq!(editor.text().as_str(), "AB");
    editor.redo();
    assert_eq!(editor.text().as_str(), "B");
    editor.undo();
    assert_eq!(editor.text().as_str(), "AB");
    editor.select(1..2);
    editor.delete_selection();
    assert_eq!(editor.text().as_str(), "A");
    editor.undo();
    assert_eq!(editor.text().as_str(), "AB");
    assert_eq!(fixture.clipboard.borrow().default.as_deref(), Some("A"));

    fixture.release_application()?;
    fixture.paste_text("sentinel");
    let before = editor.snapshot();
    let edits = fixture.edits.get();
    editor.cut();
    editor.delete_selection();
    editor.key_code(Key::Backspace);
    assert_eq!(editor.snapshot(), before);
    assert_eq!(fixture.edits.get(), edits);
    assert_eq!(
        fixture.clipboard.borrow().default.as_deref(),
        Some("sentinel")
    );
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_native_cut_and_delete_fences_preserve_clipboard_selection_and_undo() -> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_native_cut_and_delete_fences_preserve_clipboard_selection_and_undo",
        cut_and_delete_liveness,
    )
}

fn planned_keyboard_deletion() -> Result<()> {
    let original = "Ae\u{301}\u{1f9ea}\r\nB";
    let fixture = Fixture::new(original)?;
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Search);
    editor.select(4..4);
    editor.key_code(Key::Backspace);
    assert_eq!(editor.text().as_str(), "Ae\u{1f9ea}\r\nB");
    editor.undo();
    assert_eq!(editor.text().as_str(), original);
    editor.select(1..1);
    editor.key_code(Key::Delete);
    assert_eq!(editor.text().as_str(), "A\u{1f9ea}\r\nB");
    editor.undo();
    assert_eq!(editor.text().as_str(), original);
    editor.select(8..8);
    editor.key_code(Key::Delete);
    assert_eq!(editor.text().as_str(), "Ae\u{301}\u{1f9ea}B");
    editor.undo();
    assert_eq!(editor.text().as_str(), original);
    editor.select_end();
    let before = editor.snapshot();
    let edits = fixture.edits.get();
    let backspace: slint::SharedString = Key::Backspace.into();
    {
        let _borrow = fixture.app();
        editor.key_code(Key::Backspace);
        editor.shortcut(backspace.as_str());
    }
    fixture.app().managed_sleep_blocked = true;
    editor.key_code(Key::Backspace);
    editor.shortcut(backspace.as_str());
    fixture.app().managed_sleep_blocked = false;
    fixture.app().closing.set(true);
    editor.key_code(Key::Backspace);
    editor.shortcut(backspace.as_str());
    fixture.app().closing.set(false);
    assert_eq!(
        editor.snapshot(),
        before,
        "A refused deletion moved native selection"
    );
    assert_eq!(fixture.edits.get(), edits);
    assert_eq!(fixture.app().query.query, original);
    fixture.search_text("one two")?;
    editor.select_end();
    editor.shortcut(backspace.as_str());
    assert_eq!(editor.text().as_str(), "one ");
    editor.undo();
    assert_eq!(editor.text().as_str(), "one two");
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_keyboard_deletion_admits_the_original_unicode_range_before_moving_selection()
-> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_keyboard_deletion_admits_the_original_unicode_range_before_moving_selection",
        planned_keyboard_deletion,
    )
}

fn readonly_and_root_deletion() -> Result<()> {
    let fixture = Fixture::new("A")?;
    let gates = NativeGateWindow::new()?;
    let platform = fixture.app().platform.clone();
    crate::platform::configure_window(gates.window(), &platform)?;
    gates.set_value("A".into());
    let weak_ui = fixture.ui().as_weak();
    gates.on_edited(move |text| {
        if let Some(ui) = weak_ui.upgrade() {
            ui.global::<SearchModel>()
                .invoke_event("edit-query".into(), text);
        }
    });
    let inner = WindowInner::from_pub(fixture.ui().window());
    let original = inner.component();
    let gated_root = WindowInner::from_pub(gates.window()).component();
    inner.set_component(&gated_root);
    fixture.reinstall_admission();
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Search);
    accepted_suffix(&fixture, &editor);
    editor.select(0..1);
    fixture.paste_text("sentinel");
    let before = editor.snapshot();
    let edits = fixture.edits.get();
    gates.set_gate_read_only(true);
    assert!(editor.with(|input| input.read_only()));
    editor.cut();
    editor.delete_selection();
    gates.set_gate_read_only(false);
    gates.set_gate_enabled(false);
    editor.cut();
    editor.delete_selection();
    assert_eq!(editor.snapshot(), before);
    assert_eq!(fixture.edits.get(), edits);
    assert_eq!(
        fixture.clipboard.borrow().default.as_deref(),
        Some("sentinel")
    );
    gates.set_gate_enabled(true);
    editor.cut();
    assert_eq!(editor.text().as_str(), "B");
    editor.undo();
    assert_eq!(editor.text().as_str(), "AB");
    editor.select(0..1);
    let before = editor.snapshot();
    let edits = fixture.edits.get();
    fixture.paste_text("sentinel");
    // The still-registered old root must refuse deletion before any clipboard
    // or Undo work. Register the replacement only after discarding old items.
    inner.set_component(&original);
    editor.cut();
    editor.delete_selection();
    assert_eq!(editor.snapshot(), before);
    assert_eq!(fixture.edits.get(), edits);
    assert_eq!(
        fixture.clipboard.borrow().default.as_deref(),
        Some("sentinel")
    );
    drop(editor);
    fixture.reinstall_admission();
    drop(gated_root);
    drop(original);
    gates.hide()?;
    drop(gates);
    fixture.finish()
}

#[test]
fn genuine_native_readonly_and_stale_root_deletions_keep_existing_data_and_history() -> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_native_readonly_and_stale_root_deletions_keep_existing_data_and_history",
        readonly_and_root_deletion,
    )
}

fn cut_provider_changes_context() -> Result<()> {
    let fixture = Fixture::new("A")?;
    let editor = NativeEditor::find(fixture.ui().window(), EditorKind::Search);
    accepted_suffix(&fixture, &editor);
    editor.select(0..1);
    let weak_ui = fixture.ui().as_weak();
    fixture.clipboard.borrow_mut().after_default_write = Some(Box::new(move || {
        if let Some(ui) = weak_ui.upgrade() {
            ui.global::<SearchModel>().set_query("zz".into());
        }
    }));
    let edits = fixture.edits.get();
    editor.cut();
    assert_eq!(fixture.clipboard.borrow().default.as_deref(), Some("A"));
    assert_eq!(
        editor.text().as_str(),
        "zz",
        "Pending Cut overwrote provider-owned context"
    );
    assert_eq!(fixture.edits.get(), edits);
    assert_eq!(fixture.app().query.query, "AB");
    fixture.search_text("AB")?;
    editor.select_end();
    editor.undo();
    assert_eq!(
        editor.text().as_str(),
        "A",
        "A stale Cut appended removal Undo"
    );
    editor.redo();
    assert_eq!(editor.text().as_str(), "AB");
    drop(editor);
    fixture.finish()
}

#[test]
fn genuine_cut_provider_callback_cannot_spend_admission_on_changed_native_text() -> Result<()> {
    isolated(
        "app::native_edit_tests::genuine_cut_provider_callback_cannot_spend_admission_on_changed_native_text",
        cut_provider_changes_context,
    )
}
