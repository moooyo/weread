std::thread_local! {
    static READER_MODEL_PUBLICATIONS: Cell<usize> = const { Cell::new(0) };
}

pub(super) fn record_reader_model_publication() {
    READER_MODEL_PUBLICATIONS.with(|count| count.set(count.get() + 1));
}

#[derive(Default)]
struct PointerTurnWork {
    models: usize,
    catalogs: usize,
    weights: BookWeightWork,
}

fn take_pointer_turn_work() -> PointerTurnWork {
    PointerTurnWork {
        models: READER_MODEL_PUBLICATIONS.with(|count| count.replace(0)),
        catalogs: crate::store::COMPLETED_CHAPTER_HEADER_PROJECTIONS.with(|count| count.replace(0)),
        weights: BOOK_WEIGHT_WORK.with(|work| work.replace(BookWeightWork::default())),
    }
}

#[derive(serde::Serialize)]
struct PointerTurnSample {
    press_ns: u64,
    release_ns: u64,
    draw_ns: u64,
    release_to_draw_ns: u64,
    press_model_publications: usize,
    release_model_publications: usize,
    model_publications: usize,
    catalog_projections: usize,
    weight_builds: usize,
    weight_uid_inserts: usize,
    weight_chapter_lookups: usize,
    page: i32,
    anchor: u32,
}

enum PointerTurnPurpose {
    Regression,
    Measurement,
    Qualification(std::path::PathBuf),
}

struct PointerTurnSurface {
    adapter: Rc<MinimalSoftwareWindow>,
    width: usize,
    height: usize,
    rgb: Vec<Rgb8Pixel>,
}

impl PointerTurnSurface {
    fn primed(ui: &AppWindow, adapter: Rc<MinimalSoftwareWindow>) -> Self {
        let size = ui.window().size();
        let mut surface = Self {
            adapter,
            width: size.width as usize,
            height: size.height as usize,
            rgb: vec![
                Rgb8Pixel {
                    r: 255,
                    g: 255,
                    b: 255,
                };
                (size.width * size.height) as usize
            ],
        };
        surface.adapter.request_redraw();
        let mut primed = false;
        surface.adapter.draw_if_needed(|renderer| {
            renderer.set_repaint_buffer_type(RepaintBufferType::ReusedBuffer);
            renderer.render(&mut surface.rgb, surface.width);
            primed = true;
        });
        assert!(primed);
        surface
    }

    fn draw(&mut self) -> bool {
        let mut drawn = false;
        self.adapter.draw_if_needed(|renderer| {
            renderer.render(&mut self.rgb, self.width);
            drawn = true;
        });
        drawn
    }

    fn write_ppm(&self, path: std::path::PathBuf) -> Result<()> {
        use std::io::{BufWriter, Write};
        let mut file = BufWriter::new(std::fs::File::create_new(path)?);
        write!(file, "P6\n{} {}\n255\n", self.width, self.height)?;
        let mut row = Vec::with_capacity(self.width * 3);
        for pixels in self.rgb.chunks_exact(self.width) {
            row.clear();
            for pixel in pixels {
                row.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
            }
            file.write_all(&row)?;
        }
        file.flush()?;
        Ok(())
    }
}

const POINTER_TURN_BOOK: &str = "pointer-turn-fixture";

fn seed_pointer_turn_reader(app: &mut App, count: u32, body: &str) -> Result<()> {
    let current = count / 2;
    app.prefs.pull_on_open = false;
    app.prefs.push_on_close = false;
    app.prefs.report_time = false;
    assert!(app.force_offline);
    app.store.upsert_book(&Book {
        source: "local".into(),
        total_chapters: count,
        ..Book::new(POINTER_TURN_BOOK, "Pointer turn fixture")
    })?;
    let chapters: Vec<_> = (0..count)
        .map(|index| Chapter {
            id: format!("pointer-turn-{index}"),
            book_id: POINTER_TURN_BOOK.into(),
            index,
            title: format!("Public chapter {index}"),
            cached: index == current,
            body: (index == current).then(|| body.to_owned()),
            content_status: if index == current {
                ContentStatus::Full
            } else {
                ContentStatus::Unknown
            },
            ..Default::default()
        })
        .collect();
    app.store.upsert_chapters(&chapters)?;
    let catalog: Vec<_> = chapters
        .iter()
        .map(|chapter| source::RemoteChapter {
            uid: chapter.id.clone(),
            title: chapter.title.clone(),
            index: chapter.index,
            word_count: chapter.index + 1,
            ..Default::default()
        })
        .collect();
    source::content::validate_catalog(&catalog)?;
    app.store
        .set_setting(&format!("catalog.remote:{POINTER_TURN_BOOK}"), &catalog)?;
    app.load_local_reader(POINTER_TURN_BOOK, current, Some(0))?;
    let paginator = app.reader.paginator.as_mut().unwrap();
    assert!(paginator.pages.len() >= 5);
    paginator.set_page(1);
    app.persist_reader_position()?;
    app.render()
}

