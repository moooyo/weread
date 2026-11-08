#![cfg(target_os = "linux")]

use super::*;
use std::{
    fs,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug)]
struct ProcessIdentity {
    pid: u32,
    parent: u32,
    group: i32,
    birth: u64,
    state: u8,
}

fn parse_identity(stat: &str) -> Option<ProcessIdentity> {
    let pid = stat.split_once(' ')?.0.parse().ok()?;
    let fields = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    Some(ProcessIdentity {
        pid,
        parent: fields.get(1)?.parse().ok()?,
        group: fields.get(2)?.parse().ok()?,
        birth: fields.get(19)?.parse().ok()?,
        state: *fields.first()?.as_bytes().first()?,
    })
}

fn observe_identity(pid: u32) -> Option<ProcessIdentity> {
    parse_identity(&fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

fn is_same_live_process(identity: ProcessIdentity) -> bool {
    observe_identity(identity.pid).is_some_and(|current| {
        current.birth == identity.birth
            && current.group == identity.group
            && !matches!(current.state, b'Z' | b'X')
    })
}

/// A separate, still-live anchor pins the fixture group after a helper exits.
/// Its own finite TTL also closes the group if the test process is interrupted.
struct FixtureGroup {
    anchor: Option<Child>,
    identity: Option<ProcessIdentity>,
}

impl FixtureGroup {
    fn new() -> Self {
        let anchor = Command::new("/bin/sh")
            .args([
                "-c",
                r#"
                    anchor_pid=$$
                    IFS= read -r anchor_stat </proc/self/stat || exit 91
                    anchor_tail=${anchor_stat##*) }
                    set -- $anchor_tail
                    [ "$#" -ge 20 ] && [ "$3" = "$anchor_pid" ] || exit 92
                    shift 19
                    anchor_birth=$1
                    sleep 20
                    IFS= read -r anchor_stat </proc/self/stat || exit 93
                    [ "${anchor_stat%% *}" = "$anchor_pid" ] || exit 94
                    anchor_tail=${anchor_stat##*) }
                    set -- $anchor_tail
                    [ "$#" -ge 20 ] && [ "$3" = "$anchor_pid" ] || exit 95
                    shift 19
                    [ "$1" = "$anchor_birth" ] || exit 96
                    kill -KILL "-$anchor_pid"
                "#,
            ])
            .process_group(0)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Spawn the finite fixture anchor");
        let pid = anchor.id();
        let mut fixture = Self {
            anchor: Some(anchor),
            identity: None,
        };
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if let Some(identity) = observe_identity(pid) {
                // SAFETY: getpgrp has no arguments or owned resources.
                let parent_group = unsafe { libc::getpgrp() };
                assert_eq!(identity.pid, pid);
                assert_eq!(identity.parent, std::process::id());
                assert_eq!(identity.group, pid as i32);
                assert_ne!(identity.group, parent_group);
                assert!(!matches!(identity.state, b'Z' | b'X'));
                fixture.identity = Some(identity);
                return fixture;
            }
            assert!(Instant::now() < deadline, "Observe the live fixture anchor");
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn command(&self, script: &str) -> Command {
        let identity = self.identity.expect("The fixture anchor was observed");
        assert!(is_same_live_process(identity), "The fixture anchor expired");
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script]).process_group(identity.group);
        command
    }
}

impl Drop for FixtureGroup {
    fn drop(&mut self) {
        let Some(mut anchor) = self.anchor.take() else {
            return;
        };
        if let Some(identity) = self.identity {
            let owned = observe_identity(identity.pid).is_some_and(|current| {
                current.birth == identity.birth
                    && current.group == identity.group
                    && current.parent == std::process::id()
                    && current.group == current.pid as i32
                    && !matches!(current.state, b'Z' | b'X')
            });
            if owned {
                // SAFETY: the still-live, same-birth anchor proves ownership
                // of this private group and prevents numeric PGID reuse.
                unsafe { libc::kill(-identity.group, libc::SIGKILL) };
            }
        } else {
            // No helper has joined yet if construction failed. Never signal
            // an unproven group; Child still owns only the direct anchor.
            let _ = anchor.kill();
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match anchor.try_wait() {
                Ok(Some(_)) | Err(_) => break,
                Ok(None) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(2));
                }
                Ok(None) => break,
            }
        }
    }
}

fn saved_identity(path: &Path) -> ProcessIdentity {
    parse_identity(&fs::read_to_string(path).expect("Read the fixture process identity"))
        .expect("Parse the fixture process identity")
}

fn assert_reaped(identity: ProcessIdentity) {
    assert!(
        observe_identity(identity.pid).is_none_or(|current| current.birth != identity.birth),
        "The direct helper was not reaped: {identity:?}",
    );
}

fn assert_reusable(runner: &CommandRunner, group: &FixtureGroup) {
    assert_eq!(runner.occupied_slots(), 0);
    assert_eq!(
        runner
            .run_with_timeout(
                &mut group.command("printf reusable"),
                Duration::from_secs(1)
            )
            .unwrap(),
        "reusable",
    );
    assert_eq!(runner.occupied_slots(), 0);
}

#[test]
fn actual_native_entry_accepts_a_successful_empty_reply() {
    assert_eq!(run_command(Path::new("/bin/true"), &[]).unwrap(), "");
    quiesce_commands(Duration::ZERO).unwrap();
}

#[test]
fn complete_replies_accept_empty_output_and_utf8_split_across_reads() {
    let group = FixtureGroup::new();
    let runner = CommandRunner::default();
    assert_eq!(
        runner
            .run_with_timeout(&mut group.command("exit 0"), Duration::from_secs(1))
            .unwrap(),
        "",
    );
    let mut command = group.command(
        r"printf '\344'; sleep 0.02; printf '\275\240'; sleep 0.02; printf '\345\245\275'",
    );
    assert_eq!(
        runner
            .run_with_timeout(&mut command, Duration::from_secs(1))
            .unwrap(),
        "\u{4f60}\u{597d}",
    );
    assert_reusable(&runner, &group);
}

#[test]
fn output_budget_accepts_the_exact_limit_and_rejects_one_more_byte() {
    let group = FixtureGroup::new();
    let runner = CommandRunner::default();
    let mut exact = group.command(&format!("exec head -c {MAX_OUTPUT_BYTES} /dev/zero"));
    let output = runner
        .run_with_timeout(&mut exact, Duration::from_secs(1))
        .unwrap();
    assert_eq!(output.as_bytes(), vec![0_u8; MAX_OUTPUT_BYTES]);
    let mut oversized = group.command(&format!("exec head -c {} /dev/zero", MAX_OUTPUT_BYTES + 1));
    let error = runner
        .run_with_timeout(&mut oversized, Duration::from_secs(1))
        .unwrap_err();
    assert!(matches!(error, PlatformError::Operation(_)));
    assert!(error.to_string().contains("budget"));
    assert_reusable(&runner, &group);
}

#[test]
fn flooding_output_is_drained_and_rejected_before_the_operation_deadline() {
    let group = FixtureGroup::new();
    let runner = CommandRunner::default();
    let directory = tempfile::tempdir().unwrap();
    let identity_path = directory.path().join("flood-identity");
    let mut command = group.command(&format!(
        "IFS= read -r identity </proc/self/stat; printf '%s\\n' \"$identity\" >\"$WEREAD_TEST_IDENTITY\"; exec head -c {} /dev/zero",
        MAX_OUTPUT_BYTES * 256,
    ));
    command.env("WEREAD_TEST_IDENTITY", &identity_path);
    let started = Instant::now();
    let error = runner
        .run_with_timeout(&mut command, Duration::from_secs(2))
        .unwrap_err();
    assert!(error.to_string().contains("budget"));
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_reaped(saved_identity(&identity_path));
    assert_reusable(&runner, &group);
}

#[test]
fn unsuccessful_status_and_invalid_utf8_do_not_return_partial_success() {
    let group = FixtureGroup::new();
    let runner = CommandRunner::default();
    for (script, reason) in [
        ("printf partial; exit 7", "rejected"),
        (r"printf '\377'", "UTF-8"),
    ] {
        let error = runner
            .run_with_timeout(&mut group.command(script), Duration::from_secs(1))
            .unwrap_err();
        assert!(matches!(error, PlatformError::Operation(_)));
        assert!(error.to_string().contains(reason));
        assert_reusable(&runner, &group);
    }
}

#[test]
fn a_silent_timed_out_direct_child_is_killed_reaped_and_releases_its_slot() {
    let group = FixtureGroup::new();
    let runner = CommandRunner::default();
    let directory = tempfile::tempdir().unwrap();
    let identity_path = directory.path().join("silent-identity");
    let mut command = group.command(
        r#"IFS= read -r identity </proc/self/stat; printf '%s\n' "$identity" >"$WEREAD_TEST_IDENTITY"; exec sleep 5"#,
    );
    command.env("WEREAD_TEST_IDENTITY", &identity_path);
    let started = Instant::now();
    let error = runner
        .run_with_timeout(&mut command, Duration::from_millis(150))
        .unwrap_err();
    assert!(error.to_string().contains("timed out"));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_reaped(saved_identity(&identity_path));
    assert_reusable(&runner, &group);
}

#[test]
fn an_exited_helper_cannot_leave_its_inherited_stdout_read_blocked() {
    let group = FixtureGroup::new();
    let runner = CommandRunner::default();
    let directory = tempfile::tempdir().unwrap();
    let identity_path = directory.path().join("direct-identity");
    let descendant_path = directory.path().join("descendant-identity");
    let mut command = group.command(
        r#"
            IFS= read -r identity </proc/self/stat
            printf '%s\n' "$identity" >"$WEREAD_TEST_IDENTITY"
            sleep 5 &
            descendant=$!
            IFS= read -r identity <"/proc/$descendant/stat"
            printf '%s\n' "$identity" >"$WEREAD_TEST_DESCENDANT"
            printf parent-finished
            exit 0
        "#,
    );
    command.env("WEREAD_TEST_IDENTITY", &identity_path);
    command.env("WEREAD_TEST_DESCENDANT", &descendant_path);
    let started = Instant::now();
    let error = runner
        .run_with_timeout(&mut command, Duration::from_millis(150))
        .unwrap_err();
    let elapsed = started.elapsed();
    assert!(error.to_string().contains("timed out"));
    assert!(elapsed >= Duration::from_millis(100));
    assert!(elapsed < Duration::from_secs(2));
    assert_reaped(saved_identity(&identity_path));
    let descendant = saved_identity(&descendant_path);
    assert_eq!(descendant.group, group.identity.unwrap().group);
    assert!(
        is_same_live_process(descendant),
        "The descendant did not hold stdout open"
    );
    assert_reusable(&runner, &group);
    drop(group);
    let deadline = Instant::now() + Duration::from_secs(1);
    while is_same_live_process(descendant) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(!is_same_live_process(descendant));
}

#[test]
fn repeated_spawn_and_early_deadline_failures_do_not_consume_command_slots() {
    let group = FixtureGroup::new();
    let runner = CommandRunner::default();
    let directory = tempfile::tempdir().unwrap();
    let missing = directory.path().join("missing-native-helper");
    for _ in 0..MAX_COMMAND_SLOTS * 4 {
        let mut command = Command::new(&missing);
        command.process_group(group.identity.unwrap().group);
        let error = runner
            .run_with_timeout(&mut command, Duration::from_secs(1))
            .unwrap_err();
        assert!(matches!(error, PlatformError::Io(_)));
        assert_eq!(runner.occupied_slots(), 0);
    }
    let marker = directory.path().join("must-not-start");
    let mut expired = group.command(r#": >"$WEREAD_TEST_MARKER""#);
    expired.env("WEREAD_TEST_MARKER", &marker);
    assert!(
        runner
            .run_with_timeout(&mut expired, Duration::ZERO)
            .is_err()
    );
    assert!(!marker.exists());
    assert_reusable(&runner, &group);
}

#[test]
fn a_full_command_budget_rejects_new_work_without_spawning_or_queueing() {
    let group = FixtureGroup::new();
    let runner = CommandRunner::default();
    let directory = tempfile::tempdir().unwrap();
    let release = directory.path().join("release");
    let markers = (0..MAX_COMMAND_SLOTS)
        .map(|index| directory.path().join(format!("active-{index}")))
        .collect::<Vec<PathBuf>>();
    thread::scope(|scope| {
        let mut workers = Vec::new();
        for marker in &markers {
            let mut command = group.command(
                r#": >"$WEREAD_TEST_MARKER"; while [ ! -e "$WEREAD_TEST_RELEASE" ]; do sleep 0.01; done; printf released"#,
            );
            command.env("WEREAD_TEST_MARKER", marker);
            command.env("WEREAD_TEST_RELEASE", &release);
            let runner = &runner;
            workers.push(
                scope.spawn(move || runner.run_with_timeout(&mut command, Duration::from_secs(4))),
            );
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while !markers.iter().all(|path| path.exists()) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(2));
        }
        assert!(
            markers.iter().all(|path| path.exists()),
            "All admitted helpers must be live"
        );
        assert_eq!(runner.occupied_slots(), MAX_COMMAND_SLOTS);
        assert!(runner.quiesce(Duration::ZERO).is_err());
        assert_eq!(runner.occupied_slots(), MAX_COMMAND_SLOTS);
        let rejected_marker = directory.path().join("rejected-started");
        let mut rejected = group.command(r#": >"$WEREAD_TEST_MARKER"; printf unexpected"#);
        rejected.env("WEREAD_TEST_MARKER", &rejected_marker);
        let started = Instant::now();
        let error = runner
            .run_with_timeout(&mut rejected, Duration::from_secs(1))
            .unwrap_err();
        assert!(error.to_string().contains("capacity"));
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(!rejected_marker.exists());
        fs::write(&release, b"release").unwrap();
        for worker in workers {
            assert_eq!(worker.join().unwrap().unwrap(), "released");
        }
    });
    runner.quiesce(Duration::ZERO).unwrap();
    assert_reusable(&runner, &group);
}

#[test]
fn retired_children_block_quiescence_without_signals_and_share_one_wait_deadline() {
    let group = FixtureGroup::new();
    let runner = CommandRunner::default();
    runner.quiesce(Duration::ZERO).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let release = directory.path().join("release-retired");
    let descendant_path = directory.path().join("untracked-descendant");
    let identities = (0..MAX_COMMAND_SLOTS)
        .map(|index| directory.path().join(format!("retired-{index}")))
        .collect::<Vec<_>>();
    for (index, identity_path) in identities.iter().enumerate() {
        let mut command = group.command(
            r#"
                if [ "$WEREAD_TEST_CREATE_DESCENDANT" = 1 ]; then
                    sleep 5 &
                    descendant=$!
                    IFS= read -r identity <"/proc/$descendant/stat"
                    printf '%s\n' "$identity" >"$WEREAD_TEST_DESCENDANT"
                fi
                IFS= read -r identity </proc/self/stat
                printf '%s\n' "$identity" >"$WEREAD_TEST_IDENTITY"
                while [ ! -e "$WEREAD_TEST_RELEASE" ]; do sleep 0.01; done
                exit 0
            "#,
        );
        command
            .env(
                "WEREAD_TEST_CREATE_DESCENDANT",
                if index == 0 { "1" } else { "0" },
            )
            .env("WEREAD_TEST_DESCENDANT", &descendant_path)
            .env("WEREAD_TEST_IDENTITY", identity_path)
            .env("WEREAD_TEST_RELEASE", &release)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // Model retained ownership after an unsuccessful cleanup without
        // requiring a real uninterruptible kernel operation on the test host.
        let child = command.spawn().unwrap();
        runner.slots.lock().unwrap()[index] = CommandSlot::Retired(child);
    }
    let ready_deadline = Instant::now() + Duration::from_secs(2);
    let ready = || {
        identities.iter().all(|path| {
            fs::read_to_string(path)
                .ok()
                .is_some_and(|stat| parse_identity(&stat).is_some())
        })
    };
    while !ready() && Instant::now() < ready_deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(
        ready(),
        "Every retired fixture must publish a complete identity"
    );
    let originals = identities
        .iter()
        .map(|path| saved_identity(path))
        .collect::<Vec<_>>();
    let descendant = saved_identity(&descendant_path);
    assert!(
        originals
            .iter()
            .all(|identity| is_same_live_process(*identity))
    );
    assert!(is_same_live_process(descendant));
    assert_eq!(runner.occupied_slots(), MAX_COMMAND_SLOTS);

    assert!(runner.quiesce(Duration::ZERO).is_err());
    let started = Instant::now();
    let error = runner.quiesce(Duration::from_millis(120)).unwrap_err();
    assert!(matches!(error, PlatformError::Operation(_)));
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(80));
    // Applying this timeout separately to all eight slots would take 960 ms.
    assert!(elapsed < Duration::from_millis(600));
    assert_eq!(runner.occupied_slots(), MAX_COMMAND_SLOTS);
    assert!(
        originals
            .iter()
            .all(|identity| is_same_live_process(*identity))
    );
    assert!(
        is_same_live_process(descendant),
        "Quiescence signalled an untracked descendant"
    );

    fs::write(&release, b"release").unwrap();
    runner.quiesce(Duration::from_secs(1)).unwrap();
    assert_eq!(runner.occupied_slots(), 0);
    for identity in originals {
        assert_reaped(identity);
    }
    // Only the tracked direct children gate quiescence. It must not signal
    // a descendant whose ownership the production runner cannot establish.
    assert!(is_same_live_process(descendant));
    assert_reusable(&runner, &group);
    drop(group);
    let cleanup_deadline = Instant::now() + Duration::from_secs(1);
    while is_same_live_process(descendant) && Instant::now() < cleanup_deadline {
        thread::sleep(Duration::from_millis(2));
    }
    assert!(!is_same_live_process(descendant));
}

#[test]
fn quiescence_refuses_an_unknown_retired_child_status_without_replacing_its_owner() {
    let group = FixtureGroup::new();
    let runner = CommandRunner::default();
    let child = group
        .command("exit 0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        assert!(Instant::now() < deadline, "The direct fixture did not exit");
        let mut status = 0;
        // SAFETY: this test is the sole owner of its unreaped direct child.
        // Consume its status outside Child to construct a failed observation;
        // no signal is sent, and no unrelated PID can be reaped by waitpid.
        let observed = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, libc::WNOHANG) };
        if observed == pid as libc::pid_t {
            break;
        }
        if observed < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted
        {
            continue;
        }
        assert_eq!(observed, 0, "Reap only the fixture's direct child");
        thread::sleep(Duration::from_millis(2));
    }
    runner.slots.lock().unwrap()[0] = CommandSlot::Retired(child);
    assert!(runner.quiesce(Duration::ZERO).is_err());
    assert_eq!(runner.occupied_slots(), 1);
    assert!(runner.quiesce(Duration::from_millis(30)).is_err());
    assert_eq!(runner.occupied_slots(), 1);
    assert!(is_same_live_process(group.identity.unwrap()));
    // The test already reaped this child explicitly. Clearing this constructed
    // fixture does not establish a production permission to discard unknowns.
    runner.slots.lock().unwrap()[0] = CommandSlot::Free;
    runner.quiesce(Duration::ZERO).unwrap();
    assert_reusable(&runner, &group);
}
