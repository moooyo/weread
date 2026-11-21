use super::*;
use crate::orientation::NativePortraitOrientation::{Down, Up};
use crate::orientation::PortraitCalibration;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::net::UnixStream;

fn other(holding: NativePortraitOrientation) -> NativePortraitOrientation {
    match holding {
        Up => Down,
        Down => Up,
    }
}

#[test]
#[ignore = "Explicit Linux exact/reconcile experiment"]
fn one_decision() {
    let case = std::env::var("WEREAD_RECONCILE_CASE").unwrap();
    let silent = std::env::var("WEREAD_RECONCILE_CHILD").as_deref() == Ok("silent");
    let anchor = if matches!(case.as_str(), "stable_up" | "up_to_down") { Up } else { Down };
    let raw_at_launch = if anchor == Down { 1 } else { 3 };
    let live = if case.starts_with("stable_") { anchor } else { other(anchor) };
    let calibration = PortraitCalibration { holding: anchor, scanout: anchor, raw_rotation: raw_at_launch };
    let launch = OrientationRequest {
        sequence: if case == "wake_sequence" { 2 } else { 1 },
        calibration: (case != "no_anchor").then_some(if case == "unsupported_anchor" {
            PortraitCalibration { scanout: other(anchor), ..calibration }
        } else { calibration }),
    };
    let (observer, _peer) = UnixStream::pair().unwrap();
    let observer = OwnedFd::from(observer);
    let sampled_fd = observer.as_raw_fd();
    let mut raw_reads = 0;
    let mut first_reads = 0;
    #[cfg(not(published_exact))]
    let mut confirm_reads = 0;
    #[cfg(published_exact)]
    let confirm_reads = 0;
    let mut current_reads = 0;
    let mut queries = 0;
    let mut timeouts = 0;
    let mut rotation = || {
        raw_reads += 1;
        match (case.as_str(), raw_reads) {
            ("raw_changed_mid", 2) | ("raw_changed_final", 3) => Ok((raw_at_launch + 2) % 4),
            ("raw_unreadable", 1) | ("raw_lost_final", 3) => Err(io::ErrorKind::WouldBlock.into()),
            _ => Ok(raw_at_launch),
        }
    };
    let snapshot = || {
        first_reads += 1;
        (case != "first_unknown").then_some((live, observer))
    };
    let current = || {
        current_reads += 1;
        match case.as_str() {
            "request_lost" => None,
            "request_advanced" | "stable_advanced" => Some(OrientationRequest { sequence: 2, ..launch }),
            "record_changed" => Some(OrientationRequest {
                calibration: Some(PortraitCalibration { raw_rotation: 3, ..calibration }),
                ..launch
            }),
            _ => Some(launch),
        }
    };
    #[cfg(not(published_exact))]
    let confirm = |held: &OwnedFd| {
        assert_eq!(held.as_raw_fd(), sampled_fd);
        confirm_reads += 1;
        match case.as_str() {
            "second_changed" => Some(other(live)),
            "second_unknown" => None,
            _ => Some(live),
        }
    };
    let mut query = || {
        queries += 1;
        let (program, argument) = if silent { ("/bin/sleep", "5") } else { ("/bin/echo", "U") };
        let result = query_render_offset(Command::new(program).arg(argument), ORIENTATION_TIMEOUT);
        match result {
            Err(OrientationQueryError::TimedOut) if silent => timeouts += 1,
            Ok(0) if !silent => (),
            other => panic!("Unexpected child result: {other:?}"),
        }
        result
    };
    let legacy = |offset| (PortraitController::new(offset), None, if silent { "timeout" } else { "success" });
    let start = Instant::now();
    #[cfg(published_exact)]
    let (controller, retained, path) = resolve_launch_orientation(
        "event_loop",
        || confirmed_launch_controller(launch, current, &mut rotation, snapshot)
            .map(|(controller, held)| (controller, Some(held), "reused")),
        &mut query,
        legacy,
    );
    #[cfg(not(published_exact))]
    let (controller, retained, path) = resolve_launch_orientation_path(
        "event_loop",
        || confirmed_event_loop_launch(launch, current, &mut rotation, snapshot, confirm)
            .map(|(controller, held, choice)| ((controller, Some(held), choice.phase()), choice)),
        &mut query,
        legacy,
    );
    let elapsed = start.elapsed().as_nanos();
    let render = controller.render_offset();
    let touch = controller.touch_offset();
    let kept = retained.is_some();
    if let Some(held) = retained {
        assert_eq!(held.as_raw_fd(), sampled_fd);
        assert!(controller.calibrated());
        assert_eq!((render, touch), ((live.offset() + 360 - anchor.offset()) % 360, live.offset()));
    } else {
        assert!(!controller.calibrated());
        assert_eq!((render, touch), (0, 0));
    }
    println!("RECON_HOST {path} {queries} {timeouts} {raw_reads} {first_reads} {confirm_reads} {current_reads} {render} {touch} {kept} {elapsed}");
}
