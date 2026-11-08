//! Offline import of unencrypted UTF-8 text and EPUB reading-order text.
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{Cursor, Read},
    path::Path,
};

use anyhow::{Context, Result, bail, ensure};
use scraper::{ElementRef, Html, Selector};
use sha2::{Digest, Sha256};
use zip::ZipArchive;

use crate::domain::{Book, Chapter};

#[path = "import_ncx_entities.rs"]
mod ncx_entities;

#[path = "import_xml_work.rs"]
mod xml_work;

#[path = "import_nav_work.rs"]
mod nav_work;

const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_BOOK_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ENTRY_BYTES: u64 = 8 * 1024 * 1024;
const MAX_WORK_BYTES: u64 = 64 * 1024 * 1024;
const MAX_COMPRESSED_WORK_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ENTRIES: usize = 8192;
const TEXT_CHAPTER_BYTES: usize = 128 * 1024;
const MAX_RETAINED_BYTES: usize = 64 * 1024 * 1024;
const MAX_TITLE_BYTES: usize = 16 * 1024;
const MAX_PATH_BYTES: usize = 4096;
const MAX_NCX_NODES: u32 = 250_000;
const NCX_NODE_VISITS_PER_NODE: usize = 6;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct ImportLimitExceeded(&'static str);

#[derive(Clone, Copy)]
struct RetainedBudget {
    // This bounds explicitly retained payloads and conservative collection
    // slots. It is not a hard limit on parser DOMs or allocator bookkeeping.
    used: usize,
}

impl RetainedBudget {
    fn new() -> Self {
        Self {
            // Count the returned object and spare slots for the Vec's first
            // allocation. Per-chapter charges cover later geometric growth.
            used: std::mem::size_of::<ImportedBook>() + 4 * std::mem::size_of::<Chapter>(),
        }
    }

    fn reserve(&mut self, parts: &[usize]) -> Result<()> {
        let next = parts
            .iter()
            .try_fold(self.used, |used, part| used.checked_add(*part));
        let Some(next) = next.filter(|next| *next <= MAX_RETAINED_BYTES) else {
            return Err(
                ImportLimitExceeded("import retained data exceeds the 64 MiB limit").into(),
            );
        };
        self.used = next;
        Ok(())
    }

    fn book(
        &mut self,
        id: &str,
        title: &str,
        author: &str,
        category: &str,
        intro: &str,
    ) -> Result<()> {
        check_title_length(title.len())?;
        self.reserve(&[
            id.len(),
            title.len(),
            author.len(),
            category.len(),
            intro.len(),
            "local".len(),
        ])
    }

    fn chapter(
        &mut self,
        id: &str,
        index: usize,
        title_bytes: usize,
        body_bytes: usize,
    ) -> Result<()> {
        ensure!(index < MAX_ENTRIES, "import contains too many chapters");
        check_title_length(title_bytes)?;
        self.reserve(&[
            2 * std::mem::size_of::<Chapter>(),
            id.len(),
            1,
            index.to_string().len(),
            id.len(),
            title_bytes,
            body_bytes,
        ])
    }

    fn strings(&mut self, parts: &[&str], fixed_bytes: usize) -> Result<()> {
        let mut bytes = fixed_bytes;
        for part in parts {
            bytes = bytes
                .checked_add(part.len())
                .context("import metadata size overflow")?;
        }
        self.reserve(&[bytes])
    }
}

fn check_title_length(bytes: usize) -> Result<()> {
    if bytes > MAX_TITLE_BYTES {
        return Err(ImportLimitExceeded("import title exceeds the 16 KiB limit").into());
    }
    Ok(())
}

fn append_limited(
    output: &mut String,
    text: &str,
    maximum: usize,
    message: &'static str,
) -> Result<()> {
    if output
        .len()
        .checked_add(text.len())
        .is_none_or(|length| length > maximum)
    {
        return Err(ImportLimitExceeded(message).into());
    }
    output.push_str(text);
    Ok(())
}

#[derive(Debug)]
pub struct ImportedBook {
    pub book: Book,
    pub chapters: Vec<Chapter>,
}

/// Imports local bytes only. No links, images, fonts or external resources are fetched.
pub fn import_file(path: &Path) -> Result<ImportedBook> {
    let file = File::open(path).context("open local import")?;
    ensure!(
        file.metadata()?.is_file(),
        "import path is not a regular file"
    );
    ensure!(
        file.metadata()?.len() <= MAX_FILE_BYTES,
        "import exceeds the 64 MiB file limit"
    );
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_FILE_BYTES,
        "import exceeds the 64 MiB file limit"
    );
    let id = format!("local-{:x}", Sha256::digest(&bytes));
    let fallback_title = path
        .file_stem()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("Imported book");
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match extension.as_str() {
        "txt" => import_text(&bytes, &id, fallback_title),
        "epub" => import_epub(&bytes, &id, fallback_title),
        _ => bail!("supported local import formats are UTF-8 TXT and unencrypted EPUB"),
    }
}

fn import_text(bytes: &[u8], id: &str, title: &str) -> Result<ImportedBook> {
    ensure!(
        bytes.len() as u64 <= MAX_BOOK_BYTES,
        "text exceeds the 32 MiB book limit"
    );
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    let text = std::str::from_utf8(bytes).context("TXT import requires UTF-8 encoding")?;
    ensure!(
        !text.contains('\0'),
        "TXT import contains binary null characters"
    );
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    ensure!(!text.trim().is_empty(), "text has no readable content");
    let mut budget = RetainedBudget::new();
    budget.book(id, title, "", "", "")?;
    let mut chapters = Vec::new();
    let mut section_title = title.to_owned();
    let mut has_pending_heading = false;
    let mut body = String::new();
    for line in text.lines() {
        if is_heading(line.trim()) {
            if !body.trim().is_empty() {
                append_text_chapters(id, &section_title, &body, &mut chapters, &mut budget)?;
                body.clear();
                section_title = line.trim().to_owned();
            } else if has_pending_heading {
                check_title_length(
                    section_title
                        .len()
                        .checked_add(3)
                        .and_then(|length| length.checked_add(line.trim().len()))
                        .context("import title size overflow")?,
                )?;
                section_title.push_str(" / ");
                section_title.push_str(line.trim());
            } else {
                section_title = line.trim().to_owned();
            }
            has_pending_heading = true;
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    if !body.trim().is_empty() {
        append_text_chapters(id, &section_title, &body, &mut chapters, &mut budget)?;
    } else if has_pending_heading {
        append_text_chapters(
            id,
            &section_title,
            &section_title,
            &mut chapters,
            &mut budget,
        )?;
    }
    // A heading-only text is still readable; keep it instead of silently dropping it.
    if chapters.is_empty() {
        append_text_chapters(id, title, &text, &mut chapters, &mut budget)?;
    }
    finish_import(id, title, "", "", "", chapters)
}

fn append_text_chapters(
    id: &str,
    title: &str,
    body: &str,
    chapters: &mut Vec<Chapter>,
    budget: &mut RetainedBudget,
) -> Result<()> {
    for (part, body) in split_text(body).enumerate() {
        let suffix = if part == 0 {
            String::new()
        } else {
            format!(" ({})", part + 1)
        };
        let title_bytes = title
            .len()
            .checked_add(suffix.len())
            .context("import title size overflow")?;
        budget.chapter(id, chapters.len(), title_bytes, body.len())?;
        let mut chapter_title = String::with_capacity(title_bytes);
        chapter_title.push_str(title);
        chapter_title.push_str(&suffix);
        chapters.push(make_chapter(
            id,
            chapters.len(),
            chapter_title,
            body.to_owned(),
        )?);
    }
    Ok(())
}

fn is_heading(line: &str) -> bool {
    let length = line.chars().count();
    if length == 0 || length > 60 {
        return false;
    }
    let lower = line.to_ascii_lowercase();
    for prefix in ["chapter ", "part ", "book "] {
        if let Some(remainder) = lower.strip_prefix(prefix) {
            let number = remainder
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .trim_end_matches(['.', ':', '-', ')']);
            return !number.is_empty()
                && (number.bytes().all(|byte| byte.is_ascii_digit())
                    || number.bytes().all(|byte| {
                        matches!(byte, b'i' | b'v' | b'x' | b'l' | b'c' | b'd' | b'm')
                    })
                    || matches!(
                        number,
                        "one"
                            | "two"
                            | "three"
                            | "four"
                            | "five"
                            | "six"
                            | "seven"
                            | "eight"
                            | "nine"
                            | "ten"
                            | "eleven"
                            | "twelve"
                            | "thirteen"
                            | "fourteen"
                            | "fifteen"
                            | "sixteen"
                            | "seventeen"
                            | "eighteen"
                            | "nineteen"
                            | "twenty"
                    ));
        }
    }
    if !line.starts_with('\u{7b2c}') {
        return false;
    }
    line.chars()
        .skip(1)
        .take(16)
        .any(|character| matches!(character, '\u{7ae0}' | '\u{56de}' | '\u{8282}' | '\u{5377}'))
        && !line.chars().any(|character| {
            matches!(
                character,
                '\u{3002}' | '\u{ff1b}' | ';' | '\u{ff1f}' | '\u{ff01}'
            )
        })
}

struct TextParts<'a> {
    rest: &'a str,
}

fn split_text(text: &str) -> TextParts<'_> {
    TextParts { rest: text.trim() }
}

impl<'a> Iterator for TextParts<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let rest = self.rest;
            if rest.is_empty() {
                return None;
            }
            let mut end = rest.len().min(TEXT_CHAPTER_BYTES);
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            if end < rest.len() {
                if let Some(newline) = rest[..end].rfind('\n').filter(|index| *index >= end / 2) {
                    end = newline;
                }
            }
            self.rest = rest[end..].trim_start();
            let part = rest[..end].trim();
            if !part.is_empty() {
                return Some(part);
            }
        }
    }
}

#[derive(Clone)]
struct ManifestItem {
    href: String,
    media_type: String,
    properties: String,
}

