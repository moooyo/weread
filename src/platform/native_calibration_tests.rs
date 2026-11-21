use super::*;
use std::{cell::RefCell, fs, os::unix::fs::PermissionsExt, rc::Rc};

fn helper(mode: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("query");
    let source = format!(
        r#"#!/usr/bin/python3
import os, struct, sys, time
mode = {mode}
with open({pid_path}, 'w') as stream:
    stream.write(str(os.getpid()))
if mode == 'unavailable': sys.exit(69)
if mode == 'wrong_magic':
    sys.stdout.buffer.write(b'BAD\n'); sys.stdout.buffer.flush(); time.sleep(10)
if mode == 'no_handshake': time.sleep(10)
sys.stdout.buffer.write(b'WQ1\n'); sys.stdout.buffer.flush()
while True:
    request = sys.stdin.buffer.read(1)
    if not request: break
    opcode = request[0]
    value = b'active' if opcode == 1 else b'U'
    if mode == 'hang_query': time.sleep(10)
    if mode == 'invalid_utf8': value = b'\xff'
    if mode == 'inactive': value = b'screenSaver'
    status = 1 if mode == 'rejected' else 0
    response_opcode = 9 if mode == 'wrong_opcode' else opcode
    length = 129 if mode == 'oversized' else len(value)
    frame = bytes([response_opcode, status]) + struct.pack('>H', length) + value
    if mode == 'truncated':
        sys.stdout.buffer.write(frame[:3]); sys.stdout.buffer.flush(); sys.exit(0)
    for byte in frame:
        sys.stdout.buffer.write(bytes([byte])); sys.stdout.buffer.flush()
if mode == 'trailing': sys.stdout.buffer.write(b'extra'); sys.stdout.buffer.flush()
sys.exit(1 if mode == 'failed_exit' else 0)
"#,
        mode = serde_json::to_string(mode).unwrap(),
        pid_path = serde_json::to_string(&directory.path().join("pid")).unwrap()
    );
    fs::write(&path, source).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    (directory, path)
}

fn assert_reaped(directory: &Path) {
    let pid: i32 = fs::read_to_string(directory.join("pid"))
        .unwrap()
        .parse()
        .unwrap();
    // SAFETY: Signal zero only inspects the recorded fixture PID.
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    native_command::quiesce_commands(Duration::ZERO).unwrap();
}

#[test]
fn stable_calibration_preserves_interleaved_fresh_observations() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let query_calls = calls.clone();
    let framebuffer_calls = calls.clone();
    let result = calibrate(
        DeviceProfile::Scribe,
        move |property| {
            query_calls.borrow_mut().push(match property {
                Property::PowerState => "power",
                Property::Direction => "direction",
            });
            Ok(match property {
                Property::PowerState => "active",
                Property::Direction => "L",
            }
            .into())
        },
        move || {
            framebuffer_calls.borrow_mut().push("framebuffer");
            Ok((true, 3))
        },
    )
    .unwrap();
    assert_eq!(result, "U:3");
    assert_eq!(
        *calls.borrow(),
        [
            "power",
            "framebuffer",
            "direction",
            "framebuffer",
            "direction",
            "power"
        ]
    );
}

#[test]
fn fixed_displays_skip_direction_queries_but_recheck_power_and_framebuffer() {
    let mut powers = 0;
    let mut frames = 0;
    let result = calibrate(
        DeviceProfile::Paperwhite6,
        |property| {
            assert_eq!(property, Property::PowerState);
            powers += 1;
            Ok("active".into())
        },
        || {
            frames += 1;
            Ok((false, 0))
        },
    )
    .unwrap();
    assert_eq!((result.as_str(), powers, frames), ("FIXED:0", 2, 2));
}

#[test]
fn changed_geometry_or_direction_never_publishes_a_pair() {
    for changed in [(true, 2), (false, 3)] {
        let mut frames = [(true, 3), changed].into_iter();
        assert!(
            calibrate(
                DeviceProfile::Scribe,
                |property| Ok(if property == Property::PowerState {
                    "active"
                } else {
                    "U"
                }
                .into()),
                || Ok(frames.next().unwrap())
            )
            .is_err()
        );
    }
    let mut directions = ["U", "D"].into_iter();
    assert!(
        calibrate(
            DeviceProfile::Scribe,
            |property| Ok(if property == Property::PowerState {
                "active"
            } else {
                directions.next().unwrap()
            }
            .into()),
            || Ok((true, 3))
        )
        .is_err()
    );
    assert!(
        calibrate(
            DeviceProfile::Scribe,
            |_| Ok("active".into()),
            || Ok((true, 4))
        )
        .is_err()
    );
}

