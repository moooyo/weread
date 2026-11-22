use super::{App, AppWindow, Arguments, DeviceProfile, ShellModel};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model};
use slint_backend_kindle::reader_trial;
use std::cell::RefCell;
use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use weread::ui::{ReaderModel, SleepScreenModel};

const TURNS: u32 = 24;
const WARMUPS: u32 = 3;
const HOLD_LIMIT_NS: u64 = 480_000_000;
const DEADLINE_NS: u64 = 120_000_000_000;
const FRAME_TIMEOUT_NS: u64 = 8_000_000_000;

fn build_arm() -> Option<&'static str> {
    match option_env!("WEREAD_TURN_BUILD_ARM").unwrap_or("A") {
        arm @ ("A" | "B") => Some(arm),
        _ => None,
    }
}

pub(super) fn probe_build_arm() -> Option<std::process::ExitCode> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--reader-turn-trial-arm"))
        || args.next().is_some()
    {
        return None;
    }
    Some(match build_arm() {
        Some(arm) => {
            println!("reader-turn-v1 {arm}");
            std::process::ExitCode::SUCCESS
        }
        None => std::process::ExitCode::FAILURE,
    })
}

fn clock() -> Result<u64> {
    reader_trial::monotonic_ns().context("Reader trial monotonic clock unavailable")
}

fn file_matches(path: &Path, expected: &[u8]) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    ensure!(metadata.is_file() && metadata.nlink() == 1 && metadata.len() == expected.len() as u64,
        "Reader trial marker identity mismatch");
    let mut bytes = Vec::with_capacity(expected.len());
    let file = File::open(path)?;
    ensure!(file.metadata()?.ino() == metadata.ino() && file.metadata()?.dev() == metadata.dev(),
        "Reader trial marker replaced");
    file.take(expected.len() as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes == expected, "Reader trial marker mismatch");
    Ok(())
}

struct Output {
    root: PathBuf,
    owner: String,
    device: u64,
    inode: u64,
}

impl Output {
    fn check(&self) -> Result<()> {
        let metadata = fs::symlink_metadata(&self.root)?;
        ensure!(metadata.is_dir() && metadata.dev() == self.device && metadata.ino() == self.inode
            && fs::canonicalize(&self.root)? == self.root, "Reader trial stage changed");
        file_matches(&self.root.join("owner"), self.owner.as_bytes())?;
        file_matches(&self.root.join("reader-turn-public"), b"pointer-turn-fixture\n")
    }

    fn new(arguments: &Arguments) -> Result<Self> {
        let arm = build_arm().context("Reader trial binary has an invalid build arm")?;
        ensure!(std::env::var("WEREAD_READER_TURN_EXPECTED_ARM").as_deref() == Ok(arm),
            "Reader trial binary does not match the scheduled arm");
        ensure!(arguments.kindle && arguments.device == DeviceProfile::Scribe && arguments.offline
            && !arguments.demo && arguments.screen == "reader" && arguments.actions.is_none()
            && arguments.capture.is_none() && arguments.exit_after_ms.is_none(), "Reader trial arguments rejected");
        ensure!(std::env::var_os("WEREAD_API_KEY").is_none()
            && std::env::var_os("WEREAD_STARTUP_TIMING").is_none()
            && std::env::var("WEREAD_NATIVE_RETURN").as_deref() != Ok("1")
            && std::env::var("WEREAD_RESUME_SESSION").as_deref() != Ok("1"), "Reader trial environment rejected");
        let root = PathBuf::from(std::env::var_os("WEREAD_HOME").context("Reader trial stage is absent")?);
        let label = root.file_name().and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix(".weread-turn-")).context("Reader trial stage name rejected")?;
        ensure!(root.parent() == Some(Path::new("/mnt/us")) && label.len() == 16
            && label.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)), "Reader trial stage rejected");
        let owner = format!("{label}\n");
        let data = root.join("data");
        ensure!(arguments.data_dir.as_deref() == Some(data.as_path()) && fs::canonicalize(&data)? == data,
            "Reader trial must use its isolated data directory");
        for output in ["turn-result.jsonl", "turn-complete"] {
            ensure!(fs::symlink_metadata(root.join(output)).is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
                "Reader trial output already exists");
        }
        let metadata = fs::symlink_metadata(&root)?;
        let output = Self { root, owner, device: metadata.dev(), inode: metadata.ino() };
        output.check()?;
        Ok(output)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Page {
    number: i32,
    anchor: i32,
    line: i32,
    next_anchor: i32,
    next_line: i32,
    count: i32,
}

