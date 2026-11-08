//! Standalone offline desktop fixture runner. This is not a product entry point.

#![recursion_limit = "256"]

use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use flate2::read::GzDecoder;
use serde::Deserialize;
use serde_json::{Value, json};
use slint::{ComponentHandle, Model, fontique_010::fontique};
use weread::{
    app::{App, AppOptions, FontRecord, model},
    i18n,
    platform::{self, DeviceProfile, Platform},
    reader::TextMetrics,
    ui::{
        AppWindow, ArticleModel, AuxiliaryModel, DetailModel, DetailReview, LibraryModel,
        LoginModel, ReaderModel, SearchModel, ShellModel, Theme,
    },
};

const MAX_FONT_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Parser)]
#[command(about = "Render isolated offline UI states with observable public models")]
struct Arguments {
    #[arg(long)]
    source_dir: Option<PathBuf>,
    #[arg(long, default_value = "pw6")]
    device: DeviceProfile,
    #[arg(long, default_value = "library")]
    screen: String,
    #[arg(long)]
    data_dir: PathBuf,
    #[arg(long)]
    capture: PathBuf,
    #[arg(long)]
    actions: Option<PathBuf>,
    /// Accepted for command compatibility; fixture mode always uses demo data.
    #[arg(long)]
    demo: bool,
    /// Accepted for command compatibility; fixture mode always disables networking.
    #[arg(long)]
    offline: bool,
    #[arg(long, default_value_t = 1_200)]
    exit_after_ms: u64,
    #[arg(long, value_parser = ["login-qr", "login-code", "detail-reviews", "detail-notes", "article-qr"])]
    fixture: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Action {
    at_ms: u64,
    section: String,
    action: String,
    #[serde(default)]
    value: String,
}

fn main() -> std::process::ExitCode {
    match run(Arguments::parse()) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("offline UI fixture: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run(arguments: Arguments) -> Result<()> {
    if !cfg!(target_os = "linux") {
        bail!("Run this disposable harness on the designated remote Linux worker");
    }
    if arguments.exit_after_ms == 0 {
        bail!("A nonzero bounded deadline is required");
    }
    let mut actions: Vec<Action> = arguments
        .actions
        .as_ref()
        .map(|path| -> Result<_> { Ok(serde_json::from_slice(&fs::read(path)?)?) })
        .transpose()?
        .unwrap_or_default();
    actions.sort_by_key(|action| action.at_ms);
    if actions
        .last()
        .is_some_and(|action| action.at_ms >= arguments.exit_after_ms)
    {
        bail!("Every action must run before the requested deadline");
    }
    fs::create_dir_all(&arguments.data_dir)?;
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select()?;
    let ui = AppWindow::new()?;
    let platform = Arc::new(Platform::new(arguments.device, false)?);
    platform::configure_window(ui.window(), &platform)?;
    let source_dir = arguments.source_dir.clone().unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("The standalone fixture package must be under tools/")
            .to_path_buf()
    });
    let fonts = initialize_fonts(&ui, &source_dir)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let application = App::attach(
        &ui,
        AppOptions {
            data_dir: arguments.data_dir.clone(),
            profile: arguments.device,
            demo: true,
            offline: true,
            initial_screen: arguments.screen.clone(),
            fonts,
        },
        platform,
        &runtime,
    )?;
    let fixture_qr = fixture_qr()?;
    apply_fixture(&ui, arguments.fixture.as_deref(), &fixture_qr);
    let failures = Rc::new(RefCell::new(Vec::<String>::new()));
    let timer = slint::Timer::default();
    let weak_ui = ui.as_weak();
    let timer_application = application.clone();
    let timer_failures = failures.clone();
    let fixture = arguments.fixture.clone();
    let profile = arguments.device;
    let deadline = arguments.exit_after_ms;
    let started = Instant::now();
    let mut next_action = 0;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(50),
        move || {
            let Some(ui) = weak_ui.upgrade() else {
                timer_failures
                    .borrow_mut()
                    .push("Window was dropped".into());
                let _ = slint::quit_event_loop();
                return;
            };
            let update = (|| -> Result<()> {
                let (width, height) = profile.logical_size();
                let actual = ui.window().size();
                if actual.width != width || actual.height != height {
                    set_geometry(ui.window(), profile)?;
                }
                let elapsed = started.elapsed().as_millis() as u64;
                while let Some(action) = actions.get(next_action) {
                    if action.at_ms > elapsed {
                        break;
                    }
                    if action.section == "pointer" {
                        dispatch_pointer(ui.window(), &action.action, &action.value)?;
                    } else if action.section == "keyboard" {
                        dispatch_keyboard(ui.window(), &action.action, &action.value)?;
                    } else {
                        timer_application.borrow_mut().dispatch(
                            &action.section,
                            &action.action,
                            &action.value,
                        )?;
                    }
                    next_action += 1;
                }
                timer_application.borrow_mut().tick()?;
                // These named overlays expose otherwise unavailable display states only.
                // They neither alter controller authorization nor invoke source operations.
                apply_fixture(&ui, fixture.as_deref(), &fixture_qr);
                if elapsed >= deadline {
                    if next_action != actions.len() {
                        bail!("Not every scheduled action was dispatched");
                    }
                    slint::quit_event_loop()?;
                }
                Ok(())
            })();
            if let Err(error) = update {
                timer_failures.borrow_mut().push(format!("{error:#}"));
                let _ = slint::quit_event_loop();
            }
        },
    );
    ui.show()?;
    set_geometry(ui.window(), arguments.device)?;
    slint::run_event_loop()?;
    timer.stop();
    application.borrow_mut().shutdown()?;
    if !failures.borrow().is_empty() {
        bail!("{}", failures.borrow().join("\n"));
    }
    capture_window(&ui, &arguments.capture, arguments.device)?;
    fs::write(
        arguments.capture.with_extension("state.json"),
        serde_json::to_vec_pretty(&model_state(&ui, arguments.fixture.as_deref()))?,
    )?;
    ui.hide()?;
    drop(application);
    runtime.shutdown_timeout(Duration::from_secs(3));
    Ok(())
}