#[test]
fn inactive_power_and_unreviewed_directions_are_rejected() {
    assert!(
        calibrate(
            DeviceProfile::Scribe,
            |_| Ok("screenSaver".into()),
            || panic!("No framebuffer read before active power")
        )
        .is_err()
    );
    let mut reads = 0;
    assert!(
        calibrate(
            DeviceProfile::Scribe,
            |property| {
                if property == Property::PowerState {
                    reads += 1;
                    Ok(if reads == 1 { "active" } else { "screenSaver" }.into())
                } else {
                    Ok("U".into())
                }
            },
            || Ok((true, 0))
        )
        .is_err()
    );
    for profile in [
        DeviceProfile::Paperwhite5,
        DeviceProfile::Paperwhite6,
        DeviceProfile::Scribe2025,
    ] {
        assert!(direction("L", profile).is_err());
        assert!(direction("R", profile).is_err());
    }
    for value in ["", " U", "U\n", "UD", "unknown"] {
        assert!(direction(value, DeviceProfile::Scribe).is_err());
    }
}

#[test]
fn unavailable_helpers_never_read_framebuffer_and_are_reaped() {
    let _registry = native_command::exclusive_registry_fixture();
    let directory = tempfile::tempdir().unwrap();
    assert!(
        query_native_portrait_calibration(
            &directory.path().join("missing"),
            DeviceProfile::Scribe,
            || panic!("No unavailable-helper sample")
        )
        .unwrap()
        .is_none()
    );
    let invalid = directory.path().join("wrong-abi");
    fs::write(&invalid, b"not an executable format").unwrap();
    fs::set_permissions(&invalid, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        query_native_portrait_calibration(&invalid, DeviceProfile::Scribe, || panic!(
            "No invalid-ABI sample"
        ))
        .unwrap()
        .is_none()
    );
    for mode in ["unavailable", "wrong_magic", "no_handshake"] {
        let (directory, path) = helper(mode);
        assert!(
            query_native_portrait_calibration(&path, DeviceProfile::Scribe, || panic!(
                "No incomplete-handshake sample"
            ))
            .unwrap()
            .is_none(),
            "{mode}"
        );
        assert_reaped(directory.path());
    }
}

#[test]
fn fragmented_complete_session_requires_successful_exit_and_eof() {
    let _registry = native_command::exclusive_registry_fixture();
    let (directory, path) = helper("valid");
    assert_eq!(
        query_native_portrait_calibration(&path, DeviceProfile::Scribe, || Ok((true, 3)))
            .unwrap()
            .as_deref(),
        Some("U:3")
    );
    assert_reaped(directory.path());
}

#[test]
fn connected_protocol_failures_never_become_legacy_availability_fallbacks() {
    let _registry = native_command::exclusive_registry_fixture();
    for mode in [
        "wrong_opcode",
        "rejected",
        "oversized",
        "truncated",
        "invalid_utf8",
        "inactive",
        "trailing",
        "failed_exit",
        "hang_query",
    ] {
        let (directory, path) = helper(mode);
        let error =
            query_native_portrait_calibration(&path, DeviceProfile::Scribe, || Ok((true, 3)))
                .expect_err(mode);
        assert!(
            directory.path().join("pid").is_file(),
            "{mode}: fixture did not publish its PID before returning {error:?}"
        );
        assert_reaped(directory.path());
    }
}

#[test]
fn a_connected_framebuffer_failure_does_not_trigger_legacy_fallback() {
    let _registry = native_command::exclusive_registry_fixture();
    let (directory, path) = helper("valid");
    assert!(
        query_native_portrait_calibration(&path, DeviceProfile::Scribe, || Err(
            io::Error::from_raw_os_error(libc::ENOENT).into()
        ))
        .is_err()
    );
    assert_reaped(directory.path());
}