fn check_pointer_turn_page(
    app: &App,
    ui: &AppWindow,
    count: u32,
    body: &str,
    ordinal: usize,
) -> Result<u32> {
    let current = count / 2;
    assert_eq!(app.route, Route::Reader(POINTER_TURN_BOOK.into(), current));
    assert_eq!(app.reader.body, body);
    assert!(app.reader.selection.is_none());
    let paginator = app.reader.paginator.as_ref().unwrap();
    assert_eq!(paginator.current_index(), ordinal);
    let page = paginator.current_page();
    assert!(page.start > 0 && page.start < page.end);
    let state = ui.global::<ReaderModel>();
    assert!(!state.get_selection_open() && !state.get_menu());
    assert_eq!(state.get_page(), ordinal as i32 + 1);
    assert!(!page.lines.is_empty());
    let lines = state.get_lines();
    assert_eq!(lines.row_count(), page.lines.len());
    for (visible, source) in lines.iter().zip(&page.lines) {
        assert_eq!(
            (visible.start, visible.end),
            (source.start as i32, source.end as i32)
        );
        assert_eq!(visible.text.as_str(), source.text);
    }
    let saved = app.store.position(POINTER_TURN_BOOK)?.unwrap();
    assert_eq!(
        (saved.chapter_index, saved.char_offset),
        (current, page.start)
    );
    let previous_weight = f64::from(current) * f64::from(current + 1) / 2.0;
    let total_weight = f64::from(count) * f64::from(count + 1) / 2.0;
    let fraction = f64::from(page.start) / body.chars().count() as f64;
    let expected = ((previous_weight + f64::from(current + 1) * fraction) / total_weight) as f32;
    assert_eq!(state.get_progress(), expected);
    assert_eq!(
        app.store
            .book_metadata_by_id(POINTER_TURN_BOOK)?
            .unwrap()
            .progress,
        expected
    );
    Ok(page.start)
}

fn freeze_pointer_frame_clock(ui: &AppWindow) {
    ui.global::<ReaderModel>().set_clock("12:34".into());
    ui.global::<ShellModel>().set_clock("12:34".into());
}

fn sample_pointer_turn(
    ui: &AppWindow,
    surface: &mut PointerTurnSurface,
    qualify: bool,
) -> Result<PointerTurnSample> {
    let target = LogicalPosition::new(744.0, 420.0);
    move_pointer(ui, target)?;
    take_pointer_turn_work();
    let press_start = Instant::now();
    ui.window()
        .try_dispatch_event(WindowEvent::PointerPressed {
            position: target,
            button: PointerEventButton::Left,
        })?;
    let press_ns = press_start.elapsed().as_nanos() as u64;
    let down = take_pointer_turn_work();
    surface.draw();
    let release = Instant::now();
    release_pointer(ui, target)?;
    let model_ready = Instant::now();
    if qualify {
        freeze_pointer_frame_clock(ui);
    }
    let drawn = surface.draw();
    let end = Instant::now();
    assert!(
        drawn,
        "A real next-page tap must schedule its software frame"
    );
    let up = take_pointer_turn_work();
    Ok(PointerTurnSample {
        press_ns,
        release_ns: (model_ready - release).as_nanos() as u64,
        draw_ns: (end - model_ready).as_nanos() as u64,
        release_to_draw_ns: (end - release).as_nanos() as u64,
        press_model_publications: down.models,
        release_model_publications: up.models,
        model_publications: down.models + up.models,
        catalog_projections: down.catalogs + up.catalogs,
        weight_builds: down.weights.builds + up.weights.builds,
        weight_uid_inserts: down.weights.uid_inserts + up.weights.uid_inserts,
        weight_chapter_lookups: down.weights.chapter_lookups + up.weights.chapter_lookups,
        page: ui.global::<ReaderModel>().get_page(),
        anchor: 0,
    })
}

fn pointer_turn_quantiles(samples: &[PointerTurnSample], percent: usize) -> serde_json::Value {
    let quantile = |field: fn(&PointerTurnSample) -> u64| {
        let mut values: Vec<_> = samples.iter().map(field).collect();
        values.sort_unstable();
        values[(values.len() * percent).div_ceil(100) - 1]
    };
    serde_json::json!({
        "press_ns": quantile(|sample| sample.press_ns),
        "release_ns": quantile(|sample| sample.release_ns),
        "draw_ns": quantile(|sample| sample.draw_ns),
        "release_to_draw_ns": quantile(|sample| sample.release_to_draw_ns),
    })
}

