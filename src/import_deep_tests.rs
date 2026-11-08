//! Deep, synthetic EPUB extraction stays isolated from the parent test process.

use std::{
    ffi::OsStr,
    fs::File,
    io::{Cursor, Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, ensure};
use zip::{ZipWriter, write::SimpleFileOptions};

const DEPTH: usize = 30_000;
const SMALL_STACK_BYTES: usize = 256 * 1024;
const CHILD_TIMEOUT: Duration = Duration::from_secs(30);
const CHILD_CASE: &str = "WEREAD_DEEP_IMPORT_CHILD_CASE";
const CHILD_PATH: &str = "WEREAD_DEEP_IMPORT_CHILD_PATH";
const COMPLETED: &str = "WEREAD_DEEP_IMPORT_COMPLETED";
const BOOK_TITLE: &str = "Deep import \u{4e66}\u{7c4d}";
const CHAPTER_TITLE: &str = "Nested \u{6807}\u{9898} \u{1f319}";
const FIRST_LINE: &str = "Reading \u{6d4b}\u{8bd5} \u{1f4d6} e\u{301} & text.";
const LAST_LINE: &str = "After the nested spans: \u{7ed3}\u{5c3e}.";

fn append_nested_spans(output: &mut String, text: &str) {
    for _ in 0..DEPTH {
        output.push_str("<span>");
    }
    // Fixture data is escaped without constructing a recursively owned tree.
    for character in text.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            _ => output.push(character),
        }
    }
    for _ in 0..DEPTH {
        output.push_str("</span>");
    }
}

fn epub_bytes() -> Result<Vec<u8>> {
    let mut chapter = String::new();
    chapter.push_str(
        "<html xmlns=\"http://www.w3.org/1999/xhtml\"><head><title>Fixture</title></head><body><h1>",
    );
    append_nested_spans(&mut chapter, CHAPTER_TITLE);
    chapter.push_str("</h1><p>");
    append_nested_spans(&mut chapter, FIRST_LINE);
    chapter.push_str("</p><p>");
    chapter.push_str(LAST_LINE);
    chapter.push_str("</p></body></html>");
    ensure!(
        chapter.len() < 1024 * 1024,
        "The deep fixture grew unexpectedly"
    );

    let container = r#"<?xml version="1.0" encoding="UTF-8"?>
        <container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0">
          <rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles>
        </container>"#;
    let package = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
        <package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bookid">
          <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
            <dc:identifier id="bookid">deep-import-fixture</dc:identifier>
            <dc:title>{BOOK_TITLE}</dc:title><dc:creator>Fixture Author</dc:creator>
          </metadata>
          <manifest><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/></manifest>
          <spine><itemref idref="chapter"/></spine>
        </package>"#
    );
    let entries = [
        ("mimetype", "application/epub+zip".as_bytes()),
        ("META-INF/container.xml", container.as_bytes()),
        ("content.opf", package.as_bytes()),
        ("chapter.xhtml", chapter.as_bytes()),
    ];
    ensure!(
        entries.iter().map(|(_, bytes)| bytes.len()).sum::<usize>() < 2 * 1024 * 1024,
        "The fixture must remain below every raw/decoded import work budget"
    );
    let mut archive = ZipWriter::new(Cursor::new(Vec::new()));
    // Stored entries also keep compression work out of the small-stack scenario.
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, bytes) in entries {
        archive.start_file(name, options)?;
        archive.write_all(bytes)?;
    }
    Ok(archive.finish()?.into_inner())
}

fn check_public_import(path: PathBuf) -> Result<()> {
    let imported = crate::importer::import_file(&path)?;
    ensure!(
        imported.book.title == BOOK_TITLE,
        "The Unicode book title changed"
    );
    ensure!(
        imported.book.author == "Fixture Author",
        "The author changed"
    );
    ensure!(
        imported.book.total_chapters == 1,
        "The fixture lost its reading order"
    );
    ensure!(
        imported.chapters.len() == 1,
        "The deep chapter was split or lost"
    );
    let chapter = &imported.chapters[0];
    ensure!(
        chapter.title == CHAPTER_TITLE,
        "The deeply nested Unicode chapter title changed"
    );
    let expected = format!("{CHAPTER_TITLE}\n{FIRST_LINE}\n{LAST_LINE}");
    ensure!(
        chapter.body.as_deref() == Some(expected.as_str()),
        "Deep extraction changed the canonical text (expected {} bytes, observed {} bytes)",
        expected.len(),
        chapter.body.as_deref().map_or(0, str::len)
    );
    // ImportedBook, its strings and the fixture path all have flat ownership.
    // Let their normal destruction run on the same small stack as extraction.
    drop(imported);
    Ok(())
}

