use super::*;

use std::{cell::Cell, io::Read};

use slint::{
    LogicalPosition, Model, Rgb8Pixel,
    platform::{
        Platform as SlintPlatform, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};

struct ReaderTestPlatform {
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    elapsed: Rc<Cell<Duration>>,
}

impl SlintPlatform for ReaderTestPlatform {
    fn create_window_adapter(
        &self,
    ) -> std::result::Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        self.windows.borrow_mut().push(window.clone());
        Ok(window)
    }

    fn duration_since_start(&self) -> Duration {
        self.elapsed.get()
    }
}

fn reader_test_metrics() -> Result<TextMetrics> {
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(
        &include_bytes!("../../assets/fonts/NotoSerifSC-Regular.otf.gz")[..],
    )
    .read_to_end(&mut bytes)?;
    let metrics = TextMetrics::from_font_owned(bytes.clone(), 0).map_err(anyhow::Error::msg)?;
    let mut collection = slint::fontique_010::shared_collection();
    let registered = collection.register_fonts(
        slint::fontique_010::fontique::Blob::new(Arc::new(bytes)),
        None,
    );
    assert!(
        registered
            .into_iter()
            .any(|(_, fonts)| fonts.into_iter().next().is_some())
    );
    Ok(metrics)
}

fn attach_reader_fixture(
    directory: &std::path::Path,
    metrics: &TextMetrics,
    runtime: &tokio::runtime::Runtime,
) -> Result<(AppWindow, Rc<RefCell<App>>)> {
    let ui = AppWindow::new()?;
    ui.global::<Theme>().set_ui_font("Noto Serif CJK SC".into());
    ui.global::<Theme>()
        .set_serif_font("Noto Serif CJK SC".into());
    let platform = Arc::new(Platform::new(DeviceProfile::Scribe, false)?);
    crate::platform::configure_window(ui.window(), &platform)?;
    let application = App::attach(
        &ui,
        AppOptions {
            data_dir: directory.to_path_buf(),
            profile: DeviceProfile::Scribe,
            demo: false,
            offline: true,
            initial_screen: String::new(),
            fonts: vec![FontRecord {
                id: "song".into(),
                label: "Reader test font".into(),
                family: "Noto Serif CJK SC".into(),
                metrics: metrics.clone(),
                weights: BTreeMap::new(),
                available: true,
            }],
        },
        platform,
        runtime,
    )?;
    ui.show()?;
    Ok((ui, application))
}

fn pixels(ui: &AppWindow, adapter: &MinimalSoftwareWindow) -> Vec<Rgb8Pixel> {
    let size = ui.window().size();
    let mut pixels = vec![
        Rgb8Pixel {
            r: 255,
            g: 255,
            b: 255
        };
        (size.width * size.height) as usize
    ];
    let mut rendered = false;
    adapter.request_redraw();
    adapter.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, size.width as usize);
        rendered = true;
    });
    assert!(
        rendered,
        "The software renderer must paint the reader fixture"
    );
    pixels
}

fn local_mark_drawing_id(app: &App, expected: &Annotation) -> String {
    assert_eq!(app.reader.book_id, expected.book_id);
    let header = app
        .store
        .annotation_header_by_id(&expected.book_id, &expected.id)
        .expect("The expected annotation must pass bounded header admission")
        .expect("The exact durable annotation must still exist");
    assert_eq!(header.key.id, expected.id);
    assert_eq!(header.chapter_index, expected.chapter_index);
    assert_eq!((header.start, header.end), (expected.start, expected.end));
    // The widget paints this internal token; pointer events resolve the
    // canonical durable ID separately through the production hit path.
    format!("mine-row:{}", header.key.rowid)
}

fn assert_mark_paints_ink(ui: &AppWindow, adapter: &MinimalSoftwareWindow, id: &str) {
    assert_mark_paints_ink_on_line(ui, adapter, id, None);
}

fn assert_mark_paints_ink_on_line(
    ui: &AppWindow,
    adapter: &MinimalSoftwareWindow,
    id: &str,
    source_start: Option<i32>,
) {
    let state = ui.global::<ReaderModel>();
    let rows: Vec<_> = state.get_lines().iter().collect();
    let (line, mark) = rows
        .iter()
        .find_map(|line| {
            if source_start.is_some_and(|start| line.start != start) {
                return None;
            }
            line.marks
                .iter()
                .find(|mark| mark.id.as_str() == id)
                .map(|mark| (line.clone(), mark))
        })
        .expect("The current page must project the expected source range");
    let without: Vec<_> = rows
        .iter()
        .cloned()
        .map(|mut line| {
            line.marks = model(
                line.marks
                    .iter()
                    .filter(|mark| mark.id.as_str() != id)
                    .collect(),
            );
            line
        })
        .collect();
    state.set_lines(model(without));
    let baseline = pixels(ui, adapter);
    state.set_lines(model(rows));
    let marked = pixels(ui, adapter);
    let size = ui.window().size();
    let thickness = if mark.selected { 3.0 } else { 2.0 };
    let underline_y = (line.height - thickness - 1.0)
        .min((line.height + state.get_font_size() as f32) / 2.0 + 1.0);
    let body_top = if state.get_show_heading() {
        204.0
    } else {
        76.0
    };
    let left = (state.get_margin() as f32 + mark.x).ceil().max(0.0) as usize;
    let right = (state.get_margin() as f32 + mark.x + mark.width)
        .floor()
        .min(size.width as f32) as usize;
    let top = (body_top + line.y + underline_y).floor().max(0.0) as usize;
    let bottom = (body_top + line.y + underline_y + thickness)
        .ceil()
        .min(size.height as f32) as usize;
    assert!(right > left + 4 && bottom > top);
    let mut added_ink = 0;
    for y in top..bottom {
        for x in left..right {
            let index = y * size.width as usize + x;
            if baseline[index].r > 160 && marked[index].r < 80 {
                added_ink += 1;
            }
        }
    }
    assert!(
        added_ink >= (right - left) / 4,
        "A source mark must paint visible black pixels at its measured text range: {added_ink} pixels for {left}..{right}, {top}..{bottom}"
    );
}