fn run_pointer_turns(purpose: PointerTurnPurpose) -> Result<()> {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ReaderTestPlatform {
        windows: windows.clone(),
        elapsed: Rc::new(Cell::new(Duration::ZERO)),
    }))?;
    let metrics = reader_test_metrics()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let body: String = (0..128)
        .map(|paragraph| {
            format!(
                "Public paragraph {paragraph:03}: A reader keeps exact Unicode anchors for \u{96ea}\u{5c71} and \u{6c5f}\u{6d41} while moving between neighboring pages.\n"
            )
        })
        .collect();
    for count in [24, 1863] {
        let directory = tempfile::tempdir()?;
        let (ui, application) = attach_reader_fixture(directory.path(), &metrics, &runtime)?;
        seed_pointer_turn_reader(&mut application.borrow_mut(), count, &body)?;
        let qualify = matches!(purpose, PointerTurnPurpose::Qualification(_));
        if qualify {
            freeze_pointer_frame_clock(&ui);
        }
        let mut surface = PointerTurnSurface::primed(&ui, windows.borrow_mut().pop().unwrap());
        assert_eq!((surface.width, surface.height), (930, 1240));
        let source = check_pointer_turn_page(&application.borrow(), &ui, count, &body, 1)?;
        let (warmups, measured) = if matches!(purpose, PointerTurnPurpose::Measurement) {
            (3, 24)
        } else {
            (0, 1)
        };
        let mut samples = Vec::new();
        for iteration in 0..warmups + measured {
            let mut sample = sample_pointer_turn(&ui, &mut surface, qualify)?;
            sample.anchor = check_pointer_turn_page(&application.borrow(), &ui, count, &body, 2)?;
            assert!(sample.anchor > source);
            if iteration >= warmups {
                if matches!(purpose, PointerTurnPurpose::Regression) {
                    assert_eq!(
                        (
                            sample.press_model_publications,
                            sample.release_model_publications
                        ),
                        (0, 1)
                    );
                    assert!(sample.catalog_projections <= 2);
                    assert!(sample.weight_builds <= 2);
                }
                samples.push(sample);
            }
            if iteration + 1 < warmups + measured {
                let previous = LogicalPosition::new(186.0, 420.0);
                press_pointer(&ui, previous)?;
                release_pointer(&ui, previous)?;
                assert!(surface.draw());
                assert_eq!(
                    check_pointer_turn_page(&application.borrow(), &ui, count, &body, 1)?,
                    source
                );
            }
        }
        match &purpose {
            PointerTurnPurpose::Measurement => println!(
                "READER_TURN_JSON {}",
                serde_json::json!({
                    "chapters": count,
                    "warmups": warmups,
                    "samples": samples,
                    "p50_ns": pointer_turn_quantiles(&samples, 50),
                    "p95_ns": pointer_turn_quantiles(&samples, 95),
                })
            ),
            PointerTurnPurpose::Qualification(directory) => {
                surface.write_ppm(directory.join(format!("reader-turn-{count}.ppm")))?
            }
            PointerTurnPurpose::Regression => {}
        }
        application.borrow_mut().terminal_shutdown()?;
        ui.hide()?;
    }
    Ok(())
}

#[test]
fn ordinary_pointer_turn_projects_only_the_new_page() -> Result<()> {
    std::thread::spawn(|| run_pointer_turns(PointerTurnPurpose::Regression))
        .join()
        .unwrap()
}

#[test]
#[ignore = "software pointer and draw timing; not Kindle panel latency"]
fn measure_ordinary_pointer_turns() -> Result<()> {
    std::thread::spawn(|| run_pointer_turns(PointerTurnPurpose::Measurement))
        .join()
        .unwrap()
}

#[test]
#[ignore = "writes public software frames only to an explicitly supplied directory"]
fn qualify_ordinary_pointer_turn_frames() -> Result<()> {
    let directory = std::env::var_os("WEREAD_READER_TURN_FRAME_DIR")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| {
            anyhow::anyhow!("WEREAD_READER_TURN_FRAME_DIR must name an explicit output directory")
        })?;
    anyhow::ensure!(
        directory.is_absolute() && directory.is_dir(),
        "An existing absolute output directory is required"
    );
    std::thread::spawn(move || run_pointer_turns(PointerTurnPurpose::Qualification(directory)))
        .join()
        .unwrap()
}
