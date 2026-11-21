use super::*;
use crate::gyro::NativePortrait;
use crate::managed_sleep::ManagedSleepState;
use crate::orientation::NativePortraitOrientation::{Down, Up};
use crate::orientation::PortraitCalibration;
use crate::touch::tests::{event_pipe, touch_pipe_for_size, write_packet};
use slint::{ComponentHandle, platform::WindowEvent};
use std::cell::Cell;
use std::os::fd::OwnedFd;

const WIDTH: u32 = 1860;
const HEIGHT: u32 = 2480;
type Point = (i32, i32);
type Hit = (String, f32, f32);
const TARGETS: [(&str, Point, Point, Point); 3] = [
    ("mirror", (234, 76), (1626, 2404), (117, 38)),
    ("settings", (1626, 2404), (234, 76), (813, 1202)),
    ("library", (232, 2404), (1628, 76), (116, 1202)),
];

slint::slint! {
    component Target inherits Rectangle {
        callback hit(float, float);
        callback cancelled();
        out property <bool> held: area.pressed;
        area := TouchArea {
            clicked => {
                root.hit((root.x + self.mouse-x) / 1px, (root.y + self.mouse-y) / 1px);
            }
            pointer-event(event) => {
                if event.kind == PointerEventKind.cancel { root.cancelled(); }
            }
        }
    }
    export component PoseWindow inherits Window {
        width: 930px;
        height: 1240px;
        callback hit(string, float, float);
        out property <bool> settings-held: settings.held;
        out property <int> cancellations;
        Target {
            background: white;
            hit(column, row) => { root.hit("mirror", column, row); }
        }
        Target {
            x: 80px; y: 1170px; width: 72px; height: 64px; background: #808080;
            hit(column, row) => { root.hit("library", column, row); }
        }
        settings := Target {
            x: 777px; y: 1170px; width: 72px; height: 64px; background: black;
            hit(column, row) => { root.hit("settings", column, row); }
            cancelled => { root.cancellations += 1; }
        }
    }
}

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn sensor() -> (GyroObserver, OwnedFd) {
    let (reader, writer) = event_pipe();
    (GyroObserver::from_test_stream(reader.into()), writer)
}

fn pose_report(observer: &mut GyroObserver, writer: &OwnedFd, pressure: i32) -> NativePortrait {
    write_packet(writer, 3, 0x18, pressure);
    assert_eq!(observer.poll().unwrap(), None);
    assert!(!observer.is_synchronized());
    write_packet(writer, 0, 0, 0);
    let pose = observer.poll().unwrap().unwrap();
    assert!(observer.is_synchronized());
    pose
}

fn request(
    sequence: u64,
    direction: NativePortraitOrientation,
    rotation: u32,
) -> OrientationRequest {
    OrientationRequest {
        sequence,
        calibration: Some(PortraitCalibration {
            holding: direction,
            scanout: direction,
            raw_rotation: rotation,
        }),
    }
}

fn launch(inverted: bool, changed_before_loop: bool) -> (PortraitRuntime, OwnedFd) {
    let (holding, rotation, pressure) = if inverted { (Down, 1, 16) } else { (Up, 3, 15) };
    let initial = request(1, holding, rotation);
    let splash = resolve_launch_orientation(
        "splash",
        || {
            let (mut probe, writer) = sensor();
            confirmed_launch_mapping(
                Some((holding, rotation)),
                || Ok(rotation),
                || Some((pose_report(&mut probe, &writer, pressure).into(), ())),
            )
            .map(|(mapping, ())| mapping.render_offset())
        },
        || panic!("a matched splash must reuse its orientation"),
        std::convert::identity,
    );
    assert_eq!(splash, 0);

    let pressure = if changed_before_loop { 16 } else { pressure };
    let legacy_calls = Cell::new(0);
    let (mut probe, mut writer) = sensor();
    let sampled_fd = probe.fd();
    let mut runtime = resolve_launch_orientation_path(
        "event_loop",
        || {
            confirmed_event_loop_launch(
                initial,
                || Some(initial),
                || Ok(rotation),
                || {
                    let pose = pose_report(&mut probe, &writer, pressure);
                    Some((pose.into(), probe))
                },
                |observer| {
                    assert_eq!(observer.fd(), sampled_fd);
                    assert!(observer.is_synchronized() && !observer.has_backlog());
                    Some(if pressure == 16 { Down } else { Up })
                },
            )
            .map(|(controller, observer, path)| {
                (
                    PortraitRuntime::from_launch(controller, observer, initial.sequence),
                    path,
                )
            })
        },
        || {
            legacy_calls.set(legacy_calls.get() + 1);
            Err(OrientationQueryError::TimedOut)
        },
        PortraitRuntime::new,
    );
    assert_eq!(
        legacy_calls.get(),
        usize::from(changed_before_loop && !cfg!(feature = "startup-orientation-reconcile"))
    );
    if runtime.observer.is_some() {
        assert!(
            !runtime.apply_new_request(initial, || panic!("launch generation already applied"))
        );
        assert_eq!(runtime.observer.as_ref().unwrap().fd(), sampled_fd);
        assert!(runtime.attempted && !runtime.snapshot_pending && !runtime.input_gated());
    } else {
        assert!(changed_before_loop);
        let (mut reopened, reopened_writer) = sensor();
        writer = reopened_writer;
        let pose = pose_report(&mut reopened, &writer, pressure);
        assert!(runtime.controller.apply_request(initial, rotation).unwrap());
        assert!(
            runtime
                .observe_pose_with(Some(pose), || Ok(rotation))
                .unwrap()
        );
        runtime.observer = Some(reopened);
    }
    (runtime, writer)
}