struct EpubReader<'a> {
    archive: ZipArchive<Cursor<&'a [u8]>>,
    names: HashSet<String>,
    read_names: HashSet<String>,
    read_bytes: u64,
    read_work_bytes: u64,
    compressed_work_bytes: u64,
    decoded_work_bytes: u64,
    budget: RetainedBudget,
    xml_work: xml_work::Budget,
    html_work: nav_work::Budget,
}

impl<'a> EpubReader<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self> {
        let mut archive = ZipArchive::new_with_file_limit(Cursor::new(bytes), MAX_ENTRIES)
            .context("open EPUB archive with bounded directory records")?;
        ensure!(
            archive.len() <= MAX_ENTRIES,
            "EPUB contains too many entries"
        );
        let mut names = HashSet::new();
        let mut budget = RetainedBudget::new();
        let mut declared_total = 0_u64;
        for index in 0..archive.len() {
            let entry = archive
                .by_index(index)
                .context("read EPUB entry metadata; encrypted ZIP entries are unsupported")?;
            let name = entry.name();
            let canonical = normalize_path(name.trim_end_matches('/'))?;
            ensure!(
                canonical == name.trim_end_matches('/'),
                "EPUB archive member contains a noncanonical path"
            );
            budget.strings(&[&canonical], 2 * std::mem::size_of::<String>())?;
            ensure!(
                names.insert(canonical),
                "EPUB contains duplicate archive paths"
            );
            ensure!(
                entry.size() <= MAX_ENTRY_BYTES,
                "EPUB entry exceeds the 8 MiB limit"
            );
            declared_total = declared_total
                .checked_add(entry.size())
                .context("EPUB size overflow")?;
            ensure!(
                declared_total <= MAX_BOOK_BYTES,
                "EPUB exceeds the 32 MiB uncompressed book limit"
            );
        }
        Ok(Self {
            archive,
            names,
            read_names: HashSet::new(),
            read_bytes: 0,
            read_work_bytes: 0,
            compressed_work_bytes: 0,
            decoded_work_bytes: 0,
            budget,
            xml_work: xml_work::Budget::new(),
            html_work: nav_work::Budget::new(),
        })
    }

    fn contains(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    fn read_text(&mut self, name: &str) -> Result<String> {
        let mut entry = self
            .archive
            .by_name(name)
            .with_context(|| format!("missing or encrypted EPUB entry: {name}"))?;
        ensure!(!entry.is_dir(), "EPUB reading entry is a directory");
        ensure!(
            entry.size() <= MAX_ENTRY_BYTES,
            "EPUB entry exceeds the 8 MiB limit"
        );
        let declared_size = entry.size();
        // The archive and retained-body quotas alone do not cover repeated
        // references to empty or hidden markup. Charge every physical read
        // before decompression, including entries already present in read_names.
        let next_work = self.read_work_bytes.checked_add(declared_size);
        if next_work.is_none_or(|bytes| bytes > MAX_WORK_BYTES) {
            return Err(
                ImportLimitExceeded("EPUB cumulative read work exceeds the 64 MiB limit").into(),
            );
        }
        // Empty-output DEFLATE streams can still consume an entire compressed
        // region. Count every access, including a repeated spine reference,
        // independently of the declared and decoded output quotas.
        let next_compressed = self
            .compressed_work_bytes
            .checked_add(entry.compressed_size());
        if next_compressed.is_none_or(|bytes| bytes > MAX_COMPRESSED_WORK_BYTES) {
            return Err(ImportLimitExceeded(
                "EPUB cumulative compressed work exceeds the 64 MiB limit",
            )
            .into());
        }
        // Neither reservation advances when one preflight rejects the read.
        self.read_work_bytes = next_work.unwrap();
        self.compressed_work_bytes = next_compressed.unwrap();
        let mut bytes = Vec::new();
        entry
            .by_ref()
            .take(MAX_ENTRY_BYTES + 1)
            .read_to_end(&mut bytes)
            .context("decompress EPUB entry")?;
        ensure!(
            bytes.len() as u64 <= MAX_ENTRY_BYTES,
            "EPUB entry exceeds the decompression limit"
        );
        ensure!(
            bytes.len() as u64 == declared_size,
            "EPUB entry has an inconsistent uncompressed size"
        );
        if !self.read_names.contains(name) {
            self.budget
                .strings(&[name], 2 * std::mem::size_of::<String>())?;
            self.read_names.insert(name.to_owned());
            self.read_bytes = self
                .read_bytes
                .checked_add(bytes.len() as u64)
                .context("EPUB decompression size overflow")?;
            ensure!(
                self.read_bytes <= MAX_BOOK_BYTES,
                "EPUB exceeds the 32 MiB decompression limit"
            );
        }
        // Preflight the exact UTF-8 output length without allocating a decoded
        // String, so UTF-16 expansion cannot pass its work quota before rejection.
        let decoded_bytes = decoded_epub_text_length(&bytes)? as u64;
        let next_work = self.decoded_work_bytes.checked_add(decoded_bytes);
        if next_work.is_none_or(|bytes| bytes > MAX_WORK_BYTES) {
            return Err(ImportLimitExceeded(
                "EPUB cumulative decoded work exceeds the 64 MiB limit",
            )
            .into());
        }
        self.decoded_work_bytes = next_work.unwrap();
        decode_epub_text(&bytes)
    }
}

fn decoded_epub_text_length(bytes: &[u8]) -> Result<usize> {
    let little = bytes.starts_with(&[0xFF, 0xFE]);
    if little || bytes.starts_with(&[0xFE, 0xFF]) {
        let bytes = &bytes[2..];
        ensure!(
            bytes.len() % 2 == 0,
            "EPUB text has an incomplete UTF-16 code unit"
        );
        let units = bytes.chunks_exact(2).map(|pair| {
            if little {
                u16::from_le_bytes([pair[0], pair[1]])
            } else {
                u16::from_be_bytes([pair[0], pair[1]])
            }
        });
        let mut length = 0_usize;
        for character in char::decode_utf16(units) {
            let character = character.context("decode EPUB UTF-16 text")?;
            ensure!(
                character != '\0',
                "EPUB text contains binary null characters"
            );
            length = length
                .checked_add(character.len_utf8())
                .context("decoded EPUB text size overflow")?;
        }
        ensure!(
            length as u64 <= MAX_ENTRY_BYTES * 2,
            "decoded EPUB text exceeds its memory limit"
        );
        Ok(length)
    } else {
        let text = std::str::from_utf8(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes))
            .context("EPUB text requires UTF-8 or BOM-marked UTF-16 encoding")?;
        ensure!(
            !text.contains('\0'),
            "EPUB text contains binary null characters"
        );
        Ok(text.len())
    }
}

fn decode_epub_text(bytes: &[u8]) -> Result<String> {
    let result = if let Some(bytes) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        ensure!(
            bytes.len() % 2 == 0,
            "EPUB text has an incomplete UTF-16 code unit"
        );
        String::from_utf16(
            &bytes
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>(),
        )
        .context("decode EPUB UTF-16LE text")?
    } else if let Some(bytes) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        ensure!(
            bytes.len() % 2 == 0,
            "EPUB text has an incomplete UTF-16 code unit"
        );
        String::from_utf16(
            &bytes
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect::<Vec<_>>(),
        )
        .context("decode EPUB UTF-16BE text")?
    } else {
        String::from_utf8(
            bytes
                .strip_prefix(&[0xEF, 0xBB, 0xBF])
                .unwrap_or(bytes)
                .to_vec(),
        )
        .context("EPUB text requires UTF-8 or BOM-marked UTF-16 encoding")?
    };
    ensure!(
        !result.contains('\0'),
        "EPUB text contains binary null characters"
    );
    ensure!(
        result.len() as u64 <= MAX_ENTRY_BYTES * 2,
        "decoded EPUB text exceeds its memory limit"
    );
    Ok(result)
}