fn page(ui: &AppWindow, app: &Rc<RefCell<App>>) -> Option<Page> {
    let shell = ui.global::<ShellModel>();
    let reader = ui.global::<ReaderModel>();
    let size = ui.window().size();
    if size.width != 1860 || size.height != 2480 || ui.window().scale_factor() != 2.0
        || shell.get_route() != "reader" || shell.get_demo() || shell.get_online()
        || !shell.get_dialog_kind().is_empty() || !shell.get_toast().is_empty()
        || ui.global::<SleepScreenModel>().get_active()
        || reader.get_title() != "Public Pointer Turn Fixture" || reader.get_chapter_title() != "Public Reader Chapter 0000"
        || reader.get_chapter_count() != 1863 || reader.get_refresh_choice() != "6"
        || reader.get_images().row_count() != 0 || reader.get_menu() || reader.get_selection_open()
        || reader.get_toc_open() || reader.get_thought_open() || reader.get_sync_conflict()
        || reader.get_writing_thought() || reader.get_access_notice() || reader.get_notes_panel()
    {
        return None;
    }
    let (number, anchor, line, next_anchor, next_line, count, end) = app.try_borrow().ok()?.trial_reader_page(ui)?;
    if reader.get_page() != number || reader.get_page_count() != count || count < 50
        || number < 1 || anchor < 0 || line < anchor || end <= line || next_anchor <= anchor || next_line < next_anchor
    {
        return None;
    }
    Some(Page { number, anchor, line, next_anchor, next_line, count })
}

#[derive(Serialize)]
struct Sample {
    sequence: u32,
    warmup: bool,
    chapter: u32,
    source_page: i32,
    target_page: i32,
    source_anchor: i32,
    target_anchor: i32,
    source_line_start: i32,
    target_line_start: i32,
    press_ns: u64,
    release_ns: u64,
    model_ns: u64,
    draw_begin_ns: u64,
    draw_end_ns: u64,
    written_ns: u64,
    sent_ns: u64,
    marker: u32,
    full: bool,
    rect: [u32; 4],
    waveform: u32,
    wait_begin_ns: Option<u64>,
    wait_end_ns: Option<u64>,
    wait_marker: Option<u32>,
    wait_status: &'static str,
}

#[derive(Clone, Copy)]
enum Phase {
    Opening,
    Rest { source: Page, since: u64 },
    Down { source: Page, press: u64 },
    AwaitFrame { source: Page, target: Page, press: u64, release: u64, model: u64 },
    Done,
}

pub(super) struct Driver {
    output: Output,
    started: u64,
    phase: Phase,
    samples: Vec<Sample>,
}

impl Driver {
    pub(super) fn configure(arguments: &Arguments) -> Result<Option<Self>> {
        match std::env::var_os("WEREAD_READER_TURN_TRIAL") {
            None => return Ok(None),
            Some(value) if value == "1" => {}
            Some(_) => bail!("Reader trial activation rejected"),
        }
        let output = Output::new(arguments)?;
        ensure!(reader_trial::enable(), "Reader trial backend activation rejected");
        Ok(Some(Self { output, started: clock()?, phase: Phase::Opening, samples: Vec::with_capacity(TURNS as usize) }))
    }

