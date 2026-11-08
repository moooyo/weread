//! Deep HTML contracts exercised in isolated, small-stack child processes.

use std::{
    collections::BTreeSet,
    fs::File,
    io::Read,
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail, ensure};
use scraper::{Html, Node};

use crate::{
    article::{ArticleDocument, image_anchors},
    book_assets::prepare_chapter_metadata,
    reader::{Layout, TextMetrics},
};

const DEPTH: usize = 30_000;
const STACK_BYTES: usize = 512 * 1024;
const CHILD_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_OUTPUT_BYTES: u64 = 64 * 1024;
const CHILD_CASE: &str = "WEREAD_ARTICLE_DEEP_TEST_CASE";
const COMPLETED: &str = "WEREAD_ARTICLE_DEEP_TEST_COMPLETED";

fn nested_spans(text: &str) -> String {
    let mut html = String::with_capacity(DEPTH * 13 + text.len());
    for _ in 0..DEPTH {
        html.push_str("<span>");
    }
    html.push_str(text);
    for _ in 0..DEPTH {
        html.push_str("</span>");
    }
    html
}

/// Verify that the parser retained the fixture's depth. The fixture owns only
/// flat strings and arena node IDs, including during destruction after a panic.
fn require_deep_dom(html: &str, marker: &str) -> Result<()> {
    let document = Html::parse_document(html);
    let ids: Vec<_> = document
        .tree
        .root()
        .descendants()
        .map(|node| node.id())
        .collect();
    ensure!(
        ids.len() > DEPTH,
        "The deep HTML fixture lost its arena nodes"
    );
    let leaf = document
        .tree
        .root()
        .descendants()
        .find(|node| matches!(node.value(), Node::Text(text) if text.contains(marker)))
        .context("The deep HTML fixture lost its marker")?;
    let mut current = Some(leaf.id());
    let mut depth = 0;
    while let Some(id) = current {
        depth += 1;
        current = document
            .tree
            .get(id)
            .and_then(|node| node.parent())
            .map(|node| node.id());
    }
    ensure!(depth > DEPTH, "The parser flattened the deep HTML fixture");
    Ok(())
}

fn article_body_contract() -> Result<()> {
    let text = "Deep Unicode: \u{6df1}\u{5c42}\u{6c49}\u{5b57} e\u{301}\u{1f642}";
    let hidden = nested_spans("suppressed article sentinel");
    let html = format!(
        "<article id=\"js_content\"><h2>Heading \u{1f642}</h2><blockquote>{}</blockquote>\
         <div hidden>{hidden}</div><p>Tail paragraph</p></article>",
        nested_spans(text),
    );
    require_deep_dom(&html, text)?;
    let document = ArticleDocument::parse(&html, "unused fallback");
    ensure!(
        document.plain_text() == format!("Heading \u{1f642}\n{text}\nTail paragraph"),
        "Deep article text lost Unicode, order, or hidden-subtree suppression"
    );
    let pages = document.paginate(
        &Layout {
            width: 480.0,
            height: 240.0,
            font_size: 18.0,
            ..Layout::default()
        },
        &TextMetrics::default(),
        &BTreeSet::new(),
    );
    let text_blocks: Vec<_> = pages
        .iter()
        .flatten()
        .filter(|block| !block.text.is_empty())
        .map(|block| (block.kind.as_str(), block.text.as_str()))
        .collect();
    ensure!(
        text_blocks
            == [
                ("heading", "Heading \u{1f642}"),
                ("quote", text),
                ("paragraph", "Tail paragraph")
            ],
        "Deep article traversal lost inherited block kind or block order"
    );
    Ok(())
}

fn book_anchor_contract() -> Result<()> {
    let prefix = "\u{7532}\u{1f642}e\u{301}";
    let html = format!(
        "<main>{}<img src=\"first.png\" alt=\"First image\">\
         <span style=\"display: none\">hidden text<img src=\"hidden.png\"></span>\
         <img src=\"second.png\" alt=\"Second image\">\u{4e59}</main>",
        nested_spans(prefix),
    );
    require_deep_dom(&html, prefix)?;
    let canonical = format!("{prefix}\u{4e59}");
    let anchors = image_anchors(&html, &canonical)?;
    ensure!(
        anchors.len() == 2,
        "Deep book traversal retained a hidden image"
    );
    ensure!(
        anchors[0].source == "first.png"
            && anchors[1].source == "second.png"
            && anchors[0].caption == "First image"
            && anchors[1].caption == "Second image",
        "Deep book traversal changed image order or captions"
    );
    let expected = Some(prefix.chars().count() as u32);
    ensure!(
        anchors.iter().all(|image| image.anchor == expected),
        "Deep book anchors must address Unicode scalars rather than bytes"
    );
    prepare_chapter_metadata(&html, &canonical)
        .context("Prepare deep book metadata through the production entry")?;
    Ok(())
}