fn import_epub(bytes: &[u8], id: &str, fallback_title: &str) -> Result<ImportedBook> {
    let mut reader = EpubReader::new(bytes)?;
    if reader.contains("mimetype") {
        ensure!(
            reader.read_text("mimetype")?.trim() == "application/epub+zip",
            "archive is not an EPUB publication"
        );
    }
    let container = reader.read_text("META-INF/container.xml")?;
    let container = reader
        .xml_work
        .parse(&container, xml_work::Kind::Container)
        .context("parse EPUB container XML")?;
    let rootfile = container
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "rootfile"
                && node.attribute("media-type") == Some("application/oebps-package+xml")
        })
        .or_else(|| {
            container
                .descendants()
                .find(|node| node.is_element() && node.tag_name().name() == "rootfile")
        })
        .and_then(|node| node.attribute("full-path"))
        .context("EPUB container has no package document")?;
    let package_path = resolve_href("", rootfile)?;
    let package = reader.read_text(&package_path)?;
    let package = reader
        .xml_work
        .parse(&package, xml_work::Kind::Package)
        .context("parse EPUB package XML")?;
    let metadata = package
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "metadata");
    let metadata_value = |name, budget: &mut RetainedBudget| -> Result<String> {
        let node = metadata.and_then(|metadata| {
            metadata
                .descendants()
                .find(|node| node.is_element() && node.tag_name().name() == name)
        });
        let mut value = String::new();
        if let Some(node) = node {
            let maximum = if name == "title" {
                MAX_TITLE_BYTES
            } else {
                (MAX_ENTRY_BYTES * 2) as usize
            };
            for text in node
                .descendants()
                .filter_map(|node| node.text().filter(|_| node.is_text()))
            {
                budget.reserve(&[text.len()])?;
                append_limited(
                    &mut value,
                    text,
                    maximum,
                    "EPUB metadata text exceeds its retained limit",
                )?;
            }
        }
        let start = value.len() - value.trim_start().len();
        value.drain(..start);
        value.truncate(value.trim_end().len());
        Ok(value)
    };
    let title = metadata_value("title", &mut reader.budget)?;
    let author = metadata_value("creator", &mut reader.budget)?;
    let category = metadata_value("subject", &mut reader.budget)?;
    let intro = metadata_value("description", &mut reader.budget)?;
    let title = if title.is_empty() {
        fallback_title
    } else {
        &title
    };
    reader.budget.book(id, title, &author, &category, &intro)?;
    let manifest_node = package
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "manifest")
        .context("EPUB package has no manifest")?;
    let mut manifest = HashMap::new();
    for item in manifest_node
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "item")
    {
        let item_id = item
            .attribute("id")
            .context("EPUB manifest item has no identifier")?;
        let href = item
            .attribute("href")
            .context("EPUB manifest item has no href")?;
        if item_id.len() > MAX_PATH_BYTES || href.len() > MAX_PATH_BYTES * 3 {
            return Err(ImportLimitExceeded(
                "EPUB manifest identifier or path exceeds its retained limit",
            )
            .into());
        }
        let media_type = item.attribute("media-type").unwrap_or_default();
        let properties = item.attribute("properties").unwrap_or_default();
        reader.budget.strings(
            &[item_id, href, media_type, properties],
            2 * std::mem::size_of::<(String, ManifestItem)>(),
        )?;
        ensure!(
            manifest
                .insert(
                    item_id.to_owned(),
                    ManifestItem {
                        href: href.to_owned(),
                        media_type: media_type.to_owned(),
                        properties: properties.to_owned()
                    }
                )
                .is_none(),
            "EPUB contains duplicate manifest identifiers"
        );
    }
    let encrypted = encrypted_resources(&mut reader)?;
    let mut titles = HashMap::new();
    if let Some(item) = manifest.values().find(|item| {
        item.properties
            .split_whitespace()
            .any(|property| property == "nav")
    }) {
        let path = resolve_href(&package_path, &item.href)?;
        if !encrypted.contains(&path) {
            let nav = reader.read_text(&path)?;
            titles.extend(nav_titles_with_work(
                &nav,
                &path,
                &mut reader.budget,
                &mut reader.html_work,
            )?);
        }
    }
    let spine = package
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "spine")
        .context("EPUB package has no spine")?;
    if titles.is_empty() {
        if let Some(item) = spine.attribute("toc").and_then(|toc| manifest.get(toc)) {
            let path = resolve_href(&package_path, &item.href)?;
            if !encrypted.contains(&path) {
                let ncx = reader.read_text(&path)?;
                let before_navigation = reader.budget;
                match ncx_titles_with_xml_budget(
                    &ncx,
                    &path,
                    &mut reader.budget,
                    &mut reader.xml_work,
                ) {
                    Ok(navigation) => titles.extend(navigation),
                    Err(error) if error.downcast_ref::<ImportLimitExceeded>().is_some() => {
                        return Err(error);
                    }
                    Err(_) => {
                        // Keep the existing malformed-NCX fallback without
                        // charging a map that the failed parser already dropped.
                        reader.budget = before_navigation;
                    }
                }
            }
        }
    }
    let mut chapters = Vec::new();
    let mut text_bytes = 0_u64;
    for (spine_index, itemref) in spine
        .children()
        .filter(|node| node.is_element() && node.tag_name().name() == "itemref")
        .enumerate()
    {
        ensure!(
            spine_index < MAX_ENTRIES,
            "EPUB spine contains too many entries"
        );
        let item_id = itemref
            .attribute("idref")
            .context("EPUB spine item has no idref")?;
        let item = manifest
            .get(item_id)
            .context("EPUB spine references a missing manifest item")?;
        let path = resolve_href(&package_path, &item.href)?;
        ensure!(
            !encrypted.contains(&path),
            "encrypted EPUB chapter content is unsupported"
        );
        ensure!(
            matches!(
                item.media_type.as_str(),
                "application/xhtml+xml" | "text/html" | "text/plain" | ""
            ),
            "EPUB spine has an unsupported reading format"
        );
        let content = reader.read_text(&path)?;
        let (body, heading) = if item.media_type == "text/plain" {
            (
                content
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    .trim()
                    .to_owned(),
                None,
            )
        } else {
            extract_html_with_work(
                &content,
                !titles.contains_key(&path),
                &mut reader.budget,
                &mut reader.html_work,
            )?
        };
        if body.is_empty() {
            continue;
        }
        text_bytes = text_bytes
            .checked_add(body.len() as u64)
            .context("EPUB reading text size overflow")?;
        ensure!(
            text_bytes <= MAX_BOOK_BYTES,
            "EPUB extracted text exceeds the 32 MiB book limit"
        );
        let chapter_title = titles
            .get(&path)
            .map(String::as_str)
            .or(heading.as_deref())
            .unwrap_or(item_id);
        reader
            .budget
            .chapter(id, chapters.len(), chapter_title.len(), body.len())?;
        chapters.push(make_chapter(
            id,
            chapters.len(),
            chapter_title.to_owned(),
            body,
        )?);
    }
    ensure!(
        !chapters.is_empty(),
        "EPUB has no readable unencrypted spine text"
    );
    finish_import(id, title, &author, &category, &intro, chapters)
}

fn encrypted_resources(reader: &mut EpubReader<'_>) -> Result<HashSet<String>> {
    if !reader.contains("META-INF/encryption.xml") {
        return Ok(HashSet::new());
    }
    let encryption = reader.read_text("META-INF/encryption.xml")?;
    let encryption = reader
        .xml_work
        .parse(&encryption, xml_work::Kind::Encryption)
        .context("parse EPUB encryption metadata")?;
    let mut resources = HashSet::new();
    for node in encryption
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "CipherReference")
    {
        let path = resolve_href(
            "",
            node.attribute("URI")
                .context("EPUB encrypted resource has no URI")?,
        )?;
        if !resources.contains(&path) {
            reader
                .budget
                .strings(&[&path], 2 * std::mem::size_of::<String>())?;
            resources.insert(path);
        }
    }
    Ok(resources)
}

#[cfg(test)]
fn nav_titles(
    html: &str,
    nav_path: &str,
    budget: &mut RetainedBudget,
) -> Result<HashMap<String, String>> {
    nav_titles_with_work(html, nav_path, budget, &mut nav_work::Budget::new())
}

fn nav_titles_with_work(
    html: &str,
    nav_path: &str,
    budget: &mut RetainedBudget,
    work: &mut nav_work::Budget,
) -> Result<HashMap<String, String>> {
    let document = Html::parse_document(html);
    // The locked selector walks ancestors for every candidate. A forest
    // projection carries nav ancestry once, preserves tree.nodes() order,
    // and prepays every label before any repeated subtree traversal or copy.
    let projection = nav_work::Projection::new_with_work(&document, nav_path, budget, work)?;
    let temporary_bytes = projection.metadata_bytes();
    let result = (|| -> Result<HashMap<String, String>> {
        let mut titles = HashMap::new();
        for anchor in projection.candidates() {
            let Some(href) = anchor.value().attr("href") else {
                continue;
            };
            let path = match resolve_href(nav_path, href) {
                Ok(path) => path,
                Err(error) if error.downcast_ref::<ImportLimitExceeded>().is_some() => {
                    return Err(error);
                }
                Err(_) => continue,
            };
            if titles.contains_key(&path) {
                continue;
            }
            let mut title = String::new();
            append_element_with_observer(
                anchor,
                &mut title,
                MAX_TITLE_BYTES,
                || nav_work::observe_label_visit(),
                || nav_work::observe_label_child_inspection(),
            )?;
            nav_work::observe_label_copy(title.len());
            let title = compact_whitespace(&title);
            if !title.is_empty() {
                budget.strings(
                    &[&path, &title],
                    2 * std::mem::size_of::<(String, String)>(),
                )?;
                titles.entry(path).or_insert(title);
                nav_work::observe_title();
            }
        }
        Ok(titles)
    })();
    // Only the projection and fixed temporary scratch were reserved here.
    // Retained title charges and admitted work never receive this refund.
    drop(projection);
    budget.used -= temporary_bytes;
    result
}

#[derive(Clone, Copy)]
struct NcxWorkLimits {
    node_visits: usize,
    label_bytes: usize,
}

impl Default for NcxWorkLimits {
    fn default() -> Self {
        Self {
            node_visits: NCX_NODE_VISITS_PER_NODE * MAX_NCX_NODES as usize,
            label_bytes: MAX_WORK_BYTES as usize,
        }
    }
}

struct NcxWorkBudget {
    remaining_node_visits: usize,
    label_bytes: usize,
    limits: NcxWorkLimits,
}

#[derive(Clone, Copy, Default)]
enum NcxFirstText<'a> {
    #[default]
    Missing,
    Found(Option<&'a str>),
}

impl NcxWorkBudget {
    fn new(nodes: usize, limits: NcxWorkLimits) -> Result<Self> {
        let node_visits = nodes
            .checked_mul(NCX_NODE_VISITS_PER_NODE)
            .filter(|visits| *visits <= limits.node_visits)
            .ok_or(ImportLimitExceeded(
                "NCX navigation node work exceeds its allowance",
            ))?;
        Ok(Self {
            remaining_node_visits: node_visits,
            label_bytes: 0,
            limits,
        })
    }

    fn visit(&mut self) -> Result<()> {
        self.remaining_node_visits =
            self.remaining_node_visits
                .checked_sub(1)
                .ok_or(ImportLimitExceeded(
                    "NCX navigation node work exceeds its allowance",
                ))?;
        #[cfg(test)]
        ncx_work_tests::observe_visit();
        Ok(())
    }

    fn admit_label(&mut self, bytes: usize) -> Result<()> {
        // A shared first-text descendant may be copied by many labels, even
        // when whitespace normalization produces no retained title at all.
        let fee = bytes.checked_mul(3).ok_or(ImportLimitExceeded(
            "NCX navigation label work exceeds its allowance",
        ))?;
        self.label_bytes = self
            .label_bytes
            .checked_add(fee)
            .filter(|used| *used <= self.limits.label_bytes)
            .ok_or(ImportLimitExceeded(
                "NCX navigation label work exceeds its allowance",
            ))?;
        #[cfg(test)]
        ncx_work_tests::observe_label_admission(bytes, fee);
        Ok(())
    }
}