fn has_mark(ui: &AppWindow, id: &str) -> bool {
    ui.global::<ReaderModel>()
        .get_lines()
        .iter()
        .any(|line| line.marks.iter().any(|mark| mark.id.as_str() == id))
}

fn advance_pointer_clock(elapsed: &Cell<Duration>, milliseconds: u64) {
    elapsed.set(elapsed.get() + Duration::from_millis(milliseconds));
    slint::platform::update_timers_and_animations();
}

fn press_pointer(ui: &AppWindow, position: LogicalPosition) -> Result<()> {
    ui.window()
        .try_dispatch_event(WindowEvent::PointerMoved { position })?;
    ui.window()
        .try_dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        })?;
    Ok(())
}

fn move_pointer(ui: &AppWindow, position: LogicalPosition) -> Result<()> {
    ui.window()
        .try_dispatch_event(WindowEvent::PointerMoved { position })?;
    Ok(())
}

fn release_pointer(ui: &AppWindow, position: LogicalPosition) -> Result<()> {
    ui.window()
        .try_dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        })?;
    Ok(())
}

fn cancel_pointer(ui: &AppWindow) -> Result<()> {
    ui.window().try_dispatch_event(WindowEvent::PointerExited)?;
    Ok(())
}

fn source_point(app: &App, metrics: &TextMetrics, start: u32, end: u32) -> LogicalPosition {
    let paginator = app.reader.paginator.as_ref().unwrap();
    let line = paginator
        .current_page()
        .lines
        .iter()
        .find(|line| line.start <= start && start < end && end <= line.end)
        .expect("The requested source cluster must fit one visible line");
    let (layout, _) = app.reader_geometry();
    let left = line
        .x_at_offset(start, &layout, metrics)
        .expect("Admit the pointer fixture");
    let right = line
        .x_at_offset(end, &layout, metrics)
        .expect("Admit the pointer fixture");
    assert!(
        right > left,
        "A pointer target must have a visible glyph advance"
    );
    let body_top = if paginator.current_index() == 0 {
        204.0
    } else {
        76.0
    };
    LogicalPosition::new(
        reader_margin(true, app.reader.preferences.margin) + (left + right) / 2.0,
        body_top + line.y + line.height / 2.0,
    )
}

fn assert_drag_selection(
    application: &Rc<RefCell<App>>,
    ui: &AppWindow,
    expected: (u32, u32),
    popup: bool,
) -> Result<()> {
    let quote;
    {
        let app = application.borrow();
        assert_eq!(app.reader.selection, Some(expected));
        assert_eq!(app.reader.paginator.as_ref().unwrap().current_index(), 0);
        assert!(
            app.store.annotations(&app.reader.book_id)?.is_empty(),
            "Pointer selection must not save an annotation"
        );
        assert!(
            !app.prefs.mine,
            "Previewing text must preserve hidden saved-mark preferences"
        );
        quote = scalar_slice(&app.reader.body, expected.0, expected.1);
    }
    let state = ui.global::<ReaderModel>();
    assert_eq!(state.get_selection_text().as_str(), quote.as_str());
    assert_eq!(state.get_selection_open(), popup);
    assert!(!state.get_menu());
    assert!(!state.get_writing_thought());
    assert!(!state.get_mine());
    assert_eq!(state.get_page(), 1);
    Ok(())
}

fn assert_no_temporary_selection(application: &Rc<RefCell<App>>, ui: &AppWindow) -> Result<()> {
    {
        let app = application.borrow();
        assert!(app.reader.selection.is_none());
        assert!(app.store.annotations(&app.reader.book_id)?.is_empty());
    }
    let state = ui.global::<ReaderModel>();
    assert!(!state.get_selection_open());
    assert!(!state.get_menu());
    assert!(!has_mark(ui, "selection"));
    Ok(())
}

fn assert_selection_paints_lines(ui: &AppWindow, adapter: &MinimalSoftwareWindow, minimum: usize) {
    let starts: Vec<_> = ui
        .global::<ReaderModel>()
        .get_lines()
        .iter()
        .filter_map(|line| {
            line.marks
                .iter()
                .any(|mark| mark.id.as_str() == "selection" && mark.selected)
                .then_some(line.start)
        })
        .collect();
    assert!(
        starts.len() >= minimum,
        "A cross-line selection must project onto every touched line"
    );
    for start in starts {
        assert_mark_paints_ink_on_line(ui, adapter, "selection", Some(start));
    }
}

