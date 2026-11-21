use std::ffi::OsString;
use std::fs::{File, Metadata, OpenOptions};
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

const ROOT: &str = "/var/tmp/weread-library-frame";
const APPLICATION: &str = "/mnt/us/weread/app/weread";
const DATA: &str = "/mnt/us/weread/data";
const MAX_AGE_NS: u64 = 60_000_000_000;

#[cfg(feature = "trial-library-frame")]
#[path = "trial_library_frame.rs"]
mod trial;

enum Installation {
    Daily,
    #[cfg(feature = "trial-library-frame")]
    Trial(trial::Identity),
}

impl Installation {
    fn claim(_label: &str) -> Option<Self> {
        if ordinary_installation() {
            return Some(Self::Daily);
        }
        #[cfg(feature = "trial-library-frame")]
        if let Some(identity) = trial::Identity::current(_label) {
            return Some(Self::Trial(identity));
        }
        None
    }

    fn current(&self, _label: &str) -> bool {
        match self {
            Self::Daily => super::native_theme::owns_ordinary_first_generation(),
            #[cfg(feature = "trial-library-frame")]
            Self::Trial(claimed) => trial::Identity::current(_label).as_ref() == Some(claimed),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LibraryFrameContent {
    Entries,
    Empty,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LibraryFrameState {
    Cancelled,
    Waiting,
    Ready(LibraryFrameContent),
}

pub struct LibraryFrameRequest {
    root: PathBuf,
    directory: File,
    file: File,
    owner: u32,
    label: String,
    claimed: u64,
    deadline: u64,
    installation: Installation,
}

fn monotonic_ns() -> Option<u64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
        return None;
    }
    u64::try_from(time.tv_sec)
        .ok()?
        .checked_mul(1_000_000_000)?
        .checked_add(u64::try_from(time.tv_nsec).ok()?)
}

fn installed_arguments(args: impl Iterator<Item = OsString>, profile: &str) -> bool {
    matches!(profile, "scribe" | "pw6")
        && args.eq([
            APPLICATION,
            "--kindle",
            "--device",
            profile,
            "--data-dir",
            DATA,
        ]
        .map(OsString::from))
}

fn ordinary_installation() -> bool {
    std::env::var("WEREAD_APPROVED_PROFILE")
        .is_ok_and(|profile| installed_arguments(std::env::args_os(), &profile))
        && std::env::var_os("WEREAD_STARTUP_TIMING").is_none()
        && std::fs::read_link("/proc/self/exe").is_ok_and(|path| path == Path::new(APPLICATION))
        && super::native_theme::owns_ordinary_first_generation()
}

fn private(metadata: &Metadata, owner: u32, directory: bool) -> bool {
    metadata.uid() == owner
        && if directory {
            metadata.is_dir() && metadata.mode() & 0o7777 == 0o700
        } else {
            metadata.is_file() && metadata.nlink() == 1 && metadata.mode() & 0o7777 == 0o600
        }
}

fn same_private(path: &Path, file: &File, owner: u32, directory: bool) -> Option<Metadata> {
    let opened = file.metadata().ok()?;
    let named = std::fs::symlink_metadata(path).ok()?;
    (private(&opened, owner, directory)
        && private(&named, owner, directory)
        && opened.dev() == named.dev()
        && opened.ino() == named.ino())
    .then_some(opened)
}

fn parse_request(bytes: &[u8]) -> Option<(&str, u64)> {
    let body = std::str::from_utf8(bytes)
        .ok()?
        .strip_prefix("WEREAD_LIBRARY_REQUEST ")?
        .strip_suffix('\n')?;
    let (label, deadline) = body.split_once(' ')?;
    if label.len() != 16
        || !label
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        || deadline.starts_with('0')
        || !deadline.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    Some((label, deadline.parse().ok()?))
}

impl LibraryFrameRequest {
    pub fn claim() -> Option<Self> {
        Self::claim_at(Path::new(ROOT), 0, Installation::claim, monotonic_ns)
    }

    fn claim_at(
        root: &Path,
        owner: u32,
        ordinary: impl FnOnce(&str) -> Option<Installation>,
        clock: impl FnOnce() -> Option<u64>,
    ) -> Option<Self> {
        let directory = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(root)
            .ok()?;
        same_private(root, &directory, owner, true)?;
        let leaf = root.join("request");
        if !private(&std::fs::symlink_metadata(&leaf).ok()?, owner, false) {
            return None;
        }
        let descriptor = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                c"request".as_ptr(),
                libc::O_RDWR | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if descriptor < 0 {
            return None;
        }
        let mut file = unsafe { File::from_raw_fd(descriptor) };
        let metadata = same_private(&leaf, &file, owner, false)?;
        let mut bytes = [0; 80];
        let size = usize::try_from(metadata.len())
            .ok()
            .filter(|size| (1..=bytes.len()).contains(size))?;
        file.try_lock().ok()?;
        file.read_exact(&mut bytes[..size]).ok()?;
        let (label, deadline) = parse_request(&bytes[..size])?;
        let installation = ordinary(label)?;
        let claimed = clock()?;
        let remaining = deadline.checked_sub(claimed)?;
        if remaining == 0 || remaining > MAX_AGE_NS {
            return None;
        }
        let request = Self {
            root: root.into(),
            directory,
            file,
            owner,
            label: label.into(),
            claimed,
            deadline,
            installation,
        };
        if !request.current(metadata.len()) {
            return None;
        }
        request.file.set_len(0).ok()?;
        Some(request)
    }

    fn current(&self, length: u64) -> bool {
        same_private(&self.root, &self.directory, self.owner, true).is_some()
            && same_private(&self.root.join("request"), &self.file, self.owner, false)
                .is_some_and(|metadata| metadata.len() == length)
    }

    pub fn active(&self) -> bool {
        monotonic_ns().is_some_and(|now| (self.claimed..self.deadline).contains(&now))
            && self.current(0)
    }

    pub fn submitted(self, content: LibraryFrameContent) {
        self.finish(content, monotonic_ns, Installation::current);
    }

    fn finish(
        self,
        content: LibraryFrameContent,
        clock: impl FnOnce() -> Option<u64>,
        ordinary: impl FnOnce(&Installation, &str) -> bool,
    ) {
        let Some(now) = clock() else { return };
        if !(self.claimed..self.deadline).contains(&now)
            || !self.current(0)
            || !ordinary(&self.installation, &self.label)
        {
            return;
        }
        let scene = match content {
            LibraryFrameContent::Entries => "entries",
            LibraryFrameContent::Empty => "empty",
        };
        let record = format!("WEREAD_LIBRARY_SUBMITTED {} {now} {scene}\n", self.label);
        let _ = self.file.write_all_at(record.as_bytes(), 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{PermissionsExt, symlink};

    const LABEL: &str = "0123456789abcdef";
    const ARMED: &str = "WEREAD_LIBRARY_REQUEST 0123456789abcdef 9000\n";

    fn setup() -> (tempfile::TempDir, PathBuf, u32) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("private");
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let owner = fs::metadata(&root).unwrap().uid();
        (directory, root, owner)
    }

    fn arm(root: &Path) -> PathBuf {
        let path = root.join("request");
        fs::write(&path, ARMED).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        path
    }

    fn claim(root: &Path, owner: u32) -> Option<LibraryFrameRequest> {
        LibraryFrameRequest::claim_at(root, owner, |_| Some(Installation::Daily), || Some(100))
    }

    #[test]
    fn absent_or_unrelated_requests_never_sample_the_clock() {
        let (_directory, root, owner) = setup();
        for path in [&root, &root.join("absent")] {
            assert!(
                LibraryFrameRequest::claim_at(path, owner, |_| panic!(), || panic!()).is_none()
            );
        }
        let leaf = arm(&root);
        assert!(LibraryFrameRequest::claim_at(&root, owner, |_| None, || panic!()).is_none());
        assert_eq!(fs::read_to_string(leaf).unwrap(), ARMED);
    }

    #[test]
    fn only_a_short_canonical_public_request_can_be_armed() {
        assert_eq!(parse_request(ARMED.as_bytes()), Some((LABEL, 9000)));
        for invalid in [
            ARMED.trim_end(),
            "WEREAD_LIBRARY_REQUEST private-name 9000\n",
            "WEREAD_LIBRARY_REQUEST 0123456789abcDef 9000\n",
            "WEREAD_LIBRARY_REQUEST 0123456789abcdef 0001\n",
            "WEREAD_LIBRARY_REQUEST 0123456789abcdef -1\n",
            "WEREAD_LIBRARY_REQUEST 0123456789abcdef 9000 extra\n",
            "WEREAD_LIBRARY_REQUEST 0123456789abcdef 9000\n\n",
        ] {
            assert!(parse_request(invalid.as_bytes()).is_none());
        }
        let (_directory, root, owner) = setup();
        let leaf = arm(&root);
        for deadline in [99, 100, MAX_AGE_NS + 101] {
            fs::write(
                &leaf,
                format!("WEREAD_LIBRARY_REQUEST {LABEL} {deadline}\n"),
            )
            .unwrap();
            assert!(claim(&root, owner).is_none());
        }
        fs::write(&leaf, [b'a'; 81]).unwrap();
        assert!(LibraryFrameRequest::claim_at(&root, owner, |_| panic!(), || panic!()).is_none());
    }

    #[test]
    fn only_the_private_locked_regular_file_is_consumed_once() {
        let (directory, root, owner) = setup();
        let leaf = arm(&root);
        let lock = File::open(&leaf).unwrap();
        lock.try_lock().unwrap();
        assert!(claim(&root, owner).is_none());
        drop(lock);
        assert!(claim(&root, owner.wrapping_add(1)).is_none());
        fs::set_permissions(&root, fs::Permissions::from_mode(0o750)).unwrap();
        assert!(claim(&root, owner).is_none());
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&leaf, fs::Permissions::from_mode(0o640)).unwrap();
        assert!(claim(&root, owner).is_none());
        fs::set_permissions(&leaf, fs::Permissions::from_mode(0o600)).unwrap();
        let external = directory.path().join("other");
        fs::hard_link(&leaf, &external).unwrap();
        assert!(claim(&root, owner).is_none());
        fs::remove_file(&leaf).unwrap();
        symlink(&external, &leaf).unwrap();
        assert!(claim(&root, owner).is_none());
        assert_eq!(fs::read_to_string(&external).unwrap(), ARMED);
        fs::remove_file(&leaf).unwrap();
        fs::create_dir(&leaf).unwrap();
        assert!(claim(&root, owner).is_none());
        fs::remove_dir(&leaf).unwrap();
        let name = std::ffi::CString::new(std::os::unix::ffi::OsStrExt::as_bytes(leaf.as_os_str()))
            .unwrap();
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(claim(&root, owner).is_none());
        fs::remove_file(&leaf).unwrap();
        arm(&root);
        let alias = directory.path().join("alias");
        symlink(&root, &alias).unwrap();
        assert!(claim(&alias, owner).is_none());
        let request = claim(&root, owner).unwrap();
        assert_eq!(fs::read(&leaf).unwrap(), b"");
        assert!(claim(&root, owner).is_none());
        request.finish(LibraryFrameContent::Entries, || Some(150), |_, _| true);
        assert_eq!(
            fs::read_to_string(&leaf).unwrap(),
            format!("WEREAD_LIBRARY_SUBMITTED {LABEL} 150 entries\n")
        );
        assert!(claim(&root, owner).is_none());
    }

    #[test]
    fn expired_cancelled_replaced_or_unlinked_requests_never_report_success() {
        let (directory, root, owner) = setup();
        for now in [None, Some(99), Some(9000), Some(9001)] {
            let leaf = arm(&root);
            claim(&root, owner)
                .unwrap()
                .finish(LibraryFrameContent::Empty, || now, |_, _| true);
            assert!(fs::read(&leaf).unwrap().is_empty());
        }
        let leaf = arm(&root);
        let request = claim(&root, owner).unwrap();
        fs::write(&leaf, b"cancelled").unwrap();
        request.finish(LibraryFrameContent::Entries, || Some(110), |_, _| true);
        assert_eq!(fs::read(&leaf).unwrap(), b"cancelled");
        arm(&root);
        let request = claim(&root, owner).unwrap();
        let renamed = directory.path().join("old-request");
        fs::rename(&leaf, &renamed).unwrap();
        arm(&root);
        request.finish(LibraryFrameContent::Empty, || Some(110), |_, _| true);
        assert!(fs::read(&renamed).unwrap().is_empty());
        assert_eq!(fs::read_to_string(&leaf).unwrap(), ARMED);
        let request = claim(&root, owner).unwrap();
        fs::remove_file(&leaf).unwrap();
        request.finish(LibraryFrameContent::Empty, || Some(110), |_, _| true);
        assert!(!leaf.exists());
        arm(&root);
        claim(&root, owner)
            .unwrap()
            .finish(LibraryFrameContent::Empty, || Some(115), |_, _| false);
        assert!(fs::read(&leaf).unwrap().is_empty());
        arm(&root);
        claim(&root, owner)
            .unwrap()
            .finish(LibraryFrameContent::Empty, || Some(115), |_, _| true);
        assert_eq!(
            fs::read_to_string(leaf).unwrap(),
            format!("WEREAD_LIBRARY_SUBMITTED {LABEL} 115 empty\n")
        );
    }

    #[test]
    fn only_exact_installed_ordinary_scribe_and_pw6_arguments_qualify() {
        for profile in ["scribe", "pw6"] {
            let ordinary = [
                APPLICATION,
                "--kindle",
                "--device",
                profile,
                "--data-dir",
                DATA,
            ]
            .map(OsString::from);
            assert!(installed_arguments(ordinary.clone().into_iter(), profile));
            for extra in [
                "--screen",
                "--demo",
                "--offline",
                "--capture",
                "--actions",
                "--probe-native-portrait-calibration",
                "--exit-after-ms",
            ] {
                assert!(!installed_arguments(
                    ordinary.clone().into_iter().chain([extra.into()]),
                    profile
                ));
            }
            for index in [0, 3, 5] {
                let mut other = ordinary.clone();
                other[index] = "/temporary".into();
                assert!(!installed_arguments(other.into_iter(), profile));
            }
        }
    }
}