fn motion(writer: &OwnedFd, point: Point) {
    write_packet(writer, 3, 0x35, point.0);
    write_packet(writer, 3, 0x36, point.1);
    write_packet(writer, 0, 0, 0);
}

fn press(writer: &OwnedFd, tracking: i32, point: Point) {
    write_packet(writer, 3, 0x39, tracking);
    motion(writer, point);
}

fn lift(writer: &OwnedFd) {
    write_packet(writer, 3, 0x39, -1);
    write_packet(writer, 0, 0, 0);
}

struct Presentation {
    window: Rc<MinimalSoftwareWindow>,
    ui: PoseWindow,
    hits: Rc<RefCell<Vec<Hit>>>,
    runtime: PortraitRuntime,
    sensor_writer: OwnedFd,
    input: Option<TouchInput>,
    writer: OwnedFd,
    generation: u64,
    frames: u32,
    power: ManagedSleepControl,
    gate: PostFrameInputGate,
}

impl Presentation {
    fn new(inverted: bool, changed_before_loop: bool) -> Self {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).unwrap();
        let ui = PoseWindow::new().unwrap();
        window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
        window.set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
        ui.show().unwrap();
        let hits = Rc::new(RefCell::new(Vec::new()));
        let recorded = hits.clone();
        ui.on_hit(move |target, column, row| {
            recorded
                .borrow_mut()
                .push((target.to_string(), column, row));
        });
        let (runtime, sensor_writer) = launch(inverted, changed_before_loop);
        let (input, writer) = touch_pipe_for_size(WIDTH, HEIGHT);
        Self {
            window,
            ui,
            hits,
            runtime,
            sensor_writer,
            input: Some(input),
            writer,
            generation: 1,
            frames: 0,
            power: ManagedSleepControl::default(),
            gate: PostFrameInputGate::default(),
        }
    }

    fn present(&mut self, expect_turned: bool) {
        assert!(!self.runtime.input_gated());
        let size = (WIDTH * HEIGHT) as usize;
        let mut pixels = vec![Rgb8Pixel::default(); size];
        self.window.request_redraw();
        assert!(self.window.draw_if_needed(|renderer| {
            renderer.set_rendering_rotation(match self.runtime.controller.render_offset() {
                0 => RenderingRotation::NoRotation,
                180 => RenderingRotation::Rotate180,
                unexpected => panic!("invalid portrait offset {unexpected}"),
            });
            let dirty = renderer.render(&mut pixels, WIDTH as usize);
            assert_eq!(
                dirty.bounding_box_size(),
                slint::PhysicalSize::new(WIDTH, HEIGHT)
            );
        }));
        for (column, row, gray) in [(1626, 2404, 0), (232, 2404, 128), (234, 76, 255)] {
            let index = row * WIDTH as usize + column;
            let pixel = pixels[if expect_turned {
                size - 1 - index
            } else {
                index
            }];
            assert_eq!((pixel.r, pixel.g, pixel.b), (gray, gray, gray));
        }
        self.frames += 1;
        let drain = self
            .input
            .as_mut()
            .unwrap()
            .finish_presentation_transition()
            .unwrap();
        assert_eq!(drain, TransitionDrain::Ready);
        let resuming = self.power.state() == ManagedSleepState::Resuming;
        self.gate.finish_frame(drain, resuming, &self.power);
        assert_eq!(self.power.state(), ManagedSleepState::Running);
    }

    fn poll(&mut self) {
        self.input
            .as_mut()
            .unwrap()
            .poll(&self.window, self.runtime.controller.touch_offset())
            .unwrap();
    }

    fn turn_down(&mut self) {
        write_packet(&self.sensor_writer, 3, 0x18, 16);
        assert_eq!(
            self.runtime.observer.as_mut().unwrap().poll().unwrap(),
            None
        );
        assert!(self.runtime.input_gated());
        write_packet(&self.sensor_writer, 0, 0, 0);
        let pose = self.runtime.observer.as_mut().unwrap().poll().unwrap();
        assert!(!self.runtime.input_gated());
        assert!(self.runtime.observe_pose_with(pose, || Ok(3)).unwrap());
        begin_presentation_transition(&mut self.input, &mut self.generation, &self.window).unwrap();
    }

    fn hold_settings(&mut self) {
        press(&self.writer, 8, (1626, 2404));
        self.poll();
        assert!(self.ui.get_settings_held());
        assert!(self.hits.borrow().is_empty());
    }

    fn wake_inverted(&mut self) {
        self.hold_settings();
        assert!(self.power.request(true));
        let paused = pause_native_inputs(
            &self.power,
            &mut self.input,
            &mut self.runtime,
            &mut self.generation,
            &self.window,
            || Ok(()),
        );
        assert!(paused.unwrap());
        assert_eq!(self.power.state(), ManagedSleepState::Suspended);
        assert!(self.input.is_none() && self.runtime.observer.is_none());
        assert!(!self.ui.get_settings_held());
        assert_eq!(self.ui.get_cancellations(), 1);

        let wake = {
            let (mut probe, writer) = sensor();
            request(2, pose_report(&mut probe, &writer, 16).into(), 1)
        };
        assert!(self.power.request(false));
        let (fresh, reopened) = touch_pipe_for_size(WIDTH, HEIGHT);
        let resumed =
            begin_managed_resume(&self.power, &mut self.input, || Ok::<_, io::Error>(fresh));
        assert!(resumed.unwrap());
        self.writer = reopened;
        assert!(self.runtime.controller.apply_request(wake, 1).unwrap());
        let (mut observer, writer) = sensor();
        let pose = pose_report(&mut observer, &writer, 16);
        assert!(
            !self
                .runtime
                .observe_pose_with(Some(pose), || Ok(1))
                .unwrap()
        );
        self.runtime.observer = Some(observer);
        self.sensor_writer = writer;
        motion(&self.writer, (234, 76));
        assert_eq!(self.power.state(), ManagedSleepState::Resuming);
        self.present(false);
        motion(&self.writer, (234, 76));
        self.poll();
        assert!(!self.ui.get_settings_held());
        lift(&self.writer);
        self.poll();
        assert!(self.hits.borrow().is_empty());
    }

    fn tap(&mut self, index: usize, target: &str, raw: Point, logical: Point) -> Hit {
        press(&self.writer, 10 + index as i32, raw);
        lift(&self.writer);
        self.poll();
        let hits = self.hits.borrow();
        assert_eq!(hits.len(), index + 1, "{target}");
        let (actual, column, row) = &hits[index];
        assert_eq!(
            (actual.as_str(), *column, *row),
            (target, logical.0 as f32, logical.1 as f32)
        );
        hits[index].clone()
    }
}

