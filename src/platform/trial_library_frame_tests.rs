use super::*;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

const LABEL: &str = "0123456789abcdef";

struct Rig {
    _directory: tempfile::TempDir,
    volume: PathBuf,
    proc_root: PathBuf,
    shell: PathBuf,
    lock: PathBuf,
    stage: PathBuf,
    home: PathBuf,
    gui: Vec<OsString>,
}

impl Rig {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let volume = directory.path().join("mnt/us");
        let stage = volume.join(format!(".weread-render-{LABEL}"));
        let home = stage.join("arm");
        let exe = home.join("app/weread");
        let data = home.join("data");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::create_dir(&data).unwrap();
        for path in [
            exe.clone(),
            home.join("launch.sh"),
            data.join("library.sqlite"),
        ] {
            fs::write(path, "public fixture").unwrap();
        }
        for name in [
            "owner",
            "selector-live",
            "selector-entered",
            "tile-dispatch-intent",
        ] {
            fs::write(stage.join(name), format!("{LABEL}\n")).unwrap();
        }
        fs::write(stage.join("arm-name"), "A\n").unwrap();
        let lock = directory.path().join("lock");
        fs::create_dir(&lock).unwrap();
        fs::write(lock.join("pid"), "302\n").unwrap();
        let shell = directory.path().join("system-sh");
        fs::write(&shell, "system shell").unwrap();
        let proc_root = directory.path().join("proc");
        let gui = vec![
            exe.clone().into_os_string(),
            "--kindle".into(),
            "--device".into(),
            "scribe".into(),
            "--data-dir".into(),
            data.into_os_string(),
        ];
        for id in 301..=304 {
            let base = proc_root.join(id.to_string());
            fs::create_dir_all(&base).unwrap();
            let parent = if id == 301 { 7 } else { id - 1 };
            fs::write(
                base.join("stat"),
                format!("{id} (worker ) name) S {parent} {}{id}\n", "0 ".repeat(17)),
            )
            .unwrap();
            fs::write(base.join("status"), "Uid:\t0 0 0 0\n").unwrap();
            symlink(if id == 304 { &exe } else { &shell }, base.join("exe")).unwrap();
            let args: &[&str] = if id == 301 {
                &["--library"]
            } else {
                &["--detached", "--managed-logs"]
            };
            let mut command = format!("sh\0{}\0", home.join("launch.sh").display());
            for arg in args {
                command.push_str(arg);
                command.push('\0');
            }
            fs::write(base.join("cmdline"), command).unwrap();
        }
        Self {
            _directory: directory,
            volume,
            proc_root,
            shell,
            lock,
            stage,
            home,
            gui,
        }
    }

    fn read(&self) -> Option<Identity> {
        self.read_as(LABEL, self.home.as_os_str(), self.gui.clone())
    }

    fn read_as(&self, label: &str, home: &OsStr, args: Vec<OsString>) -> Option<Identity> {
        Identity::read(
            &self.volume,
            &self.proc_root,
            &self.shell,
            &self.lock,
            304,
            label,
            home,
            args.into_iter(),
        )
    }
}

#[test]
fn both_arms_require_the_tagged_normal_store_and_exact_stock_entry_tree() {
    let rig = Rig::new();
    let first = rig.read().unwrap();
    fs::write(rig.stage.join("arm-name"), b"B\n").unwrap();
    assert_ne!(rig.read().unwrap(), first);
    fs::write(
        rig.proc_root.join("302/cmdline"),
        format!(
            "/bin/sh\0{}\0--library-catalog-worker\0",
            rig.home.join("launch.sh").display()
        ),
    )
    .unwrap();
    assert!(rig.read().is_some());
    assert!(
        rig.read_as("fedcba9876543210", rig.home.as_os_str(), rig.gui.clone())
            .is_none()
    );
    assert!(
        rig.read_as(LABEL, OsStr::new("/mnt/us/weread"), rig.gui.clone())
            .is_none()
    );
    for extra in ["--demo", "--offline", "--actions", "--screen", "--capture"] {
        let mut argv = rig.gui.clone();
        argv.push(extra.into());
        assert!(rig.read_as(LABEL, rig.home.as_os_str(), argv).is_none());
    }
    for (index, wrong) in [
        (0, "/mnt/us/weread/app/weread"),
        (3, "pw6"),
        (5, "/mnt/us/weread/data"),
    ] {
        let mut argv = rig.gui.clone();
        argv[index] = wrong.into();
        assert!(rig.read_as(LABEL, rig.home.as_os_str(), argv).is_none());
    }
    fs::write(
        rig.proc_root.join("301/cmdline"),
        format!(
            "sh\0{}\0--detached\0--managed-logs\0",
            rig.home.join("launch.sh").display()
        ),
    )
    .unwrap();
    assert!(rig.read().is_none());
}