fn caption_contract() -> Result<()> {
    let caption = "Caption \u{8bf4}\u{660e}\u{1f642}";
    let html = format!(
        "<figure><img src=\"https://images.example.test/caption.png\"><figcaption>{}\
         <span hidden> hidden-caption </span><script>script-caption</script>\
         </figcaption></figure><p>Tail</p>",
        nested_spans(caption),
    );
    require_deep_dom(&html, caption)?;
    let canonical = format!("{caption}Tail");
    let anchors = image_anchors(&html, &canonical)?;
    ensure!(
        anchors.len() == 1 && anchors[0].anchor == Some(0) && anchors[0].caption == caption,
        "Deep book captions lost their suppression or optional-text alignment"
    );
    prepare_chapter_metadata(&html, &canonical)
        .context("Prepare deep caption metadata through the production entry")?;

    // The article renderer historically collects every descendant text inside
    // figcaption. That differs from the book mapper's suppressed caption walk.
    let document = ArticleDocument::parse(&html, "unused fallback");
    let images = document.images();
    ensure!(
        images.len() == 1
            && images[0].caption == format!("{caption} hidden-caption script-caption"),
        "Deep traversal changed the article renderer's existing caption contract"
    );
    ensure!(
        document.plain_text() == "Tail",
        "An attached image caption became an extra article paragraph"
    );
    Ok(())
}

struct ChildOwner(Option<Child>);

impl ChildOwner {
    fn try_wait(&mut self) -> Result<Option<ExitStatus>> {
        self.0
            .as_mut()
            .context("The isolated HTML child was already reaped")?
            .try_wait()
            .context("Poll the isolated HTML child")
    }

    fn reap(&mut self) -> Result<ExitStatus> {
        let status = self
            .0
            .as_mut()
            .context("The isolated HTML child was already reaped")?
            .wait()
            .context("Reap the isolated HTML child")?;
        self.0 = None;
        Ok(status)
    }

    fn stop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for ChildOwner {
    fn drop(&mut self) {
        self.stop();
    }
}

fn read_output(path: &Path) -> Result<String> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_OUTPUT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    let truncated = bytes.len() as u64 > MAX_OUTPUT_BYTES;
    bytes.truncate(MAX_OUTPUT_BYTES as usize);
    let mut output = String::from_utf8_lossy(&bytes).into_owned();
    if truncated {
        output.push_str("\n[child output truncated]");
    }
    Ok(output)
}

#[cfg(unix)]
fn disable_child_core_dump() -> Result<()> {
    let limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: This changes only the isolated test child's own core-file limit.
    if unsafe { libc::setrlimit(libc::RLIMIT_CORE, &limit) } != 0 {
        return Err(std::io::Error::last_os_error()).context("Disable child core files");
    }
    Ok(())
}

#[cfg(not(unix))]
fn disable_child_core_dump() -> Result<()> {
    Ok(())
}

fn run_isolated(name: &str, scenario: fn() -> Result<()>) -> Result<()> {
    let module = module_path!()
        .split_once("::")
        .map(|(_, module)| module)
        .context("The HTML test module has no crate prefix")?;
    let case = format!("{module}::{name}");
    if let Some(selected) = std::env::var_os(CHILD_CASE) {
        ensure!(
            selected.as_os_str() == std::ffi::OsStr::new(&case),
            "The isolated HTML child selected a different case"
        );
        disable_child_core_dump()?;
        std::thread::Builder::new()
            .name("weread-deep-html".into())
            .stack_size(STACK_BYTES)
            .spawn(scenario)
            .context("Start the small-stack HTML worker")?
            .join()
            .map_err(|_| anyhow::anyhow!("The small-stack HTML worker panicked"))??;
        println!("\n{COMPLETED}:{case}");
        return Ok(());
    }

    let directory = tempfile::Builder::new()
        .prefix("weread-deep-html-")
        .tempdir()?;
    let stdout_path = directory.path().join("stdout.log");
    let stderr_path = directory.path().join("stderr.log");
    let child = Command::new(std::env::current_exe()?)
        .args(["--exact", &case, "--nocapture", "--test-threads=1"])
        .env(CHILD_CASE, &case)
        .env_remove("WEREAD_API_KEY")
        .env_remove("WEREAD_LAUNCH_GENERATION")
        .current_dir(directory.path())
        .stdin(Stdio::null())
        .stdout(Stdio::from(File::create(&stdout_path)?))
        .stderr(Stdio::from(File::create(&stderr_path)?))
        .spawn()
        .context("Start the isolated HTML child")?;
    let mut owner = ChildOwner(Some(child));
    let deadline = Instant::now() + CHILD_TIMEOUT;
    loop {
        if owner.try_wait()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            owner.stop();
            bail!(
                "The isolated HTML child exceeded its deadline: {case}\nstdout:\n{}\nstderr:\n{}",
                read_output(&stdout_path)?,
                read_output(&stderr_path)?,
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let status = owner.reap()?;
    let stdout = read_output(&stdout_path)?;
    let stderr = read_output(&stderr_path)?;
    let marker = format!("{COMPLETED}:{case}");
    ensure!(
        status.success()
            && stdout
                .lines()
                .filter(|line| *line == marker.as_str())
                .count()
                == 1,
        "The isolated HTML contract did not complete exactly once: {case}\nstatus: {status}\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    Ok(())
}

#[test]
fn deep_article_body_keeps_unicode_block_order_and_quote_inheritance() -> Result<()> {
    run_isolated(
        "deep_article_body_keeps_unicode_block_order_and_quote_inheritance",
        article_body_contract,
    )
}

#[test]
fn deep_book_body_keeps_scalar_image_anchors_and_metadata_binding() -> Result<()> {
    run_isolated(
        "deep_book_body_keeps_scalar_image_anchors_and_metadata_binding",
        book_anchor_contract,
    )
}

#[test]
fn deep_caption_keeps_the_distinct_book_and_article_text_contracts() -> Result<()> {
    run_isolated(
        "deep_caption_keeps_the_distinct_book_and_article_text_contracts",
        caption_contract,
    )
}