fn scenario(case: &str) {
    let mut session = Presentation::new(case == "inverted", case == "changed_before_loop");
    press(&session.writer, 7, (1626, 2404));
    if case == "changed_after_attach" {
        assert_eq!(session.frames, 0);
        session.turn_down();
        assert_eq!(session.generation, 2);
    }
    session.present(matches!(
        case,
        "changed_before_loop" | "changed_after_attach"
    ));
    assert_eq!(session.frames, 1);
    lift(&session.writer);
    session.poll();
    assert!(!session.ui.get_settings_held());
    assert_eq!(session.ui.get_cancellations(), 0);
    assert!(session.hits.borrow().is_empty());
    if case == "wake_changed" {
        session.wake_inverted();
    }
    for (index, (target, upright, inverted, logical)) in TARGETS.into_iter().enumerate() {
        let raw = if case == "upright" { upright } else { inverted };
        let (actual, column, row) = session.tap(index, target, raw, logical);
        println!("POSE_TOUCH {case} {target} {actual} {column} {row}");
    }
    session.ui.hide().unwrap();
}

#[test]
fn scribe_portrait_frames_and_touch_share_each_qualified_presentation() {
    for case in [
        "upright",
        "inverted",
        "changed_before_loop",
        "changed_after_attach",
        "wake_changed",
    ] {
        std::thread::spawn(move || scenario(case)).join().unwrap();
    }
}

#[test]
fn active_turn_cancels_the_held_control_without_replaying_its_release() {
    std::thread::spawn(|| {
        let mut session = Presentation::new(false, false);
        session.present(false);
        session.hold_settings();
        session.turn_down();
        assert!(!session.ui.get_settings_held());
        assert_eq!(session.ui.get_cancellations(), 1);
        session.present(true);
        motion(&session.writer, (234, 76));
        lift(&session.writer);
        session.poll();
        assert!(session.hits.borrow().is_empty());
        session.tap(0, "settings", (234, 76), (813, 1202));
        session.ui.hide().unwrap();
    })
    .join()
    .unwrap();
}