#[test]
fn changed_files_aliases_identities_and_private_paths_cannot_carry_a_claim() {
    let rig = Rig::new();
    let claimed = rig.read().unwrap();
    let entered = rig.stage.join("selector-entered");
    fs::rename(&entered, rig.stage.join("retired")).unwrap();
    fs::write(&entered, format!("{LABEL}\n")).unwrap();
    assert_ne!(rig.read().unwrap(), claimed);
    fs::write(rig.stage.join("owner"), "fedcba9876543210\n").unwrap();
    assert!(rig.read().is_none());
    let rig = Rig::new();
    let data = rig.home.join("data");
    fs::rename(&data, rig.volume.join("private-data")).unwrap();
    symlink(rig.volume.join("private-data"), &data).unwrap();
    assert!(rig.read().is_none());
    let rig = Rig::new();
    let leaf = rig.home.join("data/library.sqlite");
    fs::hard_link(&leaf, rig.volume.join("private-db")).unwrap();
    assert!(rig.read().is_none());
    let rig = Rig::new();
    fs::write(rig.lock.join("pid"), "301\n").unwrap();
    assert!(rig.read().is_none());
    fs::write(rig.lock.join("pid"), "302\n").unwrap();
    let real = rig.read().unwrap();
    fs::write(
        rig.proc_root.join("304/stat"),
        format!("304 (fixture) R 303 {}800\n", "0 ".repeat(17)),
    )
    .unwrap();
    assert_ne!(rig.read().unwrap(), real);
    fs::write(rig.proc_root.join("303/status"), "Uid:\t0 0 1 0\n").unwrap();
    assert!(rig.read().is_none());
}

#[test]
fn submission_requires_the_same_armed_stage_and_the_original_request_descriptor() {
    use super::super::{Installation, LibraryFrameContent, LibraryFrameRequest};
    use std::os::unix::fs::PermissionsExt;

    for rebound in [false, true] {
        let rig = Rig::new();
        let root = rig._directory.path().join("private-request");
        let file = root.join("request");
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(&file, format!("WEREAD_LIBRARY_REQUEST {LABEL} 9000\n")).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        let owner = fs::metadata(&root).unwrap().uid();
        let request = LibraryFrameRequest::claim_at(
            &root,
            owner,
            |label| {
                assert_eq!(label, LABEL);
                Some(Installation::Trial(rig.read()?))
            },
            || Some(100),
        )
        .unwrap();
        assert!(fs::read(&file).unwrap().is_empty());
        if rebound {
            fs::write(rig.stage.join("arm-name"), "B\n").unwrap();
        }
        request.finish(
            LibraryFrameContent::Entries,
            || Some(150),
            |installation, label| {
                assert_eq!(label, LABEL);
                matches!(installation, Installation::Trial(pinned) if rig.read().as_ref() == Some(pinned))
            },
        );
        let expected = if rebound {
            String::new()
        } else {
            format!("WEREAD_LIBRARY_SUBMITTED {LABEL} 150 entries\n")
        };
        assert_eq!(fs::read_to_string(file).unwrap(), expected);
    }
}
