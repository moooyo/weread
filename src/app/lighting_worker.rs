//! Serialized lighting I/O shared by Reader lighting controls.

use crate::platform::{Platform, PlatformError};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LightKind {
    Brightness,
    Warmth,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum LightRequest {
    Query { brightness: bool, warmth: bool },
    Set { kind: LightKind, value: u8 },
}

#[derive(Debug)]
pub(super) struct LightReply {
    pub id: u64,
    pub result: LightResult,
}

#[derive(Debug)]
pub(super) enum LightResult {
    Query {
        brightness: Option<Result<u8, PlatformError>>,
        warmth: Option<Result<u8, PlatformError>>,
    },
    Set {
        kind: LightKind,
        value: u8,
        result: Result<(), PlatformError>,
    },
}

enum Command {
    Request { id: u64, request: LightRequest },
    Stop,
}

pub(super) struct LightWorker {
    commands: Sender<Command>,
    replies: Receiver<LightReply>,
    finished: Receiver<()>,
    stopping: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

/// Notify lifecycle waiters on normal return and while unwinding a worker panic.
pub(super) struct WorkerCompletion(Sender<()>);

impl WorkerCompletion {
    pub(super) fn new(completed: Sender<()>) -> Self {
        Self(completed)
    }
}

impl Drop for WorkerCompletion {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

/// Join only after a worker has actually terminated, keeping its handle on timeout.
pub(super) fn join_worker(
    worker_thread: &mut Option<JoinHandle<()>>,
    finished: &Receiver<()>,
    timeout: Duration,
    name: &'static str,
) -> Result<(), PlatformError> {
    let Some(handle) = worker_thread.as_ref() else {
        return Ok(());
    };
    let started = Instant::now();
    if !handle.is_finished() {
        match finished.recv_timeout(timeout) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => return Err(shutdown_timeout(name)),
        }
    }
    // Completion is sent before final thread teardown. Never let join wait on I/O.
    while !handle.is_finished() {
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return Err(shutdown_timeout(name));
        }
        thread::sleep(remaining.min(Duration::from_millis(1)));
    }
    worker_thread
        .take()
        .expect("The completed worker handle was present")
        .join()
        .map_err(|_| PlatformError::Operation(format!("the {name} panicked")))
}

impl LightWorker {
    pub fn new(platform: Arc<Platform>) -> Result<Self, PlatformError> {
        let (commands, requests) = mpsc::channel();
        let (results, replies) = mpsc::channel();
        let (completed, finished) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = stopping.clone();
        let thread = thread::Builder::new()
            .name("weread-reader-light".into())
            .spawn(move || {
                let _completed = WorkerCompletion::new(completed);
                run_worker(&platform, &requests, &results, &worker_stopping);
            })?;
        Ok(Self {
            commands,
            replies,
            finished,
            stopping,
            thread: Some(thread),
        })
    }

    pub fn send(&self, id: u64, request: LightRequest) -> Result<(), PlatformError> {
        if self.stopping.load(Ordering::Acquire) {
            return Err(PlatformError::Operation(
                "the lighting worker has stopped".into(),
            ));
        }
        if let LightRequest::Set { kind, value } = request {
            validate_value(kind, value)?;
        }
        self.commands
            .send(Command::Request { id, request })
            .map_err(|_| PlatformError::Operation("the lighting worker is unavailable".into()))
    }

    pub fn try_reply(&self) -> Result<Option<LightReply>, PlatformError> {
        match self.replies.try_recv() {
            Ok(reply) => Ok(Some(reply)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) if self.stopping.load(Ordering::Acquire) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(PlatformError::Operation(
                "the lighting worker ended unexpectedly".into(),
            )),
        }
    }

    /// Cancel queued work and wait for an operation already inside DeviceControl.
    /// A timeout keeps the join handle so a later shutdown can finish the join.
    pub fn shutdown(&mut self, timeout: Duration) -> Result<(), PlatformError> {
        self.request_stop();
        join_worker(&mut self.thread, &self.finished, timeout, "lighting worker")
    }

    pub(super) fn request_stop(&self) {
        if !self.stopping.swap(true, Ordering::AcqRel) {
            // Wake an idle receiver once. Repeated bounded join attempts must
            // not append messages while a device operation remains blocked.
            let _ = self.commands.send(Command::Stop);
        }
    }
}

impl Drop for LightWorker {
    fn drop(&mut self) {
        // Lifecycle callers use shutdown for a verified join. Drop only cancels
        // and detaches, so a failed bounded shutdown cannot block destruction.
        self.request_stop();
    }
}

fn shutdown_timeout(name: &str) -> PlatformError {
    PlatformError::Operation(format!("the {name} did not stop within its deadline"))
}

fn validate_value(kind: LightKind, value: u8) -> Result<u8, PlatformError> {
    if value <= 24 {
        Ok(value)
    } else {
        Err(PlatformError::InvalidValue(match kind {
            LightKind::Brightness => "brightness must be between 0 and 24",
            LightKind::Warmth => "warmth must be between 0 and 24",
        }))
    }
}

fn run_worker(
    platform: &Platform,
    requests: &Receiver<Command>,
    replies: &Sender<LightReply>,
    stopping: &AtomicBool,
) {
    while let Ok(command) = requests.recv() {
        if stopping.load(Ordering::Acquire) {
            break;
        }
        let Command::Request { id, request } = command else {
            break;
        };
        let Some(result) = perform_request(platform, request, stopping) else {
            break;
        };
        if stopping.load(Ordering::Acquire) || replies.send(LightReply { id, result }).is_err() {
            break;
        }
    }
}

fn perform_request(
    platform: &Platform,
    request: LightRequest,
    stopping: &AtomicBool,
) -> Option<LightResult> {
    let capabilities = platform.capabilities();
    if stopping.load(Ordering::Acquire) {
        return None;
    }
    match request {
        LightRequest::Query { brightness, warmth } => {
            let brightness = if brightness && capabilities.brightness {
                Some(
                    platform
                        .brightness()
                        .and_then(|value| validate_value(LightKind::Brightness, value)),
                )
            } else {
                None
            };
            if stopping.load(Ordering::Acquire) {
                return None;
            }
            let warmth = if warmth && capabilities.warmth {
                Some(
                    platform
                        .warmth()
                        .and_then(|value| validate_value(LightKind::Warmth, value)),
                )
            } else {
                None
            };
            if stopping.load(Ordering::Acquire) {
                return None;
            }
            Some(LightResult::Query { brightness, warmth })
        }
        LightRequest::Set { kind, value } => {
            let supported = match kind {
                LightKind::Brightness => capabilities.brightness,
                LightKind::Warmth => capabilities.warmth,
            };
            let result = if !supported {
                Err(PlatformError::Unsupported(match kind {
                    LightKind::Brightness => "brightness",
                    LightKind::Warmth => "warmth",
                }))
            } else {
                validate_value(kind, value).and_then(|value| match kind {
                    LightKind::Brightness => platform.set_brightness(value),
                    LightKind::Warmth => platform.set_warmth(value),
                })
            };
            Some(LightResult::Set {
                kind,
                value,
                result,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{DeviceControl, DeviceProfile, PlatformCapabilities};
    use std::sync::{Condvar, Mutex};

    #[derive(Default)]
    struct GateState {
        entered: bool,
        released: bool,
    }

    #[derive(Default)]
    struct OperationGate {
        state: Mutex<GateState>,
        changed: Condvar,
    }

    impl OperationGate {
        fn block(&self) -> Result<(), PlatformError> {
            let mut state = self.state.lock().unwrap();
            state.entered = true;
            self.changed.notify_all();
            let (state, timeout) = self
                .changed
                .wait_timeout_while(state, Duration::from_secs(2), |state| !state.released)
                .unwrap();
            if timeout.timed_out() && !state.released {
                Err(PlatformError::Operation(
                    "fixture operation was not released".into(),
                ))
            } else {
                Ok(())
            }
        }

        fn wait_until_entered(&self) {
            let state = self.state.lock().unwrap();
            let (state, timeout) = self
                .changed
                .wait_timeout_while(state, Duration::from_secs(3), |state| !state.entered)
                .unwrap();
            assert!(
                !timeout.timed_out() && state.entered,
                "The hardware operation did not start"
            );
        }

        fn release(&self) {
            self.state.lock().unwrap().released = true;
            self.changed.notify_all();
        }
    }

    struct FakeControl {
        capabilities: PlatformCapabilities,
        brightness: Mutex<u8>,
        warmth: Mutex<u8>,
        trace: Mutex<Vec<String>>,
        brightness_query_gate: Option<Arc<OperationGate>>,
        brightness_set_gate: Option<Arc<OperationGate>>,
    }

    impl FakeControl {
        fn new(brightness: bool, warmth: bool) -> Self {
            Self {
                capabilities: PlatformCapabilities {
                    brightness,
                    warmth,
                    ..Default::default()
                },
                brightness: Mutex::new(3),
                warmth: Mutex::new(4),
                trace: Mutex::new(vec![]),
                brightness_query_gate: None,
                brightness_set_gate: None,
            }
        }

        fn record(&self, operation: &str) {
            self.trace.lock().unwrap().push(operation.into());
        }

        fn trace(&self) -> Vec<String> {
            self.trace.lock().unwrap().clone()
        }

        fn wait_for_operation(&self, operation: &str) {
            let deadline = Instant::now() + Duration::from_secs(3);
            while !self.trace().iter().any(|entry| entry == operation) {
                assert!(
                    Instant::now() < deadline,
                    "The hardware operation did not finish"
                );
                thread::sleep(Duration::from_millis(1));
            }
        }
    }

    impl DeviceControl for FakeControl {
        fn capabilities(&self) -> PlatformCapabilities {
            self.capabilities
        }

        fn brightness(&self) -> Result<u8, PlatformError> {
            self.record("brightness-read-start");
            let value = *self.brightness.lock().unwrap();
            if let Some(gate) = &self.brightness_query_gate {
                gate.block()?;
            }
            self.record("brightness-read-end");
            Ok(value)
        }

        fn set_brightness(&self, value: u8) -> Result<(), PlatformError> {
            self.record(&format!("brightness-set-{value}-start"));
            if let Some(gate) = &self.brightness_set_gate {
                gate.block()?;
            }
            *self.brightness.lock().unwrap() = value;
            self.record(&format!("brightness-set-{value}-end"));
            Ok(())
        }

        fn warmth(&self) -> Result<u8, PlatformError> {
            self.record("warmth-read");
            Ok(*self.warmth.lock().unwrap())
        }

        fn set_warmth(&self, value: u8) -> Result<(), PlatformError> {
            self.record(&format!("warmth-set-{value}"));
            *self.warmth.lock().unwrap() = value;
            Ok(())
        }

        fn battery_percentage(&self) -> Result<u8, PlatformError> {
            panic!("A lighting worker must not query the battery")
        }

        fn set_inverted(&self, _: bool) -> Result<(), PlatformError> {
            panic!("A lighting worker must not change hardware inversion")
        }

        fn reset_idle_timer(&self) -> Result<(), PlatformError> {
            panic!("A lighting worker must not change idle policy")
        }
    }

    fn worker(control: Arc<FakeControl>) -> Result<LightWorker, PlatformError> {
        LightWorker::new(Arc::new(Platform::with_control(
            DeviceProfile::Scribe,
            false,
            control,
        )))
    }

    fn reply(worker: &LightWorker) -> LightReply {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(reply) = worker.try_reply().unwrap() {
                return reply;
            }
            assert!(
                Instant::now() < deadline,
                "The lighting worker did not reply"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn a_delayed_query_finishes_before_the_next_real_write() -> Result<(), PlatformError> {
        let gate = Arc::new(OperationGate::default());
        let mut control = FakeControl::new(true, false);
        control.brightness_query_gate = Some(gate.clone());
        let control = Arc::new(control);
        let mut worker = worker(control.clone())?;
        worker.send(
            1,
            LightRequest::Query {
                brightness: true,
                warmth: false,
            },
        )?;
        gate.wait_until_entered();
        worker.send(
            2,
            LightRequest::Set {
                kind: LightKind::Brightness,
                value: 8,
            },
        )?;
        assert_eq!(control.trace(), ["brightness-read-start"]);
        gate.release();
        let first = reply(&worker);
        assert_eq!(first.id, 1);
        assert!(matches!(
            first.result,
            LightResult::Query {
                brightness: Some(Ok(3)),
                warmth: None
            }
        ));
        let second = reply(&worker);
        assert_eq!(second.id, 2);
        assert!(matches!(
            second.result,
            LightResult::Set {
                kind: LightKind::Brightness,
                value: 8,
                result: Ok(()),
            }
        ));
        assert_eq!(
            control.trace(),
            [
                "brightness-read-start",
                "brightness-read-end",
                "brightness-set-8-start",
                "brightness-set-8-end",
            ]
        );
        assert_eq!(*control.brightness.lock().unwrap(), 8);
        worker.shutdown(Duration::from_secs(1))
    }

    #[test]
    fn stop_between_query_fields_cancels_the_remaining_reads_and_queued_write()
    -> Result<(), PlatformError> {
        let gate = Arc::new(OperationGate::default());
        let mut control = FakeControl::new(true, true);
        control.brightness_query_gate = Some(gate.clone());
        let control = Arc::new(control);
        let mut worker = worker(control.clone())?;
        worker.send(
            1,
            LightRequest::Query {
                brightness: true,
                warmth: true,
            },
        )?;
        gate.wait_until_entered();
        worker.send(
            2,
            LightRequest::Set {
                kind: LightKind::Warmth,
                value: 9,
            },
        )?;
        assert!(worker.shutdown(Duration::from_millis(10)).is_err());
        gate.release();
        worker.shutdown(Duration::from_secs(1))?;
        assert_eq!(
            control.trace(),
            ["brightness-read-start", "brightness-read-end"]
        );
        assert_eq!(*control.warmth.lock().unwrap(), 4);
        assert!(
            worker
                .send(
                    3,
                    LightRequest::Set {
                        kind: LightKind::Brightness,
                        value: 9
                    }
                )
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn shutdown_waits_for_an_inflight_setter_and_never_runs_the_queued_setter()
    -> Result<(), PlatformError> {
        let gate = Arc::new(OperationGate::default());
        let mut control = FakeControl::new(true, true);
        control.brightness_set_gate = Some(gate.clone());
        let control = Arc::new(control);
        let mut worker = worker(control.clone())?;
        worker.send(
            1,
            LightRequest::Set {
                kind: LightKind::Brightness,
                value: 8,
            },
        )?;
        gate.wait_until_entered();
        worker.send(
            2,
            LightRequest::Set {
                kind: LightKind::Warmth,
                value: 9,
            },
        )?;
        let stopping = worker.stopping.clone();
        let release = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !stopping.load(Ordering::Acquire) && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(1));
            }
            let observed_stop = stopping.load(Ordering::Acquire);
            gate.release();
            observed_stop
        });
        worker.shutdown(Duration::from_secs(1))?;
        assert!(
            release.join().unwrap(),
            "The operation must be released only after cancellation"
        );
        control.record("shutdown-returned");
        assert_eq!(
            control.trace(),
            [
                "brightness-set-8-start",
                "brightness-set-8-end",
                "shutdown-returned",
            ]
        );
        assert_eq!(*control.brightness.lock().unwrap(), 8);
        assert_eq!(*control.warmth.lock().unwrap(), 4);
        assert!(worker.thread.is_none());
        Ok(())
    }

    #[test]
    fn a_shutdown_timeout_retains_the_inflight_handle_for_a_later_join() -> Result<(), PlatformError>
    {
        let gate = Arc::new(OperationGate::default());
        let mut control = FakeControl::new(true, false);
        control.brightness_set_gate = Some(gate.clone());
        let control = Arc::new(control);
        let mut worker = worker(control.clone())?;
        worker.send(
            1,
            LightRequest::Set {
                kind: LightKind::Brightness,
                value: 12,
            },
        )?;
        gate.wait_until_entered();
        assert!(worker.shutdown(Duration::from_millis(10)).is_err());
        assert!(worker.thread.is_some());
        assert_eq!(control.trace(), ["brightness-set-12-start"]);
        gate.release();
        worker.shutdown(Duration::from_secs(1))?;
        assert!(worker.thread.is_none());
        assert_eq!(
            control.trace(),
            ["brightness-set-12-start", "brightness-set-12-end"]
        );
        Ok(())
    }

    #[test]
    fn repeated_shutdown_polls_keep_one_stop_message_while_device_io_is_blocked()
    -> Result<(), PlatformError> {
        let gate = Arc::new(OperationGate::default());
        let mut control = FakeControl::new(true, false);
        control.brightness_set_gate = Some(gate.clone());
        let control = Arc::new(control);
        let platform = Arc::new(Platform::with_control(
            DeviceProfile::Scribe,
            false,
            control.clone(),
        ));
        let (commands, requests) = mpsc::channel();
        let (results, replies) = mpsc::channel();
        let (completed, finished) = mpsc::channel();
        let (observed_commands, remaining_commands) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = stopping.clone();
        let thread = thread::spawn(move || {
            let _completed = WorkerCompletion::new(completed);
            run_worker(&platform, &requests, &results, &worker_stopping);
            // Cancellation exits immediately after the blocked operation. Count
            // the actual unconsumed channel entries before the receiver drops.
            let pending = requests.try_iter().collect::<Vec<_>>();
            observed_commands
                .send((
                    pending.len(),
                    pending
                        .iter()
                        .filter(|command| matches!(command, Command::Stop))
                        .count(),
                ))
                .unwrap();
        });
        let mut worker = LightWorker {
            commands,
            replies,
            finished,
            stopping,
            thread: Some(thread),
        };
        worker.send(
            1,
            LightRequest::Set {
                kind: LightKind::Brightness,
                value: 12,
            },
        )?;
        gate.wait_until_entered();
        for _ in 0..1_024 {
            assert!(worker.shutdown(Duration::ZERO).is_err());
            assert!(worker.thread.is_some());
        }
        assert!(worker.stopping.load(Ordering::Acquire));
        assert_eq!(control.trace(), ["brightness-set-12-start"]);
        gate.release();
        worker.shutdown(Duration::from_secs(1))?;
        assert!(worker.thread.is_none());
        assert_eq!(remaining_commands.recv().unwrap(), (1, 1));
        assert!(worker.try_reply()?.is_none());
        assert_eq!(
            control.trace(),
            ["brightness-set-12-start", "brightness-set-12-end"]
        );
        Ok(())
    }

    #[test]
    fn capabilities_are_independent_and_values_are_never_invented_or_clamped()
    -> Result<(), PlatformError> {
        let control = Arc::new(FakeControl::new(false, true));
        *control.warmth.lock().unwrap() = 25;
        let mut worker = worker(control.clone())?;
        worker.send(
            1,
            LightRequest::Query {
                brightness: true,
                warmth: true,
            },
        )?;
        assert!(matches!(
            reply(&worker).result,
            LightResult::Query {
                brightness: None,
                warmth: Some(Err(PlatformError::InvalidValue(_))),
            }
        ));
        worker.send(
            2,
            LightRequest::Set {
                kind: LightKind::Brightness,
                value: 8,
            },
        )?;
        assert!(matches!(
            reply(&worker).result,
            LightResult::Set {
                kind: LightKind::Brightness,
                value: 8,
                result: Err(PlatformError::Unsupported("brightness")),
            }
        ));
        assert!(matches!(
            worker.send(
                3,
                LightRequest::Set {
                    kind: LightKind::Warmth,
                    value: 25
                }
            ),
            Err(PlatformError::InvalidValue(_)),
        ));
        assert_eq!(control.trace(), ["warmth-read"]);
        worker.shutdown(Duration::from_secs(1))
    }

    #[test]
    fn drop_cancels_without_waiting_for_a_blocked_device_call() -> Result<(), PlatformError> {
        let gate = Arc::new(OperationGate::default());
        let mut control = FakeControl::new(true, false);
        control.brightness_set_gate = Some(gate.clone());
        let control = Arc::new(control);
        let worker = worker(control.clone())?;
        worker.send(
            1,
            LightRequest::Set {
                kind: LightKind::Brightness,
                value: 8,
            },
        )?;
        gate.wait_until_entered();
        let (dropped, drop_result) = mpsc::channel();
        let dropping = thread::spawn(move || {
            drop(worker);
            dropped.send(()).unwrap();
        });
        let returned_without_release = drop_result.recv_timeout(Duration::from_millis(500)).is_ok();
        gate.release();
        control.wait_for_operation("brightness-set-8-end");
        dropping.join().unwrap();
        assert!(
            returned_without_release,
            "Drop must not join an unresponsive device call"
        );
        assert_eq!(
            control.trace(),
            ["brightness-set-8-start", "brightness-set-8-end"]
        );
        Ok(())
    }
}