fn close_drag_selection(ui: &AppWindow) {
    ui.global::<ReaderModel>()
        .invoke_event("reader.selection-close".into(), "".into());
}

fn run_reader_mark_scenario() -> Result<()> {
    let windows = Rc::new(RefCell::new(Vec::new()));
    let elapsed = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(ReaderTestPlatform {
        windows: windows.clone(),
        elapsed: elapsed.clone(),
    }))?;
    let metrics = reader_test_metrics()?;
    let directory = tempfile::tempdir()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let note;
    {
        let (ui, application) = attach_reader_fixture(directory.path(), &metrics, &runtime)?;
        let adapter = windows.borrow().last().unwrap().clone();
        let (x, y);
        {
            let mut app = application.borrow_mut();
            app.prefs.pull_on_open = false;
            app.prefs.push_on_close = false;
            app.prefs.report_time = false;
            app.prefs.mine = false;
            app.store.upsert_book(&Book {
                id: "reader-render-fixture".into(),
                title: "Reader rendering fixture".into(),
                source: "fixture".into(),
                total_chapters: 1,
                on_shelf: true,
                ..Book::default()
            })?;
            app.store.upsert_chapter(&Chapter {
                id: "reader-render-chapter".into(),
                book_id: "reader-render-fixture".into(),
                index: 0,
                title: "Persistent source anchors".into(),
                cached: true,
                body: Some("A measured reader preserves \u{4e2d}\u{6587} anchors while the typography changes.\n".repeat(140)),
                content_status: ContentStatus::Full,
                ..Chapter::default()
            })?;
            app.load_reader("reader-render-fixture", 0, Some(0))?;
            app.render()?;
            assert!(!app.reader.notes_panel);
            assert_eq!(app.prefs.popup_position, "side");

            let (layout, _) = app.reader_geometry();
            let first = &app.reader.paginator.as_ref().unwrap().current_page().lines[0];
            x = (reader_margin(true, app.reader.preferences.margin)
                + (first
                    .x_at_offset(first.start + 2, &layout, &metrics)
                    .expect("Admit the pointer fixture")
                    + first
                        .x_at_offset(first.start + 3, &layout, &metrics)
                        .expect("Admit the pointer fixture"))
                    / 2.0)
                / app.reader_text_width();
            y = first.y + first.height / 2.0;
            app.reader_action("press", &format!("{x},0,{y}"))?;
            app.reader.press.as_mut().unwrap().started =
                Instant::now() - Duration::from_millis(600);
        }
        elapsed.set(Duration::from_millis(600));
        slint::platform::update_timers_and_animations();
        {
            let mut app = application.borrow_mut();
            let selected = app
                .reader
                .selection
                .expect("A stationary hold must select the pressed word");
            assert_eq!(
                scalar_slice(&app.reader.body, selected.0, selected.1),
                "measured"
            );
            assert!(!ui.global::<ReaderModel>().get_selection_open());
            app.render()?;
            assert_mark_paints_ink(&ui, &adapter, "selection");

            app.reader_action("release", &format!("{x},0,{y}"))?;
            app.reader_action("tap", &x.to_string())?;
            app.render()?;
            assert!(ui.global::<ReaderModel>().get_selection_open());
            assert!(!ui.global::<ReaderModel>().get_menu());
            assert_eq!(app.reader.paginator.as_ref().unwrap().current_index(), 0);
            app.reader_action("highlight", "selection")?;
            assert!(
                !app.prefs.mine,
                "Saving a mark must preserve the display preference"
            );
            note = app.store.annotations("reader-render-fixture")?.remove(0);
            app.reader_action("mine", "toggle")?;
            app.render()?;
            assert_mark_paints_ink(&ui, &adapter, &local_mark_drawing_id(&app, &note));

            app.reader_action("turn", "1")?;
            app.render()?;
            assert!(!has_mark(&ui, &local_mark_drawing_id(&app, &note)));
            app.reader_action("turn", "-1")?;
            app.reader_action("font-size-select", "38")?;
            app.reader_action("panel", "true")?;
            app.render()?;
            assert_mark_paints_ink(&ui, &adapter, &local_mark_drawing_id(&app, &note));
            assert_eq!(
                app.store.annotations("reader-render-fixture")?[0].quote,
                note.quote
            );
            app.finish_reading()?;
        }
        ui.hide()?;
    }
    {
        let (ui, application) = attach_reader_fixture(directory.path(), &metrics, &runtime)?;
        let adapter = windows.borrow().last().unwrap().clone();
        let mut app = application.borrow_mut();
        app.load_reader("reader-render-fixture", 0, Some(note.start))?;
        app.render()?;
        assert!(
            !app.reader.notes_panel,
            "Reopening reading must start with the full text measure"
        );
        let restored = app.store.annotations("reader-render-fixture")?.remove(0);
        assert_eq!(
            (restored.start, restored.end, &restored.quote),
            (note.start, note.end, &note.quote)
        );
        assert_eq!(app.reader.preferences.font_size, 38);
        assert_mark_paints_ink(&ui, &adapter, &local_mark_drawing_id(&app, &restored));

        let generation = ui.global::<ReaderModel>().get_press_generation();
        app.reader_action("press", "0.5,0,20")?;
        app.reader.press.as_mut().unwrap().started = Instant::now() - Duration::from_millis(600);
        app.reader_action("cancel", "")?;
        app.reader_action("hold", "")?;
        assert!(app.reader.press.is_none() && app.reader.selection.is_none());
        assert!(ui.global::<ReaderModel>().get_press_generation() > generation);
        elapsed.set(Duration::from_millis(700));
        drop(app);
        slint::platform::update_timers_and_animations();
        let mut app = application.borrow_mut();
        app.reader_action("press", "0.5,0,20")?;
        app.reader_action("hold", "")?;
        assert!(
            app.reader.selection.is_none(),
            "A new press must satisfy its own hold duration"
        );
        app.reader.press.as_mut().unwrap().started = Instant::now() - Duration::from_millis(600);
        drop(app);
        elapsed.set(Duration::from_millis(1150));
        slint::platform::update_timers_and_animations();
        assert!(
            application.borrow().reader.selection.is_none(),
            "An older timer must not select during a newer press"
        );
        elapsed.set(Duration::from_millis(1300));
        slint::platform::update_timers_and_animations();
        let mut app = application.borrow_mut();
        assert!(
            app.reader.selection.is_some(),
            "The current press must still receive its own timer"
        );
        app.reader_action("release", "0.5,0,-10")?;
        app.reader_action("tap", "0.5")?;
        app.render()?;
        let clamped = app
            .reader
            .selection
            .expect("An activated selection must clamp at the page edge");
        assert_eq!(
            clamped.0,
            app.reader.paginator.as_ref().unwrap().current_page().lines[0].start
        );
        assert!(clamped.0 < clamped.1);
        assert!(has_mark(&ui, "selection"));
        assert!(ui.global::<ReaderModel>().get_selection_open());
        assert!(!ui.global::<ReaderModel>().get_menu());
        app.reader_action("selection-close", "")?;
        app.reader_action("press", "0.5,0,20")?;
        app.reader.press.as_mut().unwrap().started = Instant::now() - Duration::from_millis(600);
        app.turn_reader(1)?;
        drop(app);
        elapsed.set(Duration::from_millis(1900));
        slint::platform::update_timers_and_animations();
        let app = application.borrow();
        assert!(app.reader.press.is_none() && app.reader.selection.is_none());
        assert_eq!(app.store.annotations("reader-render-fixture")?.len(), 1);
        ui.hide()?;
    }
    Ok(())
}