    pub(super) fn tick(&mut self, ui: &AppWindow, app: &Rc<RefCell<App>>) -> Result<bool> {
        let now = clock()?;
        ensure!(now.saturating_sub(self.started) < DEADLINE_NS, "Reader trial exceeded its deadline");
        let sequence = self.samples.len() as u32 + 1;
        let position = LogicalPosition::new(744.0, 420.0);
        match self.phase {
            Phase::Opening => {
                if reader_trial::natural_frames() > 0 {
                    let source = page(ui, app).context("First natural Reader frame lacks the public fixture")?;
                    ensure!(source.number == 1 && source.anchor == 0, "Reader trial did not start at the beginning");
                    self.phase = Phase::Rest { source, since: now };
                }
            }
            Phase::Rest { source, since } => {
                let rest = if sequence == 1 { 1_000_000_000 } else { 800_000_000 };
                if now.saturating_sub(since) >= rest {
                    ensure!(page(ui, app) == Some(source), "Reader trial source changed between turns");
                    ui.window().try_dispatch_event(WindowEvent::PointerMoved { position })?;
                    let press = clock()?;
                    ui.window().try_dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left })?;
                    self.phase = Phase::Down { source, press };
                }
            }
            Phase::Down { source, press } => {
                ensure!(page(ui, app) == Some(source), "Reader trial press changed the source page");
                let release = clock()?;
                if release.saturating_sub(press) < 45_000_000 {
                    return Ok(false);
                }
                ensure!(release > press && release - press < HOLD_LIMIT_NS, "Reader trial did not receive a short press");
                ui.window().try_dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left })?;
                let model = clock()?;
                let target = page(ui, app).context("Reader trial release did not publish a page")?;
                ensure!(target.number == source.number + 1 && target.anchor == source.next_anchor
                    && target.line == source.next_line && target.count == source.count, "Reader trial skipped the expected target");
                let weak_ui = ui.as_weak();
                let weak_app = Rc::downgrade(app);
                ensure!(reader_trial::arm(sequence, target.number, target.anchor, move || {
                    weak_ui.upgrade().zip(weak_app.upgrade()).is_some_and(|(ui, app)| page(&ui, &app) == Some(target))
                }), "Reader trial could not arm the target frame");
                self.phase = Phase::AwaitFrame { source, target, press, release, model };
            }
            Phase::AwaitFrame { source, target, press, release, model } => {
                ensure!(now.saturating_sub(release) < FRAME_TIMEOUT_NS, "Reader trial target frame timed out");
                let frame = reader_trial::poll(sequence).map_err(anyhow::Error::msg)?;
                if let Some(frame) = frame {
                    ensure!(frame.target_page == target.number && frame.target_anchor == target.anchor
                        && page(ui, app) == Some(target), "Reader trial frame identity changed");
                    ensure!(app.try_borrow()?.trial_reader_saved_anchor(target.anchor), "Reader trial durable position disagrees with the frame");
                    ensure!(release <= model && model <= frame.draw_begin_ns && frame.draw_begin_ns <= frame.draw_end_ns
                        && frame.draw_end_ns <= frame.written_ns && frame.written_ns <= frame.sent_ns && frame.sent_ns <= now,
                        "Reader trial timestamp order rejected");
                    self.samples.push(Sample { sequence, warmup: sequence <= WARMUPS, chapter: 0,
                        source_page: source.number, target_page: target.number, source_anchor: source.anchor, target_anchor: target.anchor,
                        source_line_start: source.line, target_line_start: target.line, press_ns: press, release_ns: release, model_ns: model,
                        draw_begin_ns: frame.draw_begin_ns, draw_end_ns: frame.draw_end_ns, written_ns: frame.written_ns, sent_ns: frame.sent_ns,
                        marker: frame.marker, full: frame.full, rect: frame.rect, waveform: frame.waveform, wait_begin_ns: frame.wait_begin_ns,
                        wait_end_ns: frame.wait_end_ns, wait_marker: frame.wait_marker, wait_status: frame.wait_status });
                    if sequence == TURNS {
                        self.phase = Phase::Done;
                        return Ok(true);
                    }
                    self.phase = Phase::Rest { source: target, since: now };
                }
            }
            Phase::Done => return Ok(true),
        }
        Ok(false)
    }

    pub(super) fn finish(&self) -> Result<()> {
        ensure!(matches!(self.phase, Phase::Done) && self.samples.len() == TURNS as usize,
            "Reader trial terminated without every target frame");
        self.output.check()?;
        let mut file = BufWriter::new(File::create_new(self.output.root.join("turn-result.jsonl"))?);
        for sample in &self.samples {
            serde_json::to_writer(&mut file, sample)?;
            file.write_all(b"\n")?;
        }
        file.flush()?;
        drop(file);
        let mut complete = File::create_new(self.output.root.join("turn-complete"))?;
        writeln!(complete, "{TURNS}")?;
        complete.flush()?;
        Ok(())
    }
}