fn ncx_node_index(id: roxmltree::NodeId) -> Result<usize> {
    usize::try_from(id.get()).context("NCX node identity exceeds the platform range")
}

#[cfg(test)]
fn ncx_titles(
    xml: &str,
    ncx_path: &str,
    budget: &mut RetainedBudget,
) -> Result<HashMap<String, String>> {
    #[cfg(test)]
    let limits = ncx_work_tests::configured_limits().unwrap_or_default();
    #[cfg(not(test))]
    let limits = NcxWorkLimits::default();
    ncx_titles_with_limits(xml, ncx_path, budget, limits)
}

#[cfg(test)]
fn ncx_titles_with_limits(
    xml: &str,
    ncx_path: &str,
    budget: &mut RetainedBudget,
    limits: NcxWorkLimits,
) -> Result<HashMap<String, String>> {
    ncx_titles_with_limits_and_xml_budget(
        xml,
        ncx_path,
        budget,
        limits,
        &mut xml_work::Budget::new(),
    )
}

fn ncx_titles_with_xml_budget(
    xml: &str,
    ncx_path: &str,
    budget: &mut RetainedBudget,
    xml_budget: &mut xml_work::Budget,
) -> Result<HashMap<String, String>> {
    #[cfg(test)]
    let limits = ncx_work_tests::configured_limits().unwrap_or_default();
    #[cfg(not(test))]
    let limits = NcxWorkLimits::default();
    ncx_titles_with_limits_and_xml_budget(xml, ncx_path, budget, limits, xml_budget)
}

fn ncx_titles_with_limits_and_xml_budget(
    xml: &str,
    ncx_path: &str,
    budget: &mut RetainedBudget,
    limits: NcxWorkLimits,
    xml_budget: &mut xml_work::Budget,
) -> Result<HashMap<String, String>> {
    let input = xml_budget.input(xml, xml_work::Kind::Ncx)?;
    let admitted = ncx_entities::preflight(xml, budget)?;
    xml_budget.fields(input, admitted.fields)?;
    xml_work::observe_dom(xml_work::Kind::Ncx);
    #[cfg(test)]
    ncx_entity_budget_tests::observe_dom_parse();
    // EPUB 2 NCX commonly declares an external DTD. No external entity resolver is installed.
    let document = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            nodes_limit: MAX_NCX_NODES,
            entity_resolver: None,
        },
    )
    .context("parse EPUB NCX navigation")?;
    let nodes = document.descendants().len();
    let projection_bytes = nodes
        .checked_mul(std::mem::size_of::<NcxFirstText<'_>>())
        .and_then(|bytes| bytes.checked_add(std::mem::size_of::<Vec<NcxFirstText<'_>>>()))
        .ok_or(ImportLimitExceeded(
            "NCX navigation projection exceeds its metadata allowance",
        ))?;
    #[cfg(test)]
    ncx_work_tests::observe_preflight(nodes, nodes * NCX_NODE_VISITS_PER_NODE, projection_bytes);
    // Both allowances precede the only projection allocation. Across the two
    // node passes, each tree edge is visited at most three times. Constant-time
    // first-child text probes and label lookups apply to disjoint element
    // kinds, so the complete operation uses fewer than six visits per node.
    let mut work = NcxWorkBudget::new(nodes, limits)?;
    budget.reserve(&[projection_bytes])?;
    #[cfg(test)]
    ncx_work_tests::observe_reservation(nodes * NCX_NODE_VISITS_PER_NODE, projection_bytes);
    let result = (|| -> Result<HashMap<String, String>> {
        // Node IDs follow document order. Reverse traversal can reuse each
        // child's first matching ELEMENT <text> without rescanning its subtree.
        // Each text element is read once. Found(None) keeps an empty first
        // element authoritative instead of choosing a later text element.
        // Borrowed strings never duplicate a shared descendant's payload.
        let mut first_text = vec![NcxFirstText::Missing; nodes];
        #[cfg(test)]
        ncx_work_tests::observe_projection(first_text.len(), first_text.capacity());
        for node in document.descendants().rev() {
            work.visit()?;
            let first = if node.is_element() && node.tag_name().name() == "text" {
                work.visit()?;
                let text = node.text();
                #[cfg(test)]
                ncx_work_tests::observe_text_read();
                NcxFirstText::Found(text)
            } else {
                let mut first = NcxFirstText::Missing;
                for child in node.children() {
                    work.visit()?;
                    let candidate = *first_text
                        .get(ncx_node_index(child.id())?)
                        .context("NCX child is absent from its projection")?;
                    if !matches!(candidate, NcxFirstText::Missing) {
                        first = candidate;
                        break;
                    }
                }
                first
            };
            *first_text
                .get_mut(ncx_node_index(node.id())?)
                .context("NCX node is absent from its projection")? = first;
        }

        let mut titles = HashMap::new();
        for point in document.descendants() {
            work.visit()?;
            if !point.is_element() || point.tag_name().name() != "navPoint" {
                continue;
            }
            let mut target = None;
            for child in point.children() {
                work.visit()?;
                if child.is_element() && child.tag_name().name() == "content" {
                    target = child.attribute("src");
                    break;
                }
            }
            let mut label = None;
            for child in point.children() {
                work.visit()?;
                if child.is_element() && child.tag_name().name() == "navLabel" {
                    work.visit()?;
                    if let NcxFirstText::Found(text) = *first_text
                        .get(ncx_node_index(child.id())?)
                        .context("NCX label is absent from its projection")?
                    {
                        label = text;
                    }
                    break;
                }
            }
            if let (Some(target), Some(label)) = (target, label) {
                let path = resolve_href(ncx_path, target)?;
                if titles.contains_key(&path) {
                    continue;
                }
                check_title_length(label.len())?;
                work.admit_label(label.len())?;
                let mut bounded_label = String::new();
                append_limited(
                    &mut bounded_label,
                    label,
                    MAX_TITLE_BYTES,
                    "import title exceeds the 16 KiB limit",
                )?;
                #[cfg(test)]
                ncx_work_tests::observe_label_copy(bounded_label.len());
                #[cfg(test)]
                ncx_entity_budget_tests::observe_navigation_copy(bounded_label.len());
                let label = compact_whitespace(&bounded_label);
                if !label.is_empty() {
                    budget.strings(
                        &[&path, &label],
                        2 * std::mem::size_of::<(String, String)>(),
                    )?;
                    titles.entry(path).or_insert(label);
                    #[cfg(test)]
                    ncx_work_tests::observe_title();
                }
            }
        }
        Ok(titles)
    })();
    // The projection was dropped with the closure. Release only its temporary
    // retained-data reservation; node and label work stay spent for this call.
    budget.used -= projection_bytes;
    result
}

#[cfg(test)]
fn extract_html(html: &str, need_heading: bool) -> Result<(String, Option<String>)> {
    extract_html_with_work(
        html,
        need_heading,
        &mut RetainedBudget::new(),
        &mut nav_work::Budget::new(),
    )
}

fn extract_html_with_work(
    html: &str,
    need_heading: bool,
    retained: &mut RetainedBudget,
    work: &mut nav_work::Budget,
) -> Result<(String, Option<String>)> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("body").expect("constant selector");
    let body = document
        .select(&selector)
        .next()
        .unwrap_or_else(|| document.root_element());
    let mut text = String::new();
    append_element(body, &mut text, MAX_BOOK_BYTES as usize)?;
    let mut normalized = String::new();
    for line in text.lines() {
        let line = compact_whitespace(line);
        if !line.is_empty() {
            if !normalized.is_empty() {
                append_limited(
                    &mut normalized,
                    "\n",
                    MAX_BOOK_BYTES as usize,
                    "EPUB extracted text exceeds the 32 MiB book limit",
                )?;
            }
            append_limited(
                &mut normalized,
                &line,
                MAX_BOOK_BYTES as usize,
                "EPUB extracted text exceeds the 32 MiB book limit",
            )?;
        }
    }
    if !need_heading {
        return Ok((normalized, None));
    }
    let found_heading = nav_work::heading_title(body, retained, work)?;
    Ok((normalized, found_heading))
}

fn append_element(element: ElementRef<'_>, output: &mut String, maximum: usize) -> Result<()> {
    append_element_with_observer(element, output, maximum, || {}, || {})
}

fn append_element_suppressed(element: ElementRef<'_>) -> bool {
    matches!(
        element.value().name(),
        "script"
            | "style"
            | "head"
            | "template"
            | "noscript"
            | "svg"
            | "math"
            | "iframe"
            | "object"
            | "audio"
            | "video"
    ) || element.value().attr("hidden").is_some()
        || element.value().attr("aria-hidden") == Some("true")
}

fn append_element_spacing(name: &str) -> (bool, Option<char>) {
    if matches!(
        name,
        "p" | "div"
            | "section"
            | "article"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "li"
            | "blockquote"
            | "pre"
            | "tr"
            | "br"
            | "hr"
    ) {
        (true, Some('\n'))
    } else if matches!(name, "td" | "th") {
        (false, Some(' '))
    } else {
        (false, None)
    }
}

enum AppendElementWork<'a> {
    Element(ElementRef<'a>),
    Text(&'a str),
    Space(char),
}

fn append_element_with_observer(
    element: ElementRef<'_>,
    output: &mut String,
    maximum: usize,
    mut visit: impl FnMut(),
    mut inspect_child: impl FnMut(),
) -> Result<()> {
    let limit_message = if maximum == MAX_TITLE_BYTES {
        "import title exceeds the 16 KiB limit"
    } else {
        "EPUB extracted text exceeds the 32 MiB book limit"
    };
    let mut pending = vec![AppendElementWork::Element(element)];
    while let Some(work) = pending.pop() {
        visit();
        let element = match work {
            AppendElementWork::Text(text) => {
                append_limited(output, text, maximum, limit_message)?;
                continue;
            }
            AppendElementWork::Space(space) => {
                if output
                    .len()
                    .checked_add(space.len_utf8())
                    .is_none_or(|length| length > maximum)
                {
                    return Err(ImportLimitExceeded(limit_message).into());
                }
                output.push(space);
                continue;
            }
            AppendElementWork::Element(element) => element,
        };
        let name = element.value().name();
        if append_element_suppressed(element) {
            continue;
        }
        let (block, trailing) = append_element_spacing(name);
        if block {
            if output.len() >= maximum {
                return Err(ImportLimitExceeded(limit_message).into());
            }
            output.push('\n');
        }
        if let Some(space) = trailing {
            pending.push(AppendElementWork::Space(space));
        }
        for child in element.children().rev() {
            inspect_child();
            if let Some(text) = child.value().as_text() {
                pending.push(AppendElementWork::Text(text));
            } else if let Some(child) = ElementRef::wrap(child) {
                pending.push(AppendElementWork::Element(child));
            }
        }
    }
    Ok(())
}

fn compact_whitespace(text: &str) -> String {
    let mut compacted = String::new();
    for word in text.split_whitespace() {
        if !compacted.is_empty() {
            compacted.push(' ');
        }
        compacted.push_str(word);
    }
    compacted
}

fn make_chapter(book_id: &str, index: usize, title: String, body: String) -> Result<Chapter> {
    ensure!(index < MAX_ENTRIES, "import contains too many chapters");
    ensure!(
        !body.trim().is_empty(),
        "import chapter has no readable body"
    );
    Ok(Chapter {
        id: format!("{book_id}:{index}"),
        book_id: book_id.to_owned(),
        index: index as u32,
        title,
        cached: true,
        body: Some(body),
        content_status: crate::domain::ContentStatus::Full,
        ..Chapter::default()
    })
}

fn finish_import(
    id: &str,
    title: &str,
    author: &str,
    category: &str,
    intro: &str,
    chapters: Vec<Chapter>,
) -> Result<ImportedBook> {
    ensure!(!chapters.is_empty(), "import has no readable chapters");
    let count = u32::try_from(chapters.len())?;
    Ok(ImportedBook {
        book: Book {
            id: id.to_owned(),
            title: title.to_owned(),
            author: author.to_owned(),
            category: category.to_owned(),
            intro: intro.to_owned(),
            source: "local".into(),
            total_chapters: count,
            cached_chapters: count,
            on_shelf: true,
            ..Book::default()
        },
        chapters,
    })
}

/// Resolve URI fragments and escapes before applying POSIX archive-root bounds.
fn resolve_href(base_file: &str, href: &str) -> Result<String> {
    if base_file.len() > MAX_PATH_BYTES {
        return Err(ImportLimitExceeded("EPUB resource path exceeds the 4096 byte limit").into());
    }
    let href = href.split(['#', '?']).next().unwrap_or_default();
    if href.is_empty() {
        ensure!(!base_file.is_empty(), "EPUB resource reference has no path");
        return normalize_path(base_file);
    }
    // Unsupported external links are never materialized as local metadata.
    // Keep their existing navigation fallback before allocating decoded bytes.
    ensure!(
        !href.split('/').next().unwrap_or_default().contains(':'),
        "EPUB external resource URLs are unsupported"
    );
    let href = decode_percent(href)?;
    ensure!(
        !href.starts_with('/') && !href.starts_with('\\'),
        "EPUB resource path is absolute"
    );
    ensure!(
        !href.split('/').next().unwrap_or_default().contains(':'),
        "EPUB external resource URLs are unsupported"
    );
    let parent = base_file.rsplit_once('/').map_or("", |(parent, _)| parent);
    let joined_bytes = parent
        .len()
        .checked_add(usize::from(!parent.is_empty()))
        .and_then(|length| length.checked_add(href.len()));
    if joined_bytes.is_none_or(|length| length > MAX_PATH_BYTES) {
        return Err(ImportLimitExceeded("EPUB resource path exceeds the 4096 byte limit").into());
    }
    let joined = if parent.is_empty() {
        href
    } else {
        format!("{parent}/{href}")
    };
    normalize_path(&joined)
}

fn normalize_path(path: &str) -> Result<String> {
    if path.len() > MAX_PATH_BYTES {
        return Err(ImportLimitExceeded("EPUB resource path exceeds the 4096 byte limit").into());
    }
    ensure!(!path.is_empty(), "invalid EPUB resource path length");
    ensure!(
        !path.starts_with('/') && !path.contains('\\'),
        "invalid EPUB resource path separator"
    );
    ensure!(
        !path.chars().any(char::is_control),
        "EPUB resource path contains a control character"
    );
    let mut segments = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                ensure!(
                    segments.pop().is_some(),
                    "EPUB resource path escapes the archive root"
                );
            }
            _ => {
                ensure!(
                    !segment.contains(':'),
                    "invalid EPUB resource path component"
                );
                segments.push(segment);
            }
        }
    }
    ensure!(
        !segments.is_empty(),
        "EPUB resource path resolves to the archive root"
    );
    Ok(segments.join("/"))
}

