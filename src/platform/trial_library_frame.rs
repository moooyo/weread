use std::ffi::{OsStr, OsString};
use std::fs::{self, Metadata, OpenOptions};
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

#[derive(Debug, Eq, PartialEq)]
struct Node(u64, u64);

impl Node {
    fn from(metadata: Metadata) -> Self {
        Self(metadata.dev(), metadata.ino())
    }
}

#[cfg(test)]
#[path = "trial_library_frame_tests.rs"]
mod tests;

fn node(path: &Path, directory: bool) -> Option<Node> {
    let metadata = fs::symlink_metadata(path).ok()?;
    if directory {
        metadata.is_dir().then(|| Node::from(metadata))
    } else {
        (metadata.is_file() && metadata.nlink() == 1).then(|| Node::from(metadata))
    }
}

fn read_small(path: &Path, limit: usize) -> Option<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() <= limit).then_some(bytes)
}

fn tag(path: &Path, expected: &[u8]) -> Option<Node> {
    let before = node(path, false)?;
    (read_small(path, expected.len())? == expected && before == node(path, false)?)
        .then_some(before)
}

#[derive(Debug, Eq, PartialEq)]
struct Process {
    id: u32,
    parent: u32,
    birth: u64,
}

fn process(proc_root: &Path, id: u32) -> Option<Process> {
    let dir = proc_root.join(id.to_string());
    let bytes = read_small(&dir.join("stat"), 2048)?;
    let stat = std::str::from_utf8(&bytes).ok()?;
    let (name, fields) = stat.rsplit_once(") ")?;
    if name.split_once(" (")?.0 != id.to_string() {
        return None;
    }
    let mut fields = fields.split_ascii_whitespace();
    if !matches!(fields.next()?, "R" | "S" | "D" | "I") {
        return None;
    }
    let parent = fields.next()?.parse().ok()?;
    let birth = fields.nth(17)?.parse().ok()?;
    let bytes = read_small(&dir.join("status"), 64 * 1024)?;
    let mut rows = std::str::from_utf8(&bytes)
        .ok()?
        .lines()
        .filter_map(|line| line.strip_prefix("Uid:"));
    let mut uids = rows.next()?.split_ascii_whitespace();
    (parent != 0
        && birth != 0
        && rows.next().is_none()
        && uids.by_ref().take(4).eq(["0"; 4])
        && uids.next().is_none())
    .then_some(Process { id, parent, birth })
}

fn shell_args(proc_root: &Path, id: u32, system_shell: &Node, launch: &Path, entry: bool) -> bool {
    let dir = proc_root.join(id.to_string());
    let Some(args) = read_small(&dir.join("cmdline"), 512) else {
        return false;
    };
    if !fs::metadata(dir.join("exe"))
        .is_ok_and(|meta| meta.is_file() && Node::from(meta) == *system_shell)
    {
        return false;
    }
    let Some(bytes) = args.strip_suffix(&[0]) else {
        return false;
    };
    let mut args = bytes.split(|byte| *byte == 0);
    if !matches!(args.next(), Some(b"sh" | b"/bin/sh"))
        || args.next() != Some(launch.as_os_str().as_bytes())
    {
        return false;
    }
    if entry {
        args.eq([b"--library".as_slice()])
    } else {
        args.clone().eq([b"--library-catalog-worker".as_slice()])
            || args.eq([b"--detached".as_slice(), b"--managed-logs".as_slice()])
    }
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct Identity {
    nodes: Vec<Node>,
    arm: u8,
    processes: [Process; 4],
    lock: Node,
}

impl Identity {
    pub(super) fn current(label: &str) -> Option<Self> {
        if std::env::var_os("WEREAD_APPROVED_PROFILE").as_deref() != Some(OsStr::new("scribe"))
            || std::env::var_os("WEREAD_STARTUP_TIMING").is_some()
            || !super::super::native_theme::owns_ordinary_first_generation()
        {
            return None;
        }
        Self::read(
            Path::new("/mnt/us"),
            Path::new("/proc"),
            Path::new("/bin/sh"),
            Path::new("/tmp/weread-launcher.lock"),
            std::process::id(),
            label,
            &std::env::var_os("WEREAD_HOME")?,
            std::env::args_os(),
        )
    }

    fn read(
        volume: &Path,
        proc_root: &Path,
        system_shell: &Path,
        lock: &Path,
        pid: u32,
        label: &str,
        home: &OsStr,
        args: impl Iterator<Item = OsString>,
    ) -> Option<Self> {
        if label.len() != 16
            || !label
                .bytes()
                .all(|ch| matches!(ch, b'0'..=b'9' | b'a'..=b'f'))
        {
            return None;
        }
        let stage = volume.join(format!(".weread-render-{label}"));
        let arm = stage.join("arm");
        let data = arm.join("data");
        let app = arm.join("app");
        let exe = app.join("weread");
        let launch = arm.join("launch.sh");
        if home != arm.as_os_str()
            || !args.eq([
                exe.as_os_str(),
                OsStr::new("--kindle"),
                OsStr::new("--device"),
                OsStr::new("scribe"),
                OsStr::new("--data-dir"),
                data.as_os_str(),
            ]
            .map(OsString::from))
            || fs::canonicalize(&stage).ok()?.as_os_str() != stage.as_os_str()
        {
            return None;
        }
        let mut nodes = vec![
            node(&stage, true)?,
            node(&arm, true)?,
            node(&data, true)?,
            node(&app, true)?,
            node(&launch, false)?,
            node(&data.join("library.sqlite"), false)?,
        ];
        let binary = node(&exe, false)?;
        let proc_exe = proc_root.join(pid.to_string()).join("exe");
        if fs::read_link(&proc_exe).ok()?.as_os_str() != exe.as_os_str()
            || Node::from(fs::metadata(&proc_exe).ok()?) != binary
        {
            return None;
        }
        nodes.push(binary);
        let tagged = format!("{label}\n");
        for name in [
            "owner",
            "selector-live",
            "selector-entered",
            "tile-dispatch-intent",
        ] {
            nodes.push(tag(&stage.join(name), tagged.as_bytes())?);
        }
        let arm_file = stage.join("arm-name");
        let named = read_small(&arm_file, 2)?;
        if !matches!(named.as_slice(), b"A\n" | b"B\n") {
            return None;
        }
        nodes.push(tag(&arm_file, &named)?);
        let lock_id = node(lock, true)?;
        let shell = Node::from(fs::metadata(system_shell).ok()?);
        let gui = process(proc_root, pid)?;
        let runner = process(proc_root, gui.parent)?;
        let master = process(proc_root, runner.parent)?;
        let entry = process(proc_root, master.parent)?;
        let processes = [gui, runner, master, entry];
        if !processes
            .windows(2)
            .all(|pair| pair[0].birth >= pair[1].birth)
            || processes.iter().enumerate().any(|(index, process)| {
                processes[..index]
                    .iter()
                    .any(|before| before.id == process.id)
            })
            || !shell_args(proc_root, processes[1].id, &shell, &launch, false)
            || !shell_args(proc_root, processes[2].id, &shell, &launch, false)
            || !shell_args(proc_root, processes[3].id, &shell, &launch, true)
            || read_small(&lock.join("pid"), 24)? != format!("{}\n", processes[2].id).as_bytes()
            || node(lock, true)? != lock_id
        {
            return None;
        }
        Some(Self {
            nodes,
            arm: named[0],
            processes,
            lock: lock_id,
        })
    }
}
