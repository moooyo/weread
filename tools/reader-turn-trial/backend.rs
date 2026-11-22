use std::cell::RefCell;
use std::fs::File;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};

const DEADLINE_NS: u64 = 5_000_000_000;

#[derive(Clone, Copy, Debug, Default)]
pub struct Frame {
    pub target_page: i32,
    pub target_anchor: i32,
    pub draw_begin_ns: u64,
    pub draw_end_ns: u64,
    pub written_ns: u64,
    pub sent_ns: u64,
    pub marker: u32,
    pub full: bool,
    pub waveform: u32,
    pub rect: [u32; 4],
    pub wait_marker: Option<u32>,
    pub wait_begin_ns: Option<u64>,
    pub wait_end_ns: Option<u64>,
    pub wait_status: &'static str,
}

pub fn monotonic_ns() -> Option<u64> {
    let mut time = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return None;
    }
    u64::try_from(time.tv_sec).ok()?.checked_mul(1_000_000_000)?.checked_add(
        u64::try_from(time.tv_nsec).ok().filter(|value| *value < 1_000_000_000)?,
    )
}

struct WaitJob {
    file: File,
    sequence: u32,
    marker: u32,
    wait: fn(&File, u32) -> bool,
}

struct WaitAnswer {
    sequence: u32,
    marker: u32,
    begin: Option<u64>,
    end: Option<u64>,
    ok: bool,
}

struct Waiter {
    jobs: SyncSender<WaitJob>,
    answers: Receiver<WaitAnswer>,
}

impl Waiter {
    fn start() -> Option<Self> {
        let (jobs, receive) = sync_channel::<WaitJob>(1);
        let (send, answers) = sync_channel(1);
        std::thread::Builder::new().name("reader-marker".into()).spawn(move || {
            while let Ok(job) = receive.recv() {
                let begin = monotonic_ns();
                let ok = (job.wait)(&job.file, job.marker);
                let end = monotonic_ns();
                if send.try_send(WaitAnswer { sequence: job.sequence, marker: job.marker, begin, end, ok }).is_err() {
                    break;
                }
            }
        }).ok()?;
        Some(Self { jobs, answers })
    }
}

struct Turn {
    sequence: u32,
    armed_ns: u64,
    matches: Box<dyn Fn() -> bool>,
    frame: Frame,
}

#[derive(Default)]
struct Observer {
    sequence: u32,
    natural: u32,
    drawn: bool,
    wrote: bool,
    fault: Option<&'static str>,
    turn: Option<Turn>,
    wait_attempted: bool,
    waiter: Option<Waiter>,
}

thread_local! { static OBSERVER: RefCell<Option<Observer>> = const { RefCell::new(None) }; }

pub fn enable() -> bool {
    OBSERVER.with_borrow_mut(|slot| {
        if slot.is_some() { return false; }
        *slot = Some(Observer::default());
        true
    })
}

pub fn natural_frames() -> u32 {
    OBSERVER.with_borrow(|slot| slot.as_ref().map_or(0, |observer| observer.natural))
}

pub fn arm(sequence: u32, target_page: i32, target_anchor: i32, matches: impl Fn() -> bool + 'static) -> bool {
    OBSERVER.with_borrow_mut(|slot| {
        let Some(observer) = slot.as_mut() else { return false; };
        let Some(now) = monotonic_ns() else { return false; };
        if observer.fault.is_some() || observer.turn.is_some() || observer.natural == 0
            || observer.sequence.checked_add(1) != Some(sequence) || target_page < 1
            || target_anchor < 0 || !matches() { return false; }
        if !observer.wait_attempted {
            observer.wait_attempted = true;
            observer.waiter = Waiter::start();
        }
        observer.sequence = sequence;
        observer.turn = Some(Turn {
            sequence, armed_ns: now, matches: Box::new(matches),
            frame: Frame { target_page, target_anchor, ..Frame::default() },
        });
        true
    })
}

pub(crate) fn new_iteration() {
    OBSERVER.with_borrow_mut(|slot| {
        let Some(observer) = slot.as_mut() else { return; };
        observer.drawn = false;
        observer.wrote = false;
        if observer.turn.as_ref().is_some_and(|turn| turn.frame.sent_ns == 0) {
            observer.fault = Some("target-did-not-submit-in-release-iteration");
        }
    });
}

pub(crate) fn draw_begin(ordinary: bool) {
    OBSERVER.with_borrow_mut(|slot| {
        let Some(observer) = slot.as_mut() else { return; };
        observer.drawn = true;
        let Some(turn) = observer.turn.as_mut().filter(|turn| turn.frame.sent_ns == 0) else { return; };
        if !ordinary || !(turn.matches)() { observer.fault = Some("unexpected-target-at-draw"); return; }
        turn.frame.draw_begin_ns = monotonic_ns().unwrap_or(0);
    });
}

