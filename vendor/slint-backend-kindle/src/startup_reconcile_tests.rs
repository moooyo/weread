use super::*;
use crate::orientation::NativePortraitOrientation::{Down, Up};
use crate::orientation::PortraitCalibration;
use std::cell::Cell;

fn initial(holding: NativePortraitOrientation, raw: u32) -> OrientationRequest {
    OrientationRequest {
        sequence: 1,
        calibration: Some(PortraitCalibration {
            holding,
            scanout: holding,
            raw_rotation: raw,
        }),
    }
}

#[test]
fn only_a_quiet_opposite_pose_adds_a_second_sample_and_a_third_scanout_read() {
    for anchor in 0..=3 {
        for (start, opposite) in [(Up, Down), (Down, Up)] {
            let request = initial(start, anchor);
            for pose in [start, opposite] {
                let events = RefCell::new(Vec::new());
                let owner = Rc::new(());
                let (mut controller, observer, path) = confirmed_event_loop_launch(
                    request,
                    || {
                        events.borrow_mut().push("request");
                        Some(request)
                    },
                    || {
                        events.borrow_mut().push("scanout");
                        Ok(anchor)
                    },
                    || {
                        events.borrow_mut().push("snapshot");
                        Some((pose, owner.clone()))
                    },
                    |observer| {
                        assert!(Rc::ptr_eq(observer, &owner));
                        events.borrow_mut().push("confirm");
                        Some(pose)
                    },
                )
                .unwrap();
                assert!(Rc::ptr_eq(&owner, &observer));
                let expected: &[&str] = if pose == start {
                    assert_eq!(path, LaunchOrientationPath::Reused);
                    &["scanout", "snapshot", "scanout", "request"]
                } else {
                    assert_eq!(path, LaunchOrientationPath::Reconciled);
                    &[
                        "scanout", "snapshot", "scanout", "confirm", "scanout", "request",
                    ]
                };
                assert_eq!(*events.borrow(), expected);
                assert!(!controller.apply_request(request, anchor).unwrap());
                assert_eq!(
                    controller.render_offset(),
                    if pose == start { 0 } else { 180 }
                );
                assert_eq!(controller.touch_offset(), pose.offset());
                assert!(controller.observe(pose, (anchor + 2) % 4).unwrap());
                assert_eq!(
                    controller.render_offset(),
                    if pose == start { 180 } else { 0 }
                );
                assert_eq!(controller.touch_offset(), pose.offset());
            }
        }
    }
}

#[test]
fn a_failed_witness_discards_its_observer_and_runs_the_legacy_query_once() {
    let launch = initial(Down, 1);
    let wake = OrientationRequest {
        sequence: 2,
        ..launch
    };
    let malformed = OrientationRequest {
        calibration: Some(PortraitCalibration {
            holding: Up,
            ..launch.calibration.unwrap()
        }),
        ..launch
    };
    for (request, current, raw, first, confirm, reads) in [
        (
            OrientationRequest::default(),
            Some(launch),
            [1, 1, 1],
            Some(Up),
            Some(Up),
            0,
        ),
        (wake, Some(wake), [1, 1, 1], Some(Up), Some(Up), 0),
        (malformed, Some(malformed), [1, 1, 1], Some(Up), Some(Up), 0),
        (initial(Down, 4), None, [4, 4, 4], Some(Up), Some(Up), 0),
        (launch, Some(launch), [-1, 1, 1], Some(Up), Some(Up), 1),
        (launch, Some(launch), [3, 3, 3], Some(Up), Some(Up), 1),
        (launch, Some(launch), [1, 1, 1], None, Some(Up), 1),
        (launch, Some(launch), [1, -1, 1], Some(Up), Some(Up), 2),
        (launch, Some(launch), [1, 3, 1], Some(Up), Some(Up), 2),
        (launch, Some(launch), [1, 1, 1], Some(Up), None, 2),
        (launch, Some(launch), [1, 1, 1], Some(Up), Some(Down), 2),
        (launch, Some(launch), [1, 1, -1], Some(Up), Some(Up), 3),
        (launch, Some(launch), [1, 1, 2], Some(Up), Some(Up), 3),
        (launch, Some(launch), [1, 1, 3], Some(Up), Some(Up), 3),
        (launch, None, [1, 1, 1], Some(Up), Some(Up), 3),
        (launch, Some(wake), [1, 1, 1], Some(Up), Some(Up), 3),
        (
            launch,
            Some(initial(Up, 1)),
            [1, 1, 1],
            Some(Up),
            Some(Up),
            3,
        ),
    ] {
        let owner = Rc::new(());
        let queries = Cell::new(0);
        let mut scans = raw.into_iter();
        let controller = resolve_launch_orientation_path(
            "event_loop",
            || {
                confirmed_event_loop_launch(
                    request,
                    || current,
                    || u32::try_from(scans.next().unwrap()).map_err(io::Error::other),
                    || Some((first?, owner.clone())),
                    |_| confirm,
                )
                .map(|(controller, _, path)| (controller, path))
            },
            || {
                assert_eq!(Rc::strong_count(&owner), 1);
                queries.set(queries.get() + 1);
                Err(OrientationQueryError::TimedOut)
            },
            PortraitController::new,
        );
        assert_eq!(queries.get(), 1);
        assert_eq!(3 - scans.len(), reads);
        assert!(!controller.calibrated());
        assert_eq!(
            (controller.render_offset(), controller.touch_offset()),
            (0, 0)
        );
    }
}