#[test]
fn reader_marks_render_and_keep_their_source_anchors_after_reflow_and_reopen() {
    std::thread::spawn(run_reader_mark_scenario)
        .join()
        .expect("The isolated reader UI scenario must not panic")
        .expect("The isolated reader UI scenario must succeed");
}

fn run_reader_drag_scenario() -> Result<()> {
    let windows = Rc::new(RefCell::new(Vec::new()));
    let elapsed = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(ReaderTestPlatform {
        windows: windows.clone(),
        elapsed: elapsed.clone(),
    }))?;
    let metrics = reader_test_metrics()?;
    let directory = tempfile::tempdir()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (ui, application) = attach_reader_fixture(directory.path(), &metrics, &runtime)?;
    let adapter = windows.borrow().last().unwrap().clone();
    let cjk = "\u{7532}\u{4e59}\u{4e19}\u{4e01}\u{620a}\u{5df1}\u{5e9a}\u{8f9b}\u{58ec}\u{7678}\u{5b50}\u{4e11}\u{5bc5}\u{536f}\u{8fb0}\u{5df3}\u{5348}\u{672a}\u{7533}\u{9149}\u{620c}\u{4ea5}";
    let cjk_body = format!("{}\n", cjk.repeat(8)).repeat(24);
    let unicode_body =
        "A e\u{301} Z unicode scalars stay attached to their source glyphs.\n".repeat(100);
    {
        let mut app = application.borrow_mut();
        app.prefs.pull_on_open = false;
        app.prefs.push_on_close = false;
        app.prefs.report_time = false;
        app.prefs.mine = false;
        app.store.upsert_book(&Book {
            id: "reader-drag-fixture".into(),
            title: "Continuous source selection".into(),
            source: "fixture".into(),
            total_chapters: 2,
            on_shelf: true,
            ..Book::default()
        })?;
        for (index, body) in [(0, cjk_body), (1, unicode_body)] {
            app.store.upsert_chapter(&Chapter {
                id: format!("reader-drag-chapter-{index}"),
                book_id: "reader-drag-fixture".into(),
                index,
                title: "Pointer source geometry".into(),
                cached: true,
                body: Some(body),
                content_status: ContentStatus::Full,
                ..Chapter::default()
            })?;
        }
        app.load_reader("reader-drag-fixture", 0, Some(0))?;
        app.render()?;
    }
    let _ = pixels(&ui, &adapter);
    let (
        anchor,
        first_end,
        last_end,
        second,
        third,
        anchor_point,
        next_point,
        far_point,
        back_point,
        second_point,
        third_move_point,
        third_up_point,
    ) = {
        let app = application.borrow();
        let page = app.reader.paginator.as_ref().unwrap().current_page();
        assert!(page.lines.len() > 3 && app.reader.paginator.as_ref().unwrap().pages.len() > 1);
        let anchor = page.lines[0].start + 6;
        let second = page.lines[1].start + 3;
        let third = page.lines[2].start + 4;
        (
            anchor,
            page.lines[0].end,
            page.lines.last().unwrap().end,
            second,
            third,
            source_point(&app, &metrics, anchor, anchor + 1),
            source_point(&app, &metrics, anchor + 1, anchor + 2),
            source_point(&app, &metrics, anchor + 3, anchor + 4),
            source_point(&app, &metrics, anchor - 2, anchor - 1),
            source_point(&app, &metrics, second, second + 1),
            source_point(&app, &metrics, third - 2, third - 1),
            source_point(&app, &metrics, third, third + 1),
        )
    };

    // Real pointer events drive the same Slint callbacks as a physical touch.
    press_pointer(&ui, anchor_point)?;
    advance_pointer_clock(&elapsed, 501);
    assert_drag_selection(&application, &ui, (anchor, anchor + 1), false)?;
    assert_mark_paints_ink(&ui, &adapter, "selection");
    move_pointer(&ui, next_point)?;
    advance_pointer_clock(&elapsed, 80);
    assert_drag_selection(&application, &ui, (anchor, anchor + 2), false)?;
    assert_mark_paints_ink(&ui, &adapter, "selection");
    move_pointer(&ui, LogicalPosition::new(next_point.x + 8.0, next_point.y))?;
    move_pointer(&ui, far_point)?;
    advance_pointer_clock(&elapsed, 80);
    assert_drag_selection(&application, &ui, (anchor, anchor + 4), false)?;
    assert_mark_paints_ink(&ui, &adapter, "selection");
    move_pointer(&ui, back_point)?;
    advance_pointer_clock(&elapsed, 80);
    assert_drag_selection(&application, &ui, (anchor - 2, anchor + 1), false)?;
    assert_mark_paints_ink(&ui, &adapter, "selection");
    move_pointer(&ui, anchor_point)?;
    advance_pointer_clock(&elapsed, 80);
    assert_drag_selection(&application, &ui, (anchor, anchor + 1), false)?;
    assert_mark_paints_ink(&ui, &adapter, "selection");
    move_pointer(&ui, second_point)?;
    advance_pointer_clock(&elapsed, 80);
    assert_drag_selection(&application, &ui, (anchor, second + 1), false)?;
    assert_selection_paints_lines(&ui, &adapter, 2);
    move_pointer(&ui, third_move_point)?;
    release_pointer(&ui, third_up_point)?;
    assert_drag_selection(&application, &ui, (anchor, third + 1), true)?;
    assert_selection_paints_lines(&ui, &adapter, 3);
    close_drag_selection(&ui);
    advance_pointer_clock(&elapsed, 100);
    assert_no_temporary_selection(&application, &ui)?;

    // An activated selection survives large movements and clamps at every page edge.
    press_pointer(&ui, anchor_point)?;
    advance_pointer_clock(&elapsed, 501);
    move_pointer(&ui, LogicalPosition::new(-40.0, anchor_point.y))?;
    advance_pointer_clock(&elapsed, 80);
    assert_drag_selection(&application, &ui, (0, anchor + 1), false)?;
    assert_mark_paints_ink(&ui, &adapter, "selection");
    let viewport = ui.window().size();
    move_pointer(
        &ui,
        LogicalPosition::new(viewport.width as f32 + 40.0, anchor_point.y),
    )?;
    advance_pointer_clock(&elapsed, 80);
    assert_drag_selection(&application, &ui, (anchor, first_end), false)?;
    move_pointer(&ui, LogicalPosition::new(anchor_point.x, 164.0))?;
    advance_pointer_clock(&elapsed, 80);
    assert_drag_selection(&application, &ui, (0, anchor + 1), false)?;
    move_pointer(
        &ui,
        LogicalPosition::new(anchor_point.x, viewport.height as f32 + 40.0),
    )?;
    advance_pointer_clock(&elapsed, 80);
    assert_drag_selection(&application, &ui, (anchor, last_end), false)?;
    let last_start = application
        .borrow()
        .reader
        .paginator
        .as_ref()
        .unwrap()
        .current_page()
        .lines
        .last()
        .unwrap()
        .start;
    assert_mark_paints_ink_on_line(&ui, &adapter, "selection", Some(last_start as i32));
    release_pointer(
        &ui,
        LogicalPosition::new(anchor_point.x, viewport.height as f32 + 60.0),
    )?;
    assert_drag_selection(&application, &ui, (anchor, last_end), true)?;
    close_drag_selection(&ui);

    press_pointer(&ui, anchor_point)?;
    advance_pointer_clock(&elapsed, 501);
    move_pointer(&ui, far_point)?;
    advance_pointer_clock(&elapsed, 80);
    assert_drag_selection(&application, &ui, (anchor, anchor + 4), false)?;
    cancel_pointer(&ui)?;
    advance_pointer_clock(&elapsed, 1000);
    assert_no_temporary_selection(&application, &ui)?;
    assert!(application.borrow().reader.press.is_none());

    // Pre-activation movement rejects a hold without turning a short drag into a tap.
    press_pointer(&ui, anchor_point)?;
    let rejected_point = LogicalPosition::new(anchor_point.x + 24.0, anchor_point.y);
    move_pointer(&ui, rejected_point)?;
    advance_pointer_clock(&elapsed, 600);
    assert_no_temporary_selection(&application, &ui)?;
    release_pointer(&ui, rejected_point)?;
    assert_no_temporary_selection(&application, &ui)?;
    assert_eq!(
        application
            .borrow()
            .reader
            .paginator
            .as_ref()
            .unwrap()
            .current_index(),
        0
    );
    press_pointer(&ui, LogicalPosition::new(700.0, anchor_point.y))?;
    move_pointer(&ui, LogicalPosition::new(500.0, anchor_point.y))?;
    advance_pointer_clock(&elapsed, 600);
    assert_no_temporary_selection(&application, &ui)?;
    release_pointer(&ui, LogicalPosition::new(300.0, anchor_point.y))?;
    assert_no_temporary_selection(&application, &ui)?;
    assert_eq!(
        application
            .borrow()
            .reader
            .paginator
            .as_ref()
            .unwrap()
            .current_index(),
        1
    );
    ui.global::<ReaderModel>()
        .invoke_event("reader.turn".into(), "-1".into());
    assert_eq!(
        application
            .borrow()
            .reader
            .paginator
            .as_ref()
            .unwrap()
            .current_index(),
        0
    );

    // A canceled hold and a queued preview cannot activate a newer press early.
    press_pointer(&ui, anchor_point)?;
    advance_pointer_clock(&elapsed, 100);
    cancel_pointer(&ui)?;
    press_pointer(&ui, far_point)?;
    advance_pointer_clock(&elapsed, 401);
    assert_no_temporary_selection(&application, &ui)?;
    advance_pointer_clock(&elapsed, 100);
    assert_drag_selection(&application, &ui, (anchor + 3, anchor + 4), false)?;
    cancel_pointer(&ui)?;
    press_pointer(&ui, anchor_point)?;
    advance_pointer_clock(&elapsed, 501);
    move_pointer(&ui, far_point)?;
    advance_pointer_clock(&elapsed, 20);
    cancel_pointer(&ui)?;
    press_pointer(&ui, far_point)?;
    advance_pointer_clock(&elapsed, 60);
    assert_no_temporary_selection(&application, &ui)?;
    advance_pointer_clock(&elapsed, 441);
    assert_drag_selection(&application, &ui, (anchor + 3, anchor + 4), false)?;
    release_pointer(&ui, far_point)?;
    assert_drag_selection(&application, &ui, (anchor + 3, anchor + 4), true)?;
    close_drag_selection(&ui);

    let (image_anchor, image_point, gap_above, gap_below, above_focus_end, below_focus_end) = {
        let mut app = application.borrow_mut();
        let (layout, first_height) = app.reader_geometry();
        // A canonical InlineImage occurrence lets the real paginator allocate the gap.
        app.reader.paginator = Some(Paginator::new_with_images(
            &app.reader.body,
            &layout,
            &metrics,
            first_height,
            &[crate::reader::InlineImage {
                id: "selection-gap-fixture".into(),
                anchor: 20,
                width: 240.0,
                height: 180.0,
            }],
        ));
        app.render()?;
        let page = app.reader.paginator.as_ref().unwrap().current_page();
        assert_eq!(page.images.len(), 1);
        let image = &page.images[0];
        let before = page
            .lines
            .iter()
            .filter(|line| line.end <= image.anchor)
            .last()
            .unwrap();
        let after = page
            .lines
            .iter()
            .find(|line| line.start >= image.anchor && line.end > line.start)
            .unwrap();
        assert!(before.y + before.height <= image.y && image.y + image.height <= after.y);
        let image_anchor = before.start + 6;
        let image_point = source_point(&app, &metrics, image_anchor, image_anchor + 1);
        let above_focus = before.start + 10;
        let below_focus = after.start + 4;
        let above_point = source_point(&app, &metrics, above_focus, above_focus + 1);
        let below_point = source_point(&app, &metrics, below_focus, below_focus + 1);
        (
            image_anchor,
            image_point,
            LogicalPosition::new(above_point.x, 204.0 + image.y + 4.0),
            LogicalPosition::new(below_point.x, 204.0 + image.y + image.height - 4.0),
            above_focus + 1,
            below_focus + 1,
        )
    };
    let _ = pixels(&ui, &adapter);
    press_pointer(&ui, image_point)?;
    advance_pointer_clock(&elapsed, 501);
    move_pointer(&ui, gap_above)?;
    advance_pointer_clock(&elapsed, 80);
    assert_drag_selection(&application, &ui, (image_anchor, above_focus_end), false)?;
    assert_mark_paints_ink(&ui, &adapter, "selection");
    move_pointer(&ui, gap_below)?;
    advance_pointer_clock(&elapsed, 80);
    assert_drag_selection(&application, &ui, (image_anchor, below_focus_end), false)?;
    assert_selection_paints_lines(&ui, &adapter, 2);
    release_pointer(&ui, gap_below)?;
    assert_drag_selection(&application, &ui, (image_anchor, below_focus_end), true)?;
    close_drag_selection(&ui);

    let (word_point, combining_point, forward_point) = {
        let mut app = application.borrow_mut();
        app.load_reader("reader-drag-fixture", 1, Some(0))?;
        app.render()?;
        (
            source_point(&app, &metrics, 5, 6),
            source_point(&app, &metrics, 2, 4),
            source_point(&app, &metrics, 7, 8),
        )
    };
    let _ = pixels(&ui, &adapter);
    press_pointer(&ui, word_point)?;
    advance_pointer_clock(&elapsed, 501);
    assert_drag_selection(&application, &ui, (5, 6), false)?;
    move_pointer(&ui, combining_point)?;
    advance_pointer_clock(&elapsed, 80);
    assert_drag_selection(&application, &ui, (2, 6), false)?;
    assert_eq!(
        ui.global::<ReaderModel>().get_selection_text().as_str(),
        "e\u{301} Z"
    );
    assert_mark_paints_ink(&ui, &adapter, "selection");
    move_pointer(&ui, forward_point)?;
    advance_pointer_clock(&elapsed, 80);
    assert_drag_selection(&application, &ui, (5, 8), false)?;
    move_pointer(&ui, combining_point)?;
    release_pointer(&ui, combining_point)?;
    assert_drag_selection(&application, &ui, (2, 6), true)?;
    ui.global::<ReaderModel>()
        .invoke_event("reader.highlight".into(), "selection".into());
    advance_pointer_clock(&elapsed, 100);
    {
        let app = application.borrow();
        let notes = app.store.annotations("reader-drag-fixture")?;
        assert_eq!(
            notes.len(),
            1,
            "Only an explicit annotation action may persist the final range"
        );
        assert_eq!(
            (notes[0].chapter_index, notes[0].start, notes[0].end),
            (1, 2, 6)
        );
        assert_eq!(notes[0].quote, "e\u{301} Z");
        assert_eq!(
            notes[0].quote,
            scalar_slice(&app.reader.body, notes[0].start, notes[0].end)
        );
        assert!(!app.prefs.mine && app.reader.selection.is_none());
        assert!(app.reader.press.is_none());
    }
    assert!(!ui.global::<ReaderModel>().get_selection_open());
    assert!(!ui.global::<ReaderModel>().get_menu());
    ui.hide()?;
    Ok(())
}