pub(crate) fn draw_end() {
    OBSERVER.with_borrow_mut(|slot| {
        if let Some(turn) = slot.as_mut().and_then(|observer| observer.turn.as_mut()).filter(|turn| turn.frame.sent_ns == 0) {
            turn.frame.draw_end_ns = monotonic_ns().unwrap_or(0);
        }
    });
}

pub(crate) fn written(region: Option<crate::PhysicalRect>, ordinary: bool) {
    OBSERVER.with_borrow_mut(|slot| {
        let Some(observer) = slot.as_mut() else { return; };
        observer.wrote = observer.drawn && region.is_some_and(|rect| rect.width != 0 && rect.height != 0);
        let Some(turn) = observer.turn.as_mut().filter(|turn| turn.frame.sent_ns == 0) else { return; };
        if !ordinary || !observer.wrote || !(turn.matches)() {
            observer.fault = Some("target-not-written-in-ordinary-frame"); return;
        }
        turn.frame.written_ns = monotonic_ns().unwrap_or(0);
    });
}

pub(crate) fn accepted(
    file: &File, marker: u32, full: bool, waveform: u32, rect: [u32; 4], wait: fn(&File, u32) -> bool,
) {
    OBSERVER.with_borrow_mut(|slot| {
        let Some(observer) = slot.as_mut() else { return; };
        if !observer.wrote { return; }
        observer.wrote = false;
        observer.natural = observer.natural.saturating_add(1);
        let Some(turn) = observer.turn.as_mut().filter(|turn| turn.frame.sent_ns == 0) else { return; };
        let frame = &mut turn.frame;
        let now = monotonic_ns().unwrap_or(0);
        if observer.fault.is_some() || marker == 0 || !(turn.armed_ns <= frame.draw_begin_ns
            && frame.draw_begin_ns <= frame.draw_end_ns && frame.draw_end_ns <= frame.written_ns
            && frame.written_ns <= now && now - turn.armed_ns < DEADLINE_NS) {
            observer.fault = Some("invalid-target-submission"); return;
        }
        frame.sent_ns = now;
        frame.marker = marker;
        frame.full = full;
        frame.waveform = waveform;
        frame.rect = rect;
        frame.wait_status = "unavailable";
        if let Some(waiter) = observer.waiter.as_ref() {
            if let Ok(file) = file.try_clone() {
                if waiter.jobs.try_send(WaitJob { file, sequence: turn.sequence, marker, wait }).is_ok() {
                    frame.wait_status = "pending";
                }
            }
        }
    });
}

pub fn poll(sequence: u32) -> Result<Option<Frame>, &'static str> {
    OBSERVER.with_borrow_mut(|slot| {
        let observer = slot.as_mut().ok_or("reader-witness-not-enabled")?;
        if let Some(error) = observer.fault { return Err(error); }
        let turn = observer.turn.as_mut().ok_or("reader-witness-not-armed")?;
        if turn.sequence != sequence || !(turn.matches)() { return Err("target-changed"); }
        let now = monotonic_ns().ok_or("reader-clock-unavailable")?;
        let frame = &mut turn.frame;
        if frame.sent_ns == 0 {
            if now.saturating_sub(turn.armed_ns) > DEADLINE_NS { return Err("no-target-send"); }
            return Ok(None);
        }
        if frame.wait_status == "pending" {
            let waiter = observer.waiter.as_ref().ok_or("wait-worker-lost")?;
            match waiter.answers.try_recv() {
                Ok(answer) => {
                    if answer.sequence != sequence || answer.marker != frame.marker { return Err("wait-marker-mismatch"); }
                    match (answer.ok, answer.begin, answer.end) {
                        (true, Some(begin), Some(end)) if frame.sent_ns <= begin && begin <= end
                            && end.saturating_sub(frame.sent_ns) <= DEADLINE_NS => {
                            frame.wait_marker = Some(answer.marker);
                            frame.wait_begin_ns = Some(begin);
                            frame.wait_end_ns = Some(end);
                            frame.wait_status = "acknowledged";
                        }
                        _ => frame.wait_status = "not-witnessed",
                    }
                }
                Err(TryRecvError::Empty) if now.saturating_sub(frame.sent_ns) < DEADLINE_NS => return Ok(None),
                Err(TryRecvError::Empty) => frame.wait_status = "timed-out",
                Err(TryRecvError::Disconnected) => frame.wait_status = "worker-unavailable",
            }
            if frame.wait_marker.is_none() { observer.waiter.take(); }
        }
        Ok(observer.turn.take().map(|turn| turn.frame))
    })
}