fn read_font(source: &Path, name: &str) -> Result<Arc<Vec<u8>>> {
    let mut bytes = Vec::new();
    GzDecoder::new(fs::File::open(source.join("assets/fonts").join(name))?)
        .take(MAX_FONT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_FONT_BYTES {
        bail!("Bundled font exceeds the font size limit");
    }
    Ok(Arc::new(bytes))
}

struct RegisteredFace {
    family: String,
    index: u32,
    weight: f32,
}

fn register_font_data(bytes: Arc<Vec<u8>>) -> Result<Vec<RegisteredFace>> {
    let mut collection = slint::fontique_010::shared_collection();
    let registered = collection.register_fonts(fontique::Blob::new(bytes), None);
    let mut faces = Vec::new();
    for (family, fonts) in registered {
        let family = collection
            .family_name(family)
            .context("Registered font has no family name")?
            .to_owned();
        for font in fonts {
            faces.push(RegisteredFace {
                family: family.clone(),
                index: font.index(),
                weight: font.weight().value(),
            });
        }
    }
    if faces.is_empty() {
        bail!("The font contains no supported font faces");
    }
    Ok(faces)
}

fn register_bundled_weight(
    bytes: Arc<Vec<u8>>,
    family: &str,
    weight: f32,
) -> Result<RegisteredFace> {
    register_font_data(bytes)?
        .into_iter()
        .find(|face| face.family == family && face.weight == weight)
        .with_context(|| {
            format!(
                "The bundled {family} weight {weight} did not register under its typographic family"
            )
        })
}

fn registered_metrics(bytes: &Arc<Vec<u8>>, face: &RegisteredFace) -> Result<TextMetrics> {
    TextMetrics::from_font(bytes.as_slice(), face.index).map_err(|error| anyhow!(error))
}

fn initialize_fonts(ui: &AppWindow, source: &Path) -> Result<Vec<FontRecord>> {
    let sans_regular = read_font(source, "NotoSansSC-Regular.otf.gz")?;
    let sans_medium_bytes = read_font(source, "NotoSansSC-Medium.otf.gz")?;
    let sans_bold_bytes = read_font(source, "NotoSansSC-Bold.otf.gz")?;
    let serif_light_bytes = read_font(source, "NotoSerifSC-Light.otf.gz")?;
    let serif_regular = read_font(source, "NotoSerifSC-Regular.otf.gz")?;
    let serif_medium_bytes = read_font(source, "NotoSerifSC-Medium.otf.gz")?;
    let serif_bold_bytes = read_font(source, "NotoSerifSC-Bold.otf.gz")?;
    // Keep family registration and metrics selection identical to src/main.rs.
    let sans_faces = register_font_data(sans_regular.clone())?;
    let serif_faces = register_font_data(serif_regular.clone())?;
    let sans = sans_faces
        .first()
        .context("Bundled Sans font has no face")?;
    let serif = serif_faces
        .first()
        .context("Bundled Serif font has no face")?;
    let sans_medium = register_bundled_weight(sans_medium_bytes.clone(), &sans.family, 500.0)?;
    let sans_bold = register_bundled_weight(sans_bold_bytes.clone(), &sans.family, 700.0)?;
    let serif_light = register_bundled_weight(serif_light_bytes.clone(), &serif.family, 300.0)?;
    let serif_medium = register_bundled_weight(serif_medium_bytes.clone(), &serif.family, 500.0)?;
    let serif_bold = register_bundled_weight(serif_bold_bytes.clone(), &serif.family, 700.0)?;
    let sans_metrics = registered_metrics(&sans_regular, sans)?;
    let serif_metrics = registered_metrics(&serif_regular, serif)?;
    ui.global::<Theme>().set_ui_font(sans.family.clone().into());
    ui.global::<Theme>()
        .set_serif_font(serif.family.clone().into());
    Ok(vec![
        FontRecord {
            id: "song".into(),
            label: i18n::text("song_font"),
            family: serif.family.clone(),
            metrics: serif_metrics.clone(),
            weights: BTreeMap::from([
                (300, registered_metrics(&serif_light_bytes, &serif_light)?),
                (400, serif_metrics),
                (500, registered_metrics(&serif_medium_bytes, &serif_medium)?),
                (700, registered_metrics(&serif_bold_bytes, &serif_bold)?),
            ]),
            available: true,
        },
        FontRecord {
            id: "kai".into(),
            label: i18n::text("kai_font"),
            family: String::new(),
            metrics: TextMetrics::default(),
            weights: BTreeMap::new(),
            available: false,
        },
        FontRecord {
            id: "hei".into(),
            label: i18n::text("hei_font"),
            family: sans.family.clone(),
            metrics: sans_metrics.clone(),
            weights: BTreeMap::from([
                (400, sans_metrics),
                (500, registered_metrics(&sans_medium_bytes, &sans_medium)?),
                (700, registered_metrics(&sans_bold_bytes, &sans_bold)?),
            ]),
            available: true,
        },
    ])
}

fn fixture_qr() -> Result<slint::Image> {
    let code = qrcode::QrCode::new(b"https://example.invalid/offline-ui-fixture")?;
    let quiet = 4usize;
    let scale = 8usize;
    let side = (code.width() + quiet * 2) * scale;
    let mut buffer = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(side as u32, side as u32);
    for (index, pixel) in buffer.make_mut_bytes().chunks_exact_mut(3).enumerate() {
        let x = index % side / scale;
        let y = index / side / scale;
        let dark = x >= quiet
            && y >= quiet
            && x < code.width() + quiet
            && y < code.width() + quiet
            && code[(x - quiet, y - quiet)] == qrcode::Color::Dark;
        pixel.fill(if dark { 0 } else { 255 });
    }
    Ok(slint::Image::from_rgb8(buffer))
}

fn apply_fixture(ui: &AppWindow, fixture: Option<&str>, qr: &slint::Image) {
    match fixture {
        Some("login-qr") => {
            let login = ui.global::<LoginModel>();
            login.set_step("qr".into());
            login.set_qr(qr.clone());
            login.set_qr_label("二维码有效期 02:48".into());
            login.set_busy(false);
            login.set_status("".into());
        }
        Some("login-code") => {
            let login = ui.global::<LoginModel>();
            login.set_step("code".into());
            login.set_code("482".into());
            login.set_code_length(3);
            login.set_digits(model(vec!["4".into(), "8".into(), "2".into(), "".into()]));
            login.set_can_submit(false);
            login.set_busy(false);
            login.set_status("".into());
        }
        Some("detail-reviews") | Some("detail-notes") => {
            let detail = ui.global::<DetailModel>();
            let notes = fixture == Some("detail-notes");
            detail.set_tab(if notes { "notes" } else { "reviews" }.into());
            detail.set_reviews(model(vec![DetailReview {
                id: "offline-review-fixture".into(),
                author: "离线示例读者".into(),
                meta: "12 赞 · 2026-11-03".into(),
                body: "文字的分量来自读者耐心的停留。这里是独立的离线界面样本。".into(),
                chapter: "第一章".into(),
                date: "2026-11-03".into(),
                quote: if notes {
                    "学而时习之，不亦说乎。"
                } else {
                    ""
                }
                .into(),
                thought: if notes {
                    "每一次重读都会看到新的细节。"
                } else {
                    ""
                }
                .into(),
                ..Default::default()
            }]));
            detail.set_status("".into());
            detail.set_selecting(false);
        }
        Some("article-qr") => {
            let article = ui.global::<ArticleModel>();
            article.set_qr(qr.clone());
            article.set_qr_open(true);
            article.set_menu(false);
        }
        _ => {}
    }
}

fn set_geometry(window: &slint::Window, profile: DeviceProfile) -> Result<()> {
    let (width, height) = profile.logical_size();
    let size = slint::LogicalSize::new(width as f32, height as f32);
    window.set_size(size);
    window.try_dispatch_event(slint::platform::WindowEvent::Resized { size })?;
    Ok(())
}

fn dispatch_pointer(window: &slint::Window, action: &str, value: &str) -> Result<()> {
    use slint::platform::{PointerEventButton, WindowEvent};
    let (x, y) = value
        .split_once(',')
        .context("Pointer coordinates must use x,y")?;
    let (x, y): (f32, f32) = (x.trim().parse()?, y.trim().parse()?);
    if !x.is_finite() || !y.is_finite() {
        bail!("Pointer coordinates must be finite");
    }
    let position = slint::LogicalPosition::new(x, y);
    match action {
        "tap" => {
            window.try_dispatch_event(WindowEvent::PointerMoved { position })?;
            window.try_dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            })?;
            window.try_dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            })?;
        }
        "down" => window.try_dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Left,
        })?,
        "up" => window.try_dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        })?,
        "move" => window.try_dispatch_event(WindowEvent::PointerMoved { position })?,
        _ => bail!("Unsupported pointer action {action}"),
    }
    Ok(())
}

