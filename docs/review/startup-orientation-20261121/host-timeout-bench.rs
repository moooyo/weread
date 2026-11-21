use super::*;
use crate::orientation::NativePortraitOrientation::{Down, Up};
use crate::orientation::PortraitCalibration;
use std::hint::black_box;

#[derive(Clone, Copy, PartialEq)]
enum Lane {
    Control,
    Match,
    Changed,
}

fn sample(label: &str, lane: Lane, silent: bool) {
    let launch = OrientationRequest {
        sequence: 1,
        calibration: Some(PortraitCalibration {
            holding: Up,
            scanout: Up,
            raw_rotation: 3,
        }),
    };
    let snapshot = || Some((black_box(if lane == Lane::Changed { Down } else { Up }), ()));
    let rotation = || Ok(black_box(3));
    let mut queries = 0;
    let mut timeouts = 0;
    let mut legacy = || {
        queries += 1;
        let (program, argument) = if silent {
            ("/bin/sleep", "5")
        } else {
            ("/bin/echo", "U")
        };
        let result = query_render_offset(Command::new(program).arg(argument), ORIENTATION_TIMEOUT);
        match result {
            Err(OrientationQueryError::TimedOut) if silent => timeouts += 1,
            Ok(0) if !silent => {}
            _ => panic!("Unexpected fixed-child outcome: {result:?}"),
        }
        result
    };
    let start = Instant::now();
    let splash = resolve_launch_orientation(
        "splash",
        || {
            if lane == Lane::Control {
                return None;
            }
            let (mapping, ()) = confirmed_launch_mapping(Some((Up, 3)), rotation, snapshot)?;
            Some(mapping.render_offset())
        },
        &mut legacy,
        std::convert::identity,
    );
    let after_splash = Instant::now();
    let controller = resolve_launch_orientation_path(
        "event_loop",
        || {
            if lane == Lane::Control {
                return None;
            }
            let (controller, (), path) = confirmed_event_loop_launch(
                launch,
                || Some(black_box(launch)),
                rotation,
                snapshot,
                |_| panic!("the historical exact-only run must not resample the pose"),
            )?;
            Some((controller, path))
        },
        &mut legacy,
        PortraitController::new,
    );
    let end = Instant::now();
    assert_eq!(splash, 0);
    assert_eq!(controller.render_offset(), 0);
    assert_eq!(controller.touch_offset(), 0);
    assert_eq!(controller.calibrated(), lane == Lane::Match);
    assert_eq!(queries, if lane == Lane::Match { 0 } else { 2 });
    assert_eq!(timeouts, if silent { queries } else { 0 });
    println!(
        "ORIENTATION_HOST,{label},{queries},{timeouts},{},{},{}",
        after_splash.duration_since(start).as_nanos(),
        end.duration_since(after_splash).as_nanos(),
        end.duration_since(start).as_nanos(),
    );
}

#[test]
#[ignore = "Explicit host-only two-site timeout experiment"]
fn controlled_two_site_timeouts() {
    assert_ne!(
        std::env::var("WEREAD_STARTUP_TIMING").ok().as_deref(),
        Some("1")
    );
    println!("ORIENTATION_HOST,case,legacy_queries,timeouts,splash_ns,event_loop_ns,total_ns");
    for pair in 0..=6 {
        let order = if pair == 0 || pair % 2 == 1 {
            ["A", "B"]
        } else {
            ["B", "A"]
        };
        for arm in order {
            let lane = if arm == "A" {
                Lane::Control
            } else {
                Lane::Match
            };
            sample(&format!("{arm}{pair}"), lane, true);
        }
    }
    sample("changed-silent", Lane::Changed, true);
    sample("control-responsive", Lane::Control, false);
    sample("match-responsive", Lane::Match, false);
}
