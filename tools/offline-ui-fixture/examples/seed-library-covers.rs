//! Seed one disposable Scribe Library page through the production store and cache APIs.

use std::{fs, io::Cursor, path::PathBuf};

use anyhow::{Context, Result, ensure};
use clap::Parser;
use image::{DynamicImage, GrayImage, ImageFormat, Luma};
use serde_json::json;
use sha2::{Digest, Sha256};
use weread::{
    book_assets::{BookAssetStore, DEFAULT_IMAGE_BUDGET},
    cache::FileCache,
    demo,
    platform::DeviceProfile,
    store::Store,
};

#[derive(Parser)]
struct Args {
    /// New directory for the public fixture; an existing directory is never opened.
    #[arg(long)]
    data_dir: PathBuf,
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn source_png(ordinal: usize) -> Result<Vec<u8>> {
    let image = GrayImage::from_fn(200, 280, |x, y| {
        let gray = if (20..180).contains(&x) && (54..222).contains(&y) {
            // Repeat all four bits inside the center crop shown by the current Library.
            let bit = ((ordinal + 1) >> (((x - 20) / 16) % 4)) & 1;
            if (bit != 0) == (y < 138) { 221 } else { 34 }
        } else if (30..43).contains(&y) || (237..250).contains(&y) {
            0
        } else if x < 16 || x >= 184 {
            102
        } else {
            255
        };
        Luma([gray])
    });
    let mut out = Cursor::new(Vec::new());
    DynamicImage::ImageLuma8(image).write_to(&mut out, ImageFormat::Png)?;
    Ok(out.into_inner())
}

fn main() -> Result<()> {
    ensure!(cfg!(target_os = "linux"), "Use the designated Linux worker");
    let root = Args::parse().data_dir;
    fs::create_dir(&root)
        .context("The fixture requires a new directory with an existing parent")?;
    let root = fs::canonicalize(root)?;
    let data = root.join("demo");
    fs::create_dir(&data)?;
    let store = Store::open(data.join("library.sqlite"))?;
    demo::seed(&store)?;
    demo::seed_images(
        &store,
        &BookAssetStore::open(data.join("images"), DEFAULT_IMAGE_BUDGET)?,
    )?;
    let cache = FileCache::open(data.join("covers"), 96 * 1024 * 1024)?;
    let shelf = store.shelf()?;
    let profile = DeviceProfile::Scribe;
    ensure!(shelf.len() == 15 && shelf.len() == profile.cover_page_size());
    let mut excluded: Vec<_> = store
        .books()?
        .into_iter()
        .filter(|b| !b.on_shelf)
        .map(|b| b.id)
        .collect();
    excluded.sort();
    ensure!(
        excluded
            == [
                "article:demo-article-0",
                "article:demo-article-1",
                "article:demo-article-2"
            ]
    );

    let mut targets = Vec::new();
    for (ordinal, mut book) in shelf.into_iter().enumerate() {
        ensure!(
            book.id == format!("demo-{ordinal}")
                && book.source == "demo"
                && book.cover_url.is_empty()
        );
        book.cover_url = format!("https://example.invalid/library-covers/{}.png", book.id);
        store.upsert_book(&book)?;
        // The key builder is private; the frame check fails if this encoding no longer
        // agrees with the production Library reader.
        let mut key = Sha256::new();
        key.update((book.id.len() as u64).to_be_bytes());
        key.update(book.id.as_bytes());
        key.update(book.cover_url.as_bytes());
        let key = format!("cover:{}", hex::encode(key.finalize()));
        let png = source_png(ordinal)?;
        let path = cache.put_cover(&key, &png, 400, 560)?;
        ensure!(cache.get(&key)?.as_ref() == Some(&path));
        targets.push(json!({
            "book_id": book.id, "row": ordinal / profile.cover_columns(),
            "column": ordinal % profile.cover_columns(), "cover_url": book.cover_url,
            "cache_key": key, "source_sha256": digest(&png),
            "thumbnail": path.strip_prefix(&root)?, "thumbnail_sha256": digest(&fs::read(&path)?),
        }));
    }
    let receipt = json!({
        "format": "weread-offline-library-covers-v1", "profile": profile.id(),
        "targets": targets, "excluded_book_ids": excluded,
        "scope": "Offline public demo, default Library cover grid, Scribe profile, page one. No network requests, full sleep covers, display timing, or device measurements.",
    });
    fs::write(
        root.join("library-covers.json"),
        serde_json::to_vec_pretty(&receipt)?,
    )?;
    Ok(())
}