fn decode_percent(value: &str) -> Result<String> {
    if value.len() > MAX_PATH_BYTES * 3 {
        return Err(
            ImportLimitExceeded("EPUB encoded resource path exceeds its retained limit").into(),
        );
    }
    let mut result = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(byte) = bytes.next() {
        if result.len() >= MAX_PATH_BYTES {
            return Err(
                ImportLimitExceeded("EPUB resource path exceeds the 4096 byte limit").into(),
            );
        }
        if byte == b'%' {
            let high = bytes
                .next()
                .and_then(hex_digit)
                .context("invalid EPUB percent escape")?;
            let low = bytes
                .next()
                .and_then(hex_digit)
                .context("invalid EPUB percent escape")?;
            result.push(high * 16 + low);
        } else {
            result.push(byte);
        }
    }
    String::from_utf8(result).context("EPUB resource URI is not UTF-8")
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
#[path = "import_deep_tests.rs"]
mod deep_tests;

#[cfg(test)]
#[path = "import_zip_work_tests.rs"]
mod zip_work_tests;

#[cfg(test)]
#[path = "import_ncx_entity_budget_tests.rs"]
mod ncx_entity_budget_tests;

#[cfg(test)]
#[path = "import_xml_work_tests.rs"]
mod xml_work_tests;

#[cfg(test)]
#[path = "import_ncx_work_tests.rs"]
mod ncx_work_tests;

#[cfg(test)]
#[path = "import_nav_work_tests.rs"]
mod nav_work_tests;

#[cfg(test)]
#[path = "import_heading_work_tests.rs"]
mod heading_work_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        io::Write,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };
    use zip::{ZipWriter, write::SimpleFileOptions};

    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn temporary_file(extension: &str, bytes: &[u8]) -> Result<PathBuf> {
        let path = std::env::temp_dir().join(format!(
            "weread-import-{}-{}.{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            extension
        ));
        fs::write(&path, bytes)?;
        Ok(path)
    }

    pub(super) fn archive(entries: &[(&str, &[u8])]) -> Result<Vec<u8>> {
        let mut output = ZipWriter::new(Cursor::new(Vec::new()));
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in entries {
            output.start_file(*name, options)?;
            output.write_all(body)?;
        }
        Ok(output.finish()?.into_inner())
    }

    fn zip32_footer(bytes: &[u8]) -> usize {
        let footer = bytes
            .len()
            .checked_sub(22)
            .expect("ZIP fixture has no footer");
        assert_eq!(&bytes[footer..footer + 4], b"PK\x05\x06");
        assert_eq!(&bytes[footer + 20..footer + 22], &[0, 0]);
        footer
    }

    fn empty_deflate_zip_fixture(blocks: usize) -> Result<Vec<u8>> {
        assert!(blocks > 0);
        let mut compressed = Vec::new();
        for index in 0..blocks {
            // RFC 1951 stored blocks: BFINAL, zero alignment/LEN, one's-complement NLEN.
            compressed.extend_from_slice(&[u8::from(index + 1 == blocks), 0, 0, 0xff, 0xff]);
        }
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        writer.start_file(
            "chapter.xhtml",
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
        )?;
        writer.write_all(&compressed)?;
        let mut bytes = writer.finish()?.into_inner();
        let footer = zip32_footer(&bytes);
        let central =
            u32::from_le_bytes(bytes[footer + 16..footer + 20].try_into().unwrap()) as usize;
        let local =
            u32::from_le_bytes(bytes[central + 42..central + 46].try_into().unwrap()) as usize;
        assert_eq!(&bytes[local..local + 4], b"PK\x03\x04");
        assert_eq!(
            u16::from_le_bytes(bytes[local + 6..local + 8].try_into().unwrap()) & 8,
            0
        );
        bytes[local + 4..local + 6].copy_from_slice(&20_u16.to_le_bytes());
        bytes[local + 8..local + 10].copy_from_slice(&8_u16.to_le_bytes());
        bytes[local + 14..local + 18].copy_from_slice(&0_u32.to_le_bytes());
        bytes[local + 22..local + 26].copy_from_slice(&0_u32.to_le_bytes());
        bytes[central + 6..central + 8].copy_from_slice(&20_u16.to_le_bytes());
        bytes[central + 10..central + 12].copy_from_slice(&8_u16.to_le_bytes());
        bytes[central + 16..central + 20].copy_from_slice(&0_u32.to_le_bytes());
        bytes[central + 24..central + 28].copy_from_slice(&0_u32.to_le_bytes());
        Ok(bytes)
    }

    #[test]
    fn repeated_empty_deflate_reads_consume_compressed_work_before_output() -> Result<()> {
        let bytes = empty_deflate_zip_fixture(3)?;
        let mut reader = EpubReader::new(&bytes)?;
        let compressed = reader.archive.by_name("chapter.xhtml")?.compressed_size();
        assert_eq!(compressed, 15);
        assert_eq!(reader.compressed_work_bytes, 0);
        reader.compressed_work_bytes = MAX_COMPRESSED_WORK_BYTES - 2 * compressed;
        assert_eq!(reader.read_text("chapter.xhtml")?, "");
        let retained = reader.budget.used;
        assert_eq!(reader.read_text("chapter.xhtml")?, "");
        assert_eq!(reader.compressed_work_bytes, MAX_COMPRESSED_WORK_BYTES);
        assert_eq!(reader.read_work_bytes, 0);
        assert_eq!(reader.decoded_work_bytes, 0);
        assert_eq!(reader.read_bytes, 0);
        assert_eq!(reader.read_names.len(), 1);
        assert_eq!(reader.budget.used, retained);
        let error = reader.read_text("chapter.xhtml").unwrap_err();
        assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
        assert!(error.to_string().contains("cumulative compressed work"));
        assert_eq!(reader.compressed_work_bytes, MAX_COMPRESSED_WORK_BYTES);
        assert_eq!(reader.read_work_bytes, 0);
        assert_eq!(reader.decoded_work_bytes, 0);
        Ok(())
    }

    #[test]
    fn rejected_compressed_reservation_does_not_spend_plain_or_decoded_work() -> Result<()> {
        let bytes = archive(&[("chapter.txt", b"Complete fixture")])?;
        let mut reader = EpubReader::new(&bytes)?;
        let compressed = reader.archive.by_name("chapter.txt")?.compressed_size();
        assert!(compressed > 0);
        reader.compressed_work_bytes = MAX_COMPRESSED_WORK_BYTES - compressed + 1;
        reader.read_work_bytes = 10;
        reader.decoded_work_bytes = 20;
        let previous = reader.compressed_work_bytes;
        let error = reader.read_text("chapter.txt").unwrap_err();
        assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
        assert!(error.to_string().contains("cumulative compressed work"));
        assert_eq!(reader.compressed_work_bytes, previous);
        assert_eq!(reader.read_work_bytes, 10);
        assert_eq!(reader.decoded_work_bytes, 20);
        assert_eq!(reader.read_bytes, 0);
        assert!(reader.read_names.is_empty());
        Ok(())
    }

    fn assert_file_count_limit(error: &zip::result::ZipError) {
        assert!(
            matches!(
                error,
                zip::result::ZipError::UnsupportedArchive(
                    zip::result::ZipError::FILE_COUNT_LIMIT_EXCEEDED
                )
            ),
            "Expected pre-index admission failure, got {error}"
        );
    }

    fn zip64_fixture(mut bytes: Vec<u8>, comment: &[u8]) -> Vec<u8> {
        let footer = zip32_footer(&bytes);
        let mut original_footer = bytes[footer..].to_vec();
        let count = u16::from_le_bytes(original_footer[10..12].try_into().unwrap()) as u64;
        let central_size = u32::from_le_bytes(original_footer[12..16].try_into().unwrap()) as u64;
        let central_offset = u32::from_le_bytes(original_footer[16..20].try_into().unwrap()) as u64;
        bytes.truncate(footer);
        bytes.extend_from_slice(b"PK\x06\x06");
        bytes.extend_from_slice(&44_u64.to_le_bytes());
        bytes.extend_from_slice(&45_u16.to_le_bytes());
        bytes.extend_from_slice(&45_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&central_size.to_le_bytes());
        bytes.extend_from_slice(&central_offset.to_le_bytes());
        bytes.extend_from_slice(b"PK\x06\x07");
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&(footer as u64).to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        original_footer[8..10].copy_from_slice(&u16::MAX.to_le_bytes());
        original_footer[10..12].copy_from_slice(&u16::MAX.to_le_bytes());
        original_footer[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
        original_footer[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        original_footer[20..22]
            .copy_from_slice(&u16::try_from(comment.len()).unwrap().to_le_bytes());
        bytes.extend_from_slice(&original_footer);
        bytes.extend_from_slice(comment);
        bytes
    }

    #[derive(Debug)]
    struct CentralReadProbe<'a> {
        bytes: Cursor<&'a [u8]>,
        central_offset: u64,
        entry_reads: std::rc::Rc<std::cell::Cell<usize>>,
    }

    impl Read for CentralReadProbe<'_> {
        fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
            // The pinned parser probes signatures separately. A 46-byte read
            // at this offset is its fixed central-entry header, not EOCD search.
            if self.bytes.position() == self.central_offset && output.len() == 46 {
                self.entry_reads.set(self.entry_reads.get() + 1);
            }
            self.bytes.read(output)
        }
    }

    impl std::io::Seek for CentralReadProbe<'_> {
        fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
            std::io::Seek::seek(&mut self.bytes, position)
        }
    }

    #[test]
    fn bounded_archive_initialization_rejects_too_many_central_records() -> Result<()> {
        let mut output = ZipWriter::new(Cursor::new(Vec::new()));
        let options =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for index in 0..=MAX_ENTRIES {
            output.start_file(format!("entry-{index}"), options)?;
        }
        let bytes = output.finish()?.into_inner();
        assert!(bytes.len() < 1024 * 1024);
        let error = EpubReader::new(&bytes)
            .err()
            .expect("Oversized directory was admitted");
        assert_file_count_limit(error.downcast_ref::<zip::result::ZipError>().unwrap());
        Ok(())
    }

    #[test]
    fn bounded_archive_initialization_preserves_safe_legacy_reads_and_empty_archives() -> Result<()>
    {
        let bytes = archive(&[("first.txt", b"First"), ("second.txt", b"Second")])?;
        let mut legacy = ZipArchive::new(Cursor::new(bytes.as_slice()))?;
        let mut bounded = ZipArchive::new_with_file_limit(Cursor::new(bytes.as_slice()), 2)?;
        assert_eq!(legacy.len(), 2);
        assert_eq!(bounded.len(), legacy.len());
        for name in ["first.txt", "second.txt"] {
            let mut original = String::new();
            legacy.by_name(name)?.read_to_string(&mut original)?;
            let mut admitted = String::new();
            bounded.by_name(name)?.read_to_string(&mut admitted)?;
            assert_eq!(admitted, original);
        }
        assert_file_count_limit(
            &ZipArchive::new_with_file_limit(Cursor::new(bytes.as_slice()), 1).unwrap_err(),
        );
        let empty = archive(&[])?;
        assert!(ZipArchive::new_with_file_limit(Cursor::new(empty.as_slice()), 0)?.is_empty());
        let malformed = b"This is not a ZIP archive";
        let original = ZipArchive::new(Cursor::new(malformed))
            .unwrap_err()
            .to_string();
        let admitted = ZipArchive::new_with_file_limit(Cursor::new(malformed), 2)
            .unwrap_err()
            .to_string();
        assert_eq!(admitted, original);
        Ok(())
    }

    #[test]
    fn a_large_declared_directory_is_rejected_before_reading_central_entries() -> Result<()> {
        let mut bytes = archive(&[("fixture.txt", b"Complete fixture")])?;
        let footer = zip32_footer(&bytes);
        let central_offset =
            u32::from_le_bytes(bytes[footer + 16..footer + 20].try_into().unwrap()) as usize;
        // Keep the declared count below the directory offset: the unbounded
        // parser would reserve this count before discovering truncated records.
        let padding = 70 * 1024;
        bytes.splice(
            central_offset..central_offset,
            std::iter::repeat_n(0, padding),
        );
        let central_offset = central_offset + padding;
        let footer = footer + padding;
        bytes[footer + 8..footer + 10].copy_from_slice(&u16::MAX.to_le_bytes());
        bytes[footer + 10..footer + 12].copy_from_slice(&u16::MAX.to_le_bytes());
        bytes[footer + 16..footer + 20].copy_from_slice(&(central_offset as u32).to_le_bytes());
        let entry_reads = std::rc::Rc::new(std::cell::Cell::new(0));
        let reader = CentralReadProbe {
            bytes: Cursor::new(bytes.as_slice()),
            central_offset: central_offset as u64,
            entry_reads: entry_reads.clone(),
        };
        // Never feed this deliberately large declaration to the legacy API.
        let error = ZipArchive::new_with_file_limit(reader, MAX_ENTRIES).unwrap_err();
        assert_file_count_limit(&error);
        assert_eq!(entry_reads.get(), 0);
        Ok(())
    }

    #[test]
    fn duplicate_paths_consume_the_file_limit_before_index_deduplication() -> Result<()> {
        let mut bytes = archive(&[("same.txt", b"One payload")])?;
        let footer = zip32_footer(&bytes);
        let central_offset =
            u32::from_le_bytes(bytes[footer + 16..footer + 20].try_into().unwrap()) as usize;
        let central = bytes[central_offset..footer].to_vec();
        let central_size = central.len();
        bytes.splice(footer..footer, central);
        let footer = footer + central_size;
        bytes[footer + 8..footer + 10].copy_from_slice(&2_u16.to_le_bytes());
        bytes[footer + 10..footer + 12].copy_from_slice(&2_u16.to_le_bytes());
        bytes[footer + 12..footer + 16].copy_from_slice(&((central_size * 2) as u32).to_le_bytes());
        // This legacy comparison contains only two physical records.
        assert_eq!(ZipArchive::new(Cursor::new(bytes.as_slice()))?.len(), 1);
        assert_file_count_limit(
            &ZipArchive::new_with_file_limit(Cursor::new(bytes.as_slice()), 1).unwrap_err(),
        );
        assert_eq!(
            ZipArchive::new_with_file_limit(Cursor::new(bytes.as_slice()), 2)?.len(),
            1
        );
        Ok(())
    }

    #[test]
    fn bounded_archive_initialization_preserves_zip64_sfx_and_comments() -> Result<()> {
        let bytes = archive(&[("first.txt", b"First"), ("second.txt", b"Second")])?;
        let comment = b"Preserved ZIP64 fixture comment";
        let bytes = zip64_fixture(bytes, comment);
        let mut prefixed = b"Self-extracting fixture prefix\n".to_vec();
        let prefix_bytes = prefixed.len();
        prefixed.extend_from_slice(&bytes);
        let legacy = ZipArchive::new(Cursor::new(prefixed.as_slice()))?;
        let mut bounded = ZipArchive::new_with_file_limit(Cursor::new(prefixed.as_slice()), 2)?;
        assert_eq!(legacy.len(), 2);
        assert_eq!(bounded.len(), legacy.len());
        assert_eq!(bounded.comment(), comment);
        assert_eq!(bounded.comment(), legacy.comment());
        let explicit = ZipArchive::with_config(
            zip::read::Config {
                archive_offset: zip::read::ArchiveOffset::Known(prefix_bytes as u64),
            },
            Cursor::new(prefixed.as_slice()),
        )?;
        assert_eq!(explicit.len(), legacy.len());
        let mut body = String::new();
        bounded.by_name("second.txt")?.read_to_string(&mut body)?;
        assert_eq!(body, "Second");
        assert_file_count_limit(
            &ZipArchive::new_with_file_limit(Cursor::new(prefixed.as_slice()), 1).unwrap_err(),
        );
        Ok(())
    }

    #[test]
    fn an_oversized_eocd_in_a_comment_can_fall_back_to_the_real_directory() -> Result<()> {
        let mut bytes = archive(&[("first.txt", b"First"), ("second.txt", b"Second")])?;
        let footer = zip32_footer(&bytes);
        let mut fake_footer = bytes[footer..].to_vec();
        // Three is deliberately safe even for the unbounded comparison. Its
        // third record is invalid; the real two-record EOCD remains earlier.
        fake_footer[8..10].copy_from_slice(&3_u16.to_le_bytes());
        fake_footer[10..12].copy_from_slice(&3_u16.to_le_bytes());
        bytes[footer + 20..footer + 22].copy_from_slice(&(fake_footer.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&fake_footer);
        let legacy = ZipArchive::new(Cursor::new(bytes.as_slice()))?;
        let bounded = ZipArchive::new_with_file_limit(Cursor::new(bytes.as_slice()), 2)?;
        assert_eq!(legacy.len(), 2);
        assert_eq!(bounded.len(), legacy.len());
        assert_eq!(bounded.comment(), fake_footer.as_slice());
        assert_eq!(bounded.comment(), legacy.comment());
        Ok(())
    }

    pub(super) fn container() -> &'static [u8] {
        br#"<?xml version="1.0"?><container><rootfiles><rootfile full-path="OEBPS/book.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#
    }

    fn package() -> &'static [u8] {
        br#"<package xmlns:dc="http://purl.org/dc/elements/1.1/"><metadata><dc:title>Fixture title</dc:title><dc:creator>Fixture author</dc:creator></metadata><manifest><item id="first" href="Text/first.xhtml" media-type="application/xhtml+xml"/><item id="second" href="Text/second%20part.xhtml" media-type="application/xhtml+xml"/><item id="nav" href="Nav/toc.xhtml" properties="nav" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="second"/><itemref idref="first"/></spine></package>"#
    }

    fn fixture(extra: &[(&str, &[u8])]) -> Result<Vec<u8>> {
        let mut entries = vec![
            ("mimetype", b"application/epub+zip".as_slice()),
            ("META-INF/container.xml", container()),
            ("OEBPS/book.opf", package()),
            ("OEBPS/Nav/toc.xhtml", b"<html><body><nav><a href='../Text/second%20part.xhtml#start'>Second from navigation</a><a href='../Text/first.xhtml'>First from navigation</a></nav></body></html>"),
            ("OEBPS/Text/first.xhtml", b"<html><head><style>invisible-css</style></head><body><h1>First heading</h1><p>First body &amp; text.</p><script>invisible-script</script></body></html>"),
            ("OEBPS/Text/second part.xhtml", b"<html><body><h1>Second heading</h1><p>Second body.</p><div hidden>invisible-hidden</div></body></html>"),
        ];
        entries.extend_from_slice(extra);
        archive(&entries)
    }

    fn repeated_spine_fixture(references: usize, title: &str) -> Result<Vec<u8>> {
        let package = format!(
            "<package><metadata><title>Repeated fixture</title></metadata><manifest><item id='chapter' href='chapter.xhtml' media-type='application/xhtml+xml'/><item id='nav' href='nav.xhtml' properties='nav' media-type='application/xhtml+xml'/></manifest><spine>{}</spine></package>",
            "<itemref idref='chapter'/>".repeat(references)
        );
        let nav =
            format!("<html><body><nav><a href='chapter.xhtml'>{title}</a></nav></body></html>");
        archive(&[
            ("META-INF/container.xml", container()),
            ("OEBPS/book.opf", package.as_bytes()),
            ("OEBPS/nav.xhtml", nav.as_bytes()),
            ("OEBPS/chapter.xhtml", b"<html><body><p>z</p></body></html>"),
        ])
    }

    #[test]
    fn repeated_spine_titles_count_every_retained_chapter_copy() -> Result<()> {
        let bytes = repeated_spine_fixture(MAX_ENTRIES, &"T".repeat(MAX_TITLE_BYTES / 2))?;
        assert!(bytes.len() < 64 * 1024);
        let error = import_epub(&bytes, "fixture-book", "Repeated fixture").unwrap_err();
        assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
        assert!(error.to_string().contains("64 MiB"));
        Ok(())
    }

    #[test]
    fn oversized_navigation_title_is_rejected_before_spine_repetition() -> Result<()> {
        let bytes = repeated_spine_fixture(MAX_ENTRIES, &"T".repeat(TEXT_CHAPTER_BYTES))?;
        let error = import_epub(&bytes, "fixture-book", "Repeated fixture").unwrap_err();
        assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
        assert!(error.to_string().contains("16 KiB"));
        Ok(())
    }

    #[test]
    fn repeated_empty_script_spine_cannot_bypass_the_cumulative_work_budget() -> Result<()> {
        let package = format!(
            "<package><metadata><title>Empty markup fixture</title></metadata><manifest><item id='chapter' href='chapter.xhtml' media-type='application/xhtml+xml'/></manifest><spine>{}</spine></package>",
            "<itemref idref='chapter'/>".repeat(MAX_ENTRIES)
        );
        let mut markup = Vec::with_capacity(MAX_ENTRY_BYTES as usize);
        markup.extend_from_slice(b"<html><body><script>");
        let closing = b"</script></body></html>";
        markup.resize(MAX_ENTRY_BYTES as usize - closing.len(), b'x');
        markup.extend_from_slice(closing);
        let bytes = archive(&[
            ("META-INF/container.xml", container()),
            ("OEBPS/book.opf", package.as_bytes()),
            ("OEBPS/chapter.xhtml", &markup),
        ])?;
        let error = import_epub(&bytes, "fixture-book", "Empty markup fixture").unwrap_err();
        assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
        assert!(error.to_string().contains("cumulative read work"));
        Ok(())
    }

    #[test]
    fn cumulative_read_quota_is_checked_before_entry_decompression() -> Result<()> {
        let contents = b"Complete fixture";
        let bytes = archive(&[("chapter.txt", contents)])?;
        let mut reader = EpubReader::new(&bytes)?;
        reader.read_work_bytes = MAX_WORK_BYTES - contents.len() as u64 + 1;
        let previous = reader.read_work_bytes;
        let error = reader.read_text("chapter.txt").unwrap_err();
        assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
        assert!(error.to_string().contains("cumulative read work"));
        assert_eq!(reader.read_work_bytes, previous);
        assert!(reader.read_names.is_empty());
        assert_eq!(reader.read_bytes, 0);
        assert_eq!(reader.decoded_work_bytes, 0);
        Ok(())
    }

    #[test]
    fn empty_spine_references_are_counted_even_when_byte_work_is_zero() -> Result<()> {
        let package = format!(
            "<package><metadata><title>Empty entry fixture</title></metadata><manifest><item id='chapter' href='chapter.xhtml' media-type='application/xhtml+xml'/></manifest><spine>{}</spine></package>",
            "<itemref idref='chapter'/>".repeat(MAX_ENTRIES + 1)
        );
        let bytes = archive(&[
            ("META-INF/container.xml", container()),
            ("OEBPS/book.opf", package.as_bytes()),
            ("OEBPS/chapter.xhtml", b""),
        ])?;
        let error = import_epub(&bytes, "fixture-book", "Empty entry fixture").unwrap_err();
        assert!(
            error
                .to_string()
                .contains("spine contains too many entries")
        );
        Ok(())
    }

    #[test]
    fn decoded_work_preflight_counts_exact_utf8_bytes_for_utf16_input() -> Result<()> {
        let text = "\u{4e2d}\u{6587}".repeat(4);
        let mut encoded = vec![0xFF, 0xFE];
        for unit in text.encode_utf16() {
            encoded.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decoded_epub_text_length(&encoded)?, text.len());
        let bytes = archive(&[("chapter.txt", &encoded)])?;
        let mut reader = EpubReader::new(&bytes)?;
        reader.decoded_work_bytes = MAX_WORK_BYTES - text.len() as u64 + 1;
        let previous = reader.decoded_work_bytes;
        let error = reader.read_text("chapter.txt").unwrap_err();
        assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
        assert!(error.to_string().contains("cumulative decoded work"));
        assert_eq!(reader.decoded_work_bytes, previous);
        assert_eq!(reader.read_work_bytes, encoded.len() as u64);
        Ok(())
    }

    #[test]
    fn consecutive_text_headings_are_bounded_before_split_title_amplification() {
        let heading = format!("Chapter 1 {}\n", "h".repeat(45));
        let mut text = heading.repeat(32_768);
        text.push_str(&"b".repeat(64 * TEXT_CHAPTER_BYTES));
        assert!(text.len() as u64 <= MAX_BOOK_BYTES);
        let error = import_text(text.as_bytes(), "fixture-book", "Text fixture").unwrap_err();
        assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
        assert!(error.to_string().contains("16 KiB"));
    }

    #[test]
    fn derived_budget_rejects_before_retaining_a_new_chapter() {
        let mut budget = RetainedBudget {
            used: MAX_RETAINED_BYTES - 1,
        };
        let before = budget.used;
        let mut chapters = Vec::new();
        let result = append_text_chapters(
            "fixture-book",
            "Chapter title",
            "Complete body",
            &mut chapters,
            &mut budget,
        );
        assert!(
            result
                .unwrap_err()
                .downcast_ref::<ImportLimitExceeded>()
                .is_some()
        );
        assert!(chapters.is_empty());
        assert_eq!(budget.used, before);
    }

    #[test]
    fn unicode_splits_preserve_complete_text_without_owned_intermediate_parts() -> Result<()> {
        let body = "\u{4e2d}\u{6587}".repeat(TEXT_CHAPTER_BYTES / 3 + 17);
        let imported = import_text(body.as_bytes(), "fixture-book", "Unicode fixture")?;
        assert!(imported.chapters.len() > 1);
        let restored = imported
            .chapters
            .iter()
            .map(|chapter| chapter.body.as_deref().unwrap())
            .collect::<String>();
        assert_eq!(restored, body);
        assert!(
            imported
                .chapters
                .iter()
                .all(|chapter| chapter.body.as_ref().unwrap().len() <= TEXT_CHAPTER_BYTES)
        );
        assert_eq!(imported.chapters[1].title, "Unicode fixture (2)");
        Ok(())
    }

    #[test]
    fn navigation_keys_and_titles_share_the_retained_metadata_budget() {
        let mut budget = RetainedBudget {
            used: MAX_RETAINED_BYTES - 32,
        };
        let nav = "<html><body><nav><a href='chapter.xhtml'>Title</a></nav></body></html>";
        let error = nav_titles(nav, "OEBPS/nav.xhtml", &mut budget).unwrap_err();
        assert!(error.downcast_ref::<ImportLimitExceeded>().is_some());
        assert!(error.to_string().contains("64 MiB"));
    }

    #[test]
    fn ncx_budget_errors_are_not_hidden_by_the_malformed_navigation_fallback() -> Result<()> {
        let package = b"<package><metadata><title>NCX fixture</title></metadata><manifest><item id='chapter' href='chapter.xhtml' media-type='application/xhtml+xml'/><item id='ncx' href='toc.ncx'/></manifest><spine toc='ncx'><itemref idref='chapter'/></spine></package>";
        let long_ncx = format!(
            "<ncx><navMap><navPoint><navLabel><text>{}</text></navLabel><content src='chapter.xhtml'/></navPoint></navMap></ncx>",
            "T".repeat(MAX_TITLE_BYTES + 1)
        );
        let chapter = b"<html><body><h1>Fallback heading</h1><p>Complete body</p></body></html>";
        for (ncx, accepted) in [(long_ncx.as_str(), false), ("<ncx>", true)] {
            let bytes = archive(&[
                ("META-INF/container.xml", container()),
                ("OEBPS/book.opf", package),
                ("OEBPS/toc.ncx", ncx.as_bytes()),
                ("OEBPS/chapter.xhtml", chapter),
            ])?;
            let result = import_epub(&bytes, "fixture-book", "NCX fixture");
            if accepted {
                assert_eq!(result?.chapters[0].title, "Fallback heading");
            } else {
                assert!(
                    result
                        .unwrap_err()
                        .downcast_ref::<ImportLimitExceeded>()
                        .is_some()
                );
            }
        }
        Ok(())
    }

    #[test]
    fn utf8_bom_text_imports_complete_chapters_with_content_identity() -> Result<()> {
        let bytes = "\u{feff}Chapter 1\r\nFirst body.\r\nChapter 2\r\n\u{4e2d}\u{6587}\u{6b63}\u{6587}\u{3002}\r\n".as_bytes();
        let path = temporary_file("txt", bytes)?;
        let imported = import_file(&path)?;
        assert_eq!(imported.chapters.len(), 2);
        assert_eq!(
            imported.chapters[1].body.as_deref(),
            Some("\u{4e2d}\u{6587}\u{6b63}\u{6587}\u{3002}")
        );
        assert_eq!(imported.book.cached_chapters, 2);
        assert_eq!(imported.book.source, "local");
        let renamed = temporary_file("TXT", bytes)?;
        assert_eq!(import_file(&renamed)?.book.id, imported.book.id);
        fs::remove_file(path)?;
        fs::remove_file(renamed)?;
        Ok(())
    }

    #[test]
    fn invalid_text_encoding_is_rejected() -> Result<()> {
        let path = temporary_file("txt", &[0xFF, 0xFE, 0x80])?;
        assert!(
            import_file(&path)
                .unwrap_err()
                .to_string()
                .contains("UTF-8")
        );
        fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn consecutive_text_headings_and_ordinary_book_sentences_are_preserved() -> Result<()> {
        let path = temporary_file("txt", b"Part One\nChapter 1\nBook reviews should remain ordinary body text.\nBook recommendations should also remain body text.\nChapter 2")?;
        let imported = import_file(&path)?;
        assert_eq!(imported.chapters[0].title, "Part One / Chapter 1");
        let body = imported.chapters[0].body.as_ref().unwrap();
        assert!(body.contains("Book reviews should remain ordinary body text."));
        assert!(body.contains("Book recommendations should also remain body text."));
        assert_eq!(imported.chapters[1].body.as_deref(), Some("Chapter 2"));
        fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn epub_follows_spine_and_decodes_navigation_paths_without_active_content() -> Result<()> {
        let path = temporary_file("epub", &fixture(&[])?)?;
        let imported = import_file(&path)?;
        assert_eq!(imported.book.title, "Fixture title");
        assert_eq!(imported.book.author, "Fixture author");
        assert_eq!(imported.chapters.len(), 2);
        assert_eq!(imported.chapters[0].title, "Second from navigation");
        assert!(
            imported.chapters[0]
                .body
                .as_ref()
                .unwrap()
                .contains("Second body.")
        );
        assert!(
            imported.chapters[1]
                .body
                .as_ref()
                .unwrap()
                .contains("First body & text.")
        );
        assert!(
            imported.chapters.iter().all(|chapter| !chapter
                .body
                .as_ref()
                .unwrap()
                .contains("invisible"))
        );
        fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn archive_and_uri_traversal_are_rejected() -> Result<()> {
        let path = temporary_file("epub", &fixture(&[("../outside", b"forbidden")])?)?;
        assert!(import_file(&path).is_err());
        assert!(resolve_href("OEBPS/book.opf", "../../outside.xhtml").is_err());
        assert!(resolve_href("OEBPS/book.opf", "%2e%2e/%2e%2e/outside.xhtml").is_err());
        assert!(resolve_href("OEBPS/book.opf", "https://example.com/book.xhtml").is_err());
        fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn malformed_package_xml_is_rejected() -> Result<()> {
        let bytes = archive(&[
            ("META-INF/container.xml", container()),
            ("OEBPS/book.opf", b"<package><unclosed>"),
        ])?;
        let path = temporary_file("epub", &bytes)?;
        assert!(
            import_file(&path)
                .unwrap_err()
                .to_string()
                .contains("package XML")
        );
        fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn epub2_ncx_with_standard_external_doctype_does_not_require_network() -> Result<()> {
        let package = br#"<package><metadata><title>EPUB 2 fixture</title></metadata><manifest><item id="chapter" href="chapter.xhtml" media-type="application/xhtml+xml"/><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/></manifest><spine toc="ncx"><itemref idref="chapter"/></spine></package>"#;
        let ncx = br#"<?xml version="1.0"?><!DOCTYPE ncx PUBLIC "-//NISO//DTD ncx 2005-1//EN" "http://www.daisy.org/z3986/2005/ncx-2005-1.dtd"><ncx><navMap><navPoint><navLabel><text>NCX chapter title</text></navLabel><content src="chapter.xhtml#start"/></navPoint></navMap></ncx>"#;
        let bytes = archive(&[
            ("META-INF/container.xml", container()),
            ("OEBPS/book.opf", package),
            ("OEBPS/toc.ncx", ncx),
            (
                "OEBPS/chapter.xhtml",
                b"<html><body><h1>Fallback heading</h1><p>Readable body.</p></body></html>",
            ),
        ])?;
        let path = temporary_file("epub", &bytes)?;
        assert_eq!(import_file(&path)?.chapters[0].title, "NCX chapter title");
        fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn aggregate_uncompressed_budget_is_enforced_across_small_entries() -> Result<()> {
        let filler = vec![b'x'; 7 * 1024 * 1024];
        let bytes = fixture(&[
            ("padding-1", &filler),
            ("padding-2", &filler),
            ("padding-3", &filler),
            ("padding-4", &filler),
            ("padding-5", &filler),
        ])?;
        let path = temporary_file("epub", &bytes)?;
        assert!(
            import_file(&path)
                .unwrap_err()
                .to_string()
                .contains("32 MiB")
        );
        fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn oversized_compressed_entry_is_rejected_before_body_decode() -> Result<()> {
        let large = vec![b'x'; MAX_ENTRY_BYTES as usize + 1];
        let bytes = fixture(&[("OEBPS/large.txt", &large)])?;
        assert!(bytes.len() < 1024 * 1024);
        let path = temporary_file("epub", &bytes)?;
        assert!(
            import_file(&path)
                .unwrap_err()
                .to_string()
                .contains("8 MiB")
        );
        fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn encrypted_body_is_rejected_but_unused_font_obfuscation_is_ignored() -> Result<()> {
        let encrypted = br#"<encryption><EncryptedData><EncryptionMethod Algorithm="example:unsupported"/><CipherData><CipherReference URI="OEBPS/Text/first.xhtml"/></CipherData></EncryptedData></encryption>"#;
        let path = temporary_file("epub", &fixture(&[("META-INF/encryption.xml", encrypted)])?)?;
        assert!(
            import_file(&path)
                .unwrap_err()
                .to_string()
                .contains("encrypted EPUB chapter")
        );
        fs::remove_file(path)?;
        let obfuscated_font = br#"<encryption><EncryptedData><EncryptionMethod Algorithm="http://www.idpf.org/2008/embedding"/><CipherData><CipherReference URI="OEBPS/Fonts/regular.otf"/></CipherData></EncryptedData></encryption>"#;
        let path = temporary_file(
            "epub",
            &fixture(&[
                ("META-INF/encryption.xml", obfuscated_font),
                ("OEBPS/Fonts/regular.otf", b"unused obfuscated font"),
            ])?,
        )?;
        assert_eq!(import_file(&path)?.chapters.len(), 2);
        fs::remove_file(path)?;
        Ok(())
    }
}