#[test]
fn reader_drag_selection_tracks_pointer_and_flushes_the_final_unicode_range() {
    std::thread::spawn(run_reader_drag_scenario)
        .join()
        .expect("The isolated pointer-selection scenario must not panic")
        .expect("The isolated pointer-selection scenario must succeed");
}

fn run_reader_access_scenario() -> Result<()> {
    use sha2::Digest;

    let windows = Rc::new(RefCell::new(Vec::new()));
    let elapsed = Rc::new(Cell::new(Duration::ZERO));
    slint::platform::set_platform(Box::new(ReaderTestPlatform {
        windows: windows.clone(),
        elapsed,
    }))?;
    let metrics = reader_test_metrics()?;
    let directory = tempfile::tempdir()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let (ui, application) = attach_reader_fixture(directory.path(), &metrics, &runtime)?;
    let adapter = windows.borrow().last().unwrap().clone();
    let session = source::AuthSession {
        user_vid: "reader-access-fixture".into(),
        cookies: BTreeMap::from([("wr_skey".into(), "synthetic-session-key".into())]),
        ..source::AuthSession::default()
    };
    let owner = hex::encode(sha2::Sha256::digest(
        format!("weread-download-vid:{}", session.user_vid).as_bytes(),
    ));
    let statuses = [
        ContentStatus::Full,
        ContentStatus::Preview,
        ContentStatus::Unknown,
        ContentStatus::Locked,
    ];
    {
        let mut app = application.borrow_mut();
        app.prefs.pull_on_open = false;
        app.prefs.push_on_close = false;
        app.prefs.report_time = false;
        app.jobs.replace_session(session.clone())?;
        app.store.upsert_book(&Book {
            id: "reader-access-fixture".into(),
            title: "Reader content access".into(),
            source: "weread".into(),
            total_chapters: statuses.len() as u32,
            on_shelf: true,
            ..Book::default()
        })?;
        let mut catalog = Vec::new();
        for (index, status) in statuses.into_iter().enumerate() {
            let uid = format!("reader-access-chapter-{index}");
            app.store.upsert_chapter(&Chapter {
                id: uid.clone(),
                book_id: "reader-access-fixture".into(),
                index: index as u32,
                title: format!("Access fixture {index}"),
                cached: status == ContentStatus::Full,
                body: (status != ContentStatus::Locked)
                    .then(|| "Preview body remains readable.".into()),
                content_status: status,
                content_account_key: owner.clone(),
            })?;
            catalog.push(source::RemoteChapter {
                uid,
                index: index as u32,
                word_count: 1000,
                ..source::RemoteChapter::default()
            });
        }
        app.store
            .set_setting("catalog.remote:reader-access-fixture", &catalog)?;
        app.load_reader("reader-access-fixture", 0, Some(0))?;
        app.render()?;
        assert!(app.reader_upload_position()?.is_some());
    }
    let state = ui.global::<ReaderModel>();
    assert!(!state.get_access_notice());
    assert!(state.get_access_hint().is_empty());
    assert!(
        state.get_access_label().is_empty(),
        "A verified full chapter must retain the quiet reading layout"
    );

    {
        let mut app = application.borrow_mut();
        app.load_reader("reader-access-fixture", 1, Some(0))?;
        app.render()?;
        assert!(!app.reader.body.is_empty());
        assert!(app.reader_upload_position()?.is_none());
        let fraction = app.local_book_fraction(app.reader.body.chars().count() as u32)?;
        assert!(
            fraction > 0.25 && fraction < 0.5,
            "A trial ending cannot finish its full chapter"
        );
        app.store.save_annotation(&Annotation {
            id: "preview-local-note".into(),
            book_id: "reader-access-fixture".into(),
            chapter_index: 1,
            start: 0,
            end: 7,
            quote: "Preview".into(),
            note: "Keep the trial note".into(),
            created_at: 1,
        })?;
        let cloud = source::ReadingPosition {
            chapter_uid: "reader-access-chapter-1".into(),
            chapter_index: 1,
            chapter_offset: 900,
            updated_at: 2,
            ..source::ReadingPosition::default()
        };
        app.apply_cloud_position(&cloud)?;
        assert!(
            app.store
                .setting::<source::ReadingPosition>("progress.pending:reader-access-fixture")?
                .is_some()
        );
        assert_eq!(
            app.reader.paginator.as_ref().unwrap().current_page().start,
            0
        );
        app.persist_reader_position()?;
        assert_eq!(
            app.store
                .position("reader-access-fixture")?
                .unwrap()
                .char_offset,
            0
        );
        app.current_download = Some(DownloadTask {
            id: "reader-access-task".into(),
            book_id: "reader-access-fixture".into(),
            account_key: owner,
            chapters: vec![1],
            status: DownloadStatus::Running,
            ..DownloadTask::default()
        });
        app.render()?;
        assert!(state.get_download_busy());
        app.current_download = None;
        app.reader_action("refresh-access", "")?;
        assert_eq!(app.reader.chapter_index, 1);
        assert_eq!(app.store.annotations("reader-access-fixture")?.len(), 1);
        app.render()?;
    }
    assert_eq!(state.get_access_label().as_str(), tr("content_preview"));
    assert!(state.get_access_notice());
    assert_eq!(state.get_access_action().as_str(), tr("retry_after_unlock"));
    let current = state.get_chapters().iter().find(|row| row.current).unwrap();
    assert!(current.meta.contains(&tr("content_preview")));
    assert!(!current.cached);

    {
        let mut app = application.borrow_mut();
        app.load_reader("reader-access-fixture", 2, Some(0))?;
        app.render()?;
        assert!(app.reader_upload_position()?.is_none());
        assert!(app.local_book_fraction(u32::MAX)? < 0.75);
        app.store
            .delete_setting("catalog.remote:reader-access-fixture")?;
        assert_eq!(
            app.local_book_fraction(u32::MAX)?,
            0.5,
            "Unknown word counts cannot expand a preview into a complete chapter"
        );
        app.load_reader("reader-access-fixture", 3, Some(0))?;
        app.render()?;
        app.change_reader_chapter(1)?;
        assert!(!app.store.book("reader-access-fixture")?.unwrap().finished);
    }
    assert_eq!(state.get_access_label().as_str(), tr("content_locked"));
    assert!(state.get_access_notice());
    {
        let mut app = application.borrow_mut();
        app.load_reader("reader-access-fixture", 2, Some(0))?;
        app.render()?;
    }
    assert_eq!(state.get_access_label().as_str(), tr("content_unverified"));
    assert_eq!(
        state.get_access_action().as_str(),
        tr("verify_content_access")
    );
    assert_ne!(state.get_access_hint().as_str(), tr("reader_locked_hint"));
    {
        let mut app = application.borrow_mut();
        let mut other = session.clone();
        other.user_vid = "reader-access-other-account".into();
        app.jobs.replace_session(other)?;
        app.load_reader("reader-access-fixture", 0, Some(0))?;
        app.render()?;
        assert!(app.reader_upload_position()?.is_none());
        assert_eq!(state.get_access_label().as_str(), tr("content_unverified"));
        assert!(
            !state
                .get_chapters()
                .iter()
                .find(|row| row.current)
                .unwrap()
                .cached
        );
        app.jobs.replace_session(session)?;
        app.load_reader("reader-access-fixture", 2, Some(0))?;
        app.render()?;
    }

    // The real notice button emits a dedicated refresh intent, without a body-surface tap.
    let intents = Rc::new(RefCell::new(Vec::new()));
    let captured = intents.clone();
    state.on_event(move |action, value| {
        captured
            .borrow_mut()
            .push((action.to_string(), value.to_string()))
    });
    state.set_can_download(true);
    let _ = pixels(&ui, &adapter);
    let point = LogicalPosition::new(
        ui.window().size().width as f32 / 2.0,
        204.0 + state.get_access_offset() + 64.0 + 12.0 + 24.0,
    );
    press_pointer(&ui, point)?;
    release_pointer(&ui, point)?;
    assert_eq!(
        &*intents.borrow(),
        &[("reader.refresh-access".into(), String::new())]
    );
    ui.hide()?;
    Ok(())
}

#[test]
fn reader_content_access_keeps_trials_readable_without_claiming_full_progress() {
    std::thread::spawn(run_reader_access_scenario)
        .join()
        .expect("The isolated content-access scenario must not panic")
        .expect("The isolated content-access scenario must succeed");
}