fn dispatch_keyboard(window: &slint::Window, action: &str, value: &str) -> Result<()> {
    use slint::platform::{Key, WindowEvent};
    let text: slint::SharedString = match value {
        "Escape" => Key::Escape.into(),
        "Home" => Key::Home.into(),
        "End" => Key::End.into(),
        "LeftArrow" => Key::LeftArrow.into(),
        "RightArrow" => Key::RightArrow.into(),
        "Tab" => Key::Tab.into(),
        _ => value.into(),
    };
    match action {
        "press" => window.try_dispatch_event(WindowEvent::KeyPressed { text })?,
        "release" => window.try_dispatch_event(WindowEvent::KeyReleased { text })?,
        "tap" => {
            window.try_dispatch_event(WindowEvent::KeyPressed { text: text.clone() })?;
            window.try_dispatch_event(WindowEvent::KeyReleased { text })?;
        }
        _ => bail!("Unsupported keyboard operation {action}"),
    }
    Ok(())
}

fn snapshot_window(
    ui: &AppWindow,
    profile: DeviceProfile,
) -> Result<slint::SharedPixelBuffer<slint::Rgba8Pixel>> {
    set_geometry(ui.window(), profile)?;
    let expected = profile.logical_size();
    let actual = ui.window().size();
    let snapshot = ui.window().take_snapshot()?;
    if (actual.width, actual.height) != expected
        || (snapshot.width(), snapshot.height()) != expected
    {
        bail!("Window or snapshot does not match the requested profile geometry");
    }
    Ok(snapshot)
}