fn disable_child_core_dump() -> Result<()> {
    #[cfg(unix)]
    {
        let limit = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        // SAFETY: this test child lowers only its own valid core-dump limits.
        if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) } != 0 {
            return Err(std::io::Error::last_os_error()).context("Disable child core dumps");
        }
    }
    Ok(())
}

struct ChildOwner {
    child: Child,
}

impl ChildOwner {
    fn wait_until_finished(&mut self, timeout: Duration) -> Result<ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self
                .child
                .try_wait()
                .context("Observe the deep-import child")?
            {
                return Ok(status);
            }
            ensure!(Instant::now() < deadline, "The deep-import child timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for ChildOwner {
    fn drop(&mut self) {
        // Keep ownership even when an assertion, timeout or I/O error unwinds.
        // The child has no subprocesses; killing it also ends its worker thread.
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

fn read_diagnostics(path: &Path) -> String {
    let Ok(file) = File::open(path) else {
        return String::from("<unavailable>");
    };
    let mut bytes = Vec::new();
    let _ = file.take(16 * 1024).read_to_end(&mut bytes);
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
fn deeply_nested_epub_text_and_titles_survive_public_import_on_a_small_stack() -> Result<()> {
    // Derive the harness name so the parent can choose the module's path name.
    let module = module_path!();
    let relative = module
        .split_once("::")
        .map_or(module, |(_, relative)| relative);
    let case = format!(
        "{relative}::deeply_nested_epub_text_and_titles_survive_public_import_on_a_small_stack"
    );
    let marker = format!("{COMPLETED}:{case}:{DEPTH}:{SMALL_STACK_BYTES}");
    if let Some(selected) = std::env::var_os(CHILD_CASE) {
        ensure!(
            selected.as_os_str() == OsStr::new(&case),
            "The deep-import child selected a different case"
        );
        disable_child_core_dump()?;
        let path = PathBuf::from(
            std::env::var_os(CHILD_PATH).context("The deep-import child has no fixture path")?,
        );
        thread::Builder::new()
            .name("weread-deep-import".into())
            .stack_size(SMALL_STACK_BYTES)
            .spawn(move || check_public_import(path))
            .context("Start the isolated small-stack import worker")?
            .join()
            .map_err(|_| anyhow!("The small-stack import worker panicked"))??;
        println!("\n{marker}");
        return Ok(());
    }

    let directory = tempfile::tempdir()?;
    let mut epub = tempfile::Builder::new()
        .prefix("weread-deep-import-")
        .suffix(".epub")
        .tempfile_in(directory.path())?;
    epub.write_all(&epub_bytes()?)?;
    epub.flush()?;
    epub.as_file().sync_all()?;
    let stdout_path = directory.path().join("child.stdout");
    let stderr_path = directory.path().join("child.stderr");
    let mut owner = ChildOwner {
        child: Command::new(std::env::current_exe()?)
            .args(["--exact", &case, "--nocapture", "--test-threads=1"])
            .env(CHILD_CASE, &case)
            .env(CHILD_PATH, epub.path())
            .env("RUST_BACKTRACE", "0")
            .env("RUST_LIB_BACKTRACE", "0")
            .env_remove("WEREAD_API_KEY")
            .env_remove("WEREAD_LAUNCH_GENERATION")
            .stdin(Stdio::null())
            // Files avoid a full pipe deadlocking a child that reports an error.
            .stdout(Stdio::from(File::create(&stdout_path)?))
            .stderr(Stdio::from(File::create(&stderr_path)?))
            .spawn()
            .context("Start the isolated deep-import test process")?,
    };
    let status = owner.wait_until_finished(CHILD_TIMEOUT);
    // This also kills and reaps a timed-out child before reading or removing logs.
    drop(owner);
    let stdout = read_diagnostics(&stdout_path);
    let stderr = read_diagnostics(&stderr_path);
    let status = status.with_context(|| {
        format!("The deep-import child did not finish\nstdout:\n{stdout}\nstderr:\n{stderr}")
    })?;
    ensure!(
        status.success()
            && stdout
                .lines()
                .filter(|line| *line == marker.as_str())
                .count()
                == 1,
        "The isolated deep import failed: {status}\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    Ok(())
}