fn capture_window(
    ui: &AppWindow,
    path: &Path,
    profile: DeviceProfile,
) -> Result<()> {
    let expected = profile.logical_size();
    let snapshot = snapshot_window(ui, profile)?;
    let actual = ui.window().size();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    image::save_buffer_with_format(
        path,
        snapshot.as_bytes(),
        snapshot.width(),
        snapshot.height(),
        image::ColorType::Rgba8,
        image::ImageFormat::Png,
    )?;
    fs::write(
        path.with_extension("metadata.json"),
        serde_json::to_vec_pretty(&json!({
            "route": ui.global::<ShellModel>().get_route().to_string(),
            "profile": profile.id(), "backend": "desktop", "logical_size": [expected.0, expected.1],
            "window_size": [actual.width, actual.height], "snapshot_size": [snapshot.width(), snapshot.height()],
            "scale_factor": ui.window().scale_factor(),
        }))?,
    )?;
    Ok(())
}

fn model_state(ui: &AppWindow, fixture: Option<&str>) -> Value {
    let shell = ui.global::<ShellModel>();
    let library = ui.global::<LibraryModel>();
    let detail = ui.global::<DetailModel>();
    let search = ui.global::<SearchModel>();
    let reader = ui.global::<ReaderModel>();
    let article = ui.global::<ArticleModel>();
    let auxiliary = ui.global::<AuxiliaryModel>();
    let login = ui.global::<LoginModel>();
    json!({
        "fixture": fixture,
        "boundary": "Offline public demo fixture; no account or physical Kindle verification",
        "shell": {"route": shell.get_route().to_string(), "dialog_kind": shell.get_dialog_kind().to_string(),
            "selection_count": shell.get_selection_count(), "online": shell.get_online()},
        "library": {"page": library.get_page(), "page_count": library.get_page_count(),
            "view_mode": library.get_view_mode().to_string(), "selecting": library.get_selecting(),
            "book_ids": library.get_books().iter().map(|book| book.id.to_string()).collect::<Vec<_>>(),
            "books": library.get_books().iter().map(|book| json!({"id": book.id.to_string(),
                "title": book.title.to_string(), "cached": book.cached, "selected": book.selected})).collect::<Vec<_>>()},
        "detail": {"tab": detail.get_tab().to_string(), "selecting": detail.get_selecting(),
            "selected_count": detail.get_selected_count(), "page": detail.get_page(), "page_count": detail.get_page_count(),
            "can_read": detail.get_can_read(), "can_download": detail.get_can_download(),
            "review_count": detail.get_reviews().row_count(),
            "chapters": detail.get_chapters().iter().map(|chapter| json!({"id": chapter.id.to_string(),
                "cached": chapter.cached, "selected": chapter.selected, "current": chapter.current})).collect::<Vec<_>>()},
        "search": {"query": search.get_query().to_string(), "composition": search.get_composition().to_string(),
            "query_width": search.get_query_width(), "composition_width": search.get_composition_width(),
            "scope": search.get_scope().to_string(), "keyboard_visible": search.get_keyboard_visible(),
            "status": search.get_status().to_string(), "result_count": search.get_results().row_count(),
            "results": search.get_results().iter().map(|entry| json!({"id": entry.id.to_string(),
                "title": entry.title.to_string()})).collect::<Vec<_>>(),
            "recent": search.get_recent().iter().map(|value| value.to_string()).collect::<Vec<_>>(),
            "recent_rows": search.get_recent_rows(),
            "recent_chips": search.get_recent_chips().iter().map(|chip| json!({
                "text": chip.text.to_string(), "x": chip.x, "width": chip.width, "row": chip.row})).collect::<Vec<_>>(),
            "candidates": search.get_candidates().iter().map(|value| value.to_string()).collect::<Vec<_>>()},
        "login": {"step": login.get_step().to_string(), "code_length": login.get_code_length(),
            "digits": login.get_digits().iter().map(|value| value.to_string()).collect::<Vec<_>>()},
        "reader": {"menu": reader.get_menu(), "tool": reader.get_tool().to_string(), "subtool": reader.get_subtool().to_string(),
            "font_size": reader.get_font_size(), "font_choice": reader.get_font_choice().to_string(),
            "font_family": reader.get_font_family().to_string(), "page": reader.get_page(), "page_count": reader.get_page_count(),
            "toc_open": reader.get_toc_open(), "toc_tab": reader.get_toc_tab().to_string(),
            "thought_open": reader.get_thought_open(), "thought_position": reader.get_thought_position().to_string(),
            "thought_title": reader.get_thought_title().to_string(), "note_id": reader.get_note_id().to_string(),
            "thought_page": reader.get_thought_page(), "thought_pages": reader.get_thought_pages(),
            "bookmarked": reader.get_bookmarked(), "selection_open": reader.get_selection_open(),
            "writing_thought": reader.get_writing_thought(), "notes_panel": reader.get_notes_panel(),
            "popular": reader.get_popular(), "mine": reader.get_mine(),
            "note_count": reader.get_note_count(), "mark_count": reader.get_mark_count(),
            "selection_text": reader.get_selection_text().to_string(),
            "thought_count": reader.get_thoughts().row_count(), "entry_count": reader.get_entries().row_count(),
            "total_thought_count": reader.get_thought_count(),
            "lines": reader.get_lines().iter().map(|line| json!({"start": line.start, "end": line.end,
                "mine": line.mine, "highlight": line.highlight, "count": line.count,
                "marks": line.marks.iter().map(|mark| json!({"id": mark.id.to_string(), "x": mark.x,
                    "width": mark.width, "mine": mark.mine, "selected": mark.selected,
                    "count": mark.count})).collect::<Vec<_>>()})).collect::<Vec<_>>(),
            "thoughts": reader.get_thoughts().iter().map(|entry| json!({"id": entry.id.to_string(),
                "quote": entry.quote.to_string(), "body": entry.body.to_string(), "mine": entry.mine,
                "quote_lines": entry.quote_lines.iter().map(|line| line.to_string()).collect::<Vec<_>>()})).collect::<Vec<_>>(),
            "entries": reader.get_entries().iter().map(|entry| json!({"id": entry.id.to_string(),
                "title": entry.title.to_string(), "subtitle": entry.subtitle.to_string(),
                "chapter": entry.chapter.to_string(), "date": entry.date.to_string(),
                "page_label": entry.page_label.to_string(), "body": entry.body.to_string()})).collect::<Vec<_>>()},
        "article": {"menu": article.get_menu(), "tool": article.get_tool().to_string(),
            "font_size": article.get_font_size(), "font_choice": article.get_font_choice().to_string(),
            "font_family": article.get_font_family().to_string(), "starred": article.get_starred(),
            "qr_open": article.get_qr_open(), "list_open": article.get_list_open(), "page": article.get_page(),
            "page_count": article.get_page_count(), "blocks": article.get_blocks().iter().map(|block| json!({
                "id": block.id.to_string(), "kind": block.kind.to_string(), "height": block.height,
                "text": block.text.to_string(), "letter_spacing": block.letter_spacing,
                "inset_top": block.inset_top, "inset_bottom": block.inset_bottom,
                "inset_horizontal": block.inset_horizontal, "content_height": block.content_height,
                "frame_height": block.frame_height, "loaded": block.loaded})).collect::<Vec<_>>()},
        "auxiliary": {"settings_subpage": auxiliary.get_settings_subpage().to_string(),
            "settings_rows": auxiliary.get_settings_rows().iter().map(|entry| json!({
                "id": entry.id.to_string(), "value": entry.value.to_string(), "checked": entry.checked,
                "enabled": entry.enabled, "kind": entry.kind.to_string(),
                "selected_value": entry.selected_value.to_string(), "option_count": entry.options.row_count()})).collect::<Vec<_>>(),
            "stats_period": auxiliary.get_stats_period().to_string(),
            "stats_selected_label": auxiliary.get_stats_selected_label().to_string(),
            "publications_account_id": auxiliary.get_publications_account_id().to_string(),
            "publications_query": auxiliary.get_publications_query().to_string(),
            "publications_row_count": auxiliary.get_publications_rows().row_count()}
    })
}
