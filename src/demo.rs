use anyhow::Result;
use chrono::{Duration, Utc};

use crate::{
    domain::{Book, Chapter, ReadingPosition, ReadingSession},
    i18n,
    store::Store,
};

/// Explicit interface fixtures. These records never enter a real account store.
pub fn seed(store: &Store) -> Result<()> {
    if store.setting::<bool>("demo.seeded")?.unwrap_or(false) {
        return seed_feeds(store);
    }
    let rows: Vec<DemoBook> = serde_json::from_str(include_str!("../assets/demo/catalog.json"))?;
    for (index, row) in rows.into_iter().enumerate() {
        store.upsert_book(&Book {
            id: format!("demo-{index}"),
            title: row.title,
            author: row.author,
            category: i18n::text("sample_category"),
            intro: i18n::text("sample_intro"),
            source: "demo".into(),
            progress: if index == 0 { 0.22 } else { 0.0 },
            total_chapters: if index == 0 { 3 } else { 0 },
            updated: index == 1 || index == 4,
            on_shelf: true,
            ..Default::default()
        })?;
    }
    let chapters: Vec<DemoChapter> =
        serde_json::from_str(include_str!("../assets/demo/chapters.json"))?;
    for (index, chapter) in chapters.into_iter().enumerate() {
        store.upsert_chapter(&Chapter {
            id: format!("demo-chapter-{index}"),
            book_id: "demo-0".into(),
            index: index as u32,
            title: chapter.title,
            cached: true,
            body: Some(chapter.body),
            content_status: crate::domain::ContentStatus::Full,
            ..Chapter::default()
        })?;
    }
    store.save_position(&ReadingPosition {
        book_id: "demo-0".into(),
        device: "demo".into(),
        ..Default::default()
    })?;
    for day in 0..7_i64 {
        let started_at = (Utc::now() - Duration::days(day) - Duration::hours(3)).timestamp();
        let seconds = (18 + day * 7) as u64 * 60;
        store.record_session(&ReadingSession {
            id: format!("demo-session-{day}"),
            book_id: "demo-0".into(),
            started_at,
            ended_at: started_at + seconds as i64,
            seconds,
        })?;
    }
    store.set_setting("demo.seeded", &true)?;
    seed_feeds(store)
}

fn seed_feeds(store: &Store) -> Result<()> {
    if store.setting::<bool>("demo.feeds-seeded")?.unwrap_or(false) {
        return Ok(());
    }
    let account = crate::source::MpAccount {
        id: "demo-publication".into(),
        name: i18n::text("demo_publication"),
        updated_at: Utc::now().timestamp(),
        ..Default::default()
    };
    let mut articles = Vec::new();
    for (index, chapter) in store.chapters("demo-0")?.into_iter().enumerate() {
        let id = format!("demo-article-{index}");
        let body = chapter.body.unwrap_or_default();
        articles.push(crate::source::MpArticle {
            id: id.clone(),
            account_id: account.id.clone(),
            title: chapter.title.clone(),
            author: i18n::text(match index {
                0 => "sample_chapter_author",
                1 => "sample_lanting_author",
                _ => "sample_loushi_author",
            }),
            published_at: (Utc::now() - Duration::days(index as i64)).timestamp(),
            ..Default::default()
        });
        store.upsert_book(&Book {
            id: format!("article:{id}"),
            title: chapter.title.clone(),
            source: "demo-article".into(),
            total_chapters: 1,
            ..Default::default()
        })?;
        store.upsert_chapter(&Chapter {
            id: format!("article:{id}:0"),
            book_id: format!("article:{id}"),
            index: 0,
            title: chapter.title,
            cached: true,
            body: Some(body.clone()),
            content_status: crate::domain::ContentStatus::Full,
            ..Chapter::default()
        })?;
        let html = body
            .split('\n')
            .map(|paragraph| format!("<p>{paragraph}</p>"))
            .collect::<String>();
        store.set_setting(&format!("article.html:{id}"), &html)?;
    }
    let mut publications = std::collections::BTreeMap::new();
    publications.insert(account.id.clone(), articles);
    store.set_setting("mp.accounts", &vec![account])?;
    store.set_setting("mp.articles", &publications)?;
    store.set_setting("demo.feeds-seeded", &true)
}

#[derive(serde::Deserialize)]
struct DemoBook {
    title: String,
    author: String,
}
#[derive(serde::Deserialize)]
struct DemoChapter {
    title: String,
    body: String,
}

/// A local illustration fixture exercises the complete image pipeline without a network request.
pub fn seed_images(store: &Store, assets: &crate::book_assets::BookAssetStore) -> Result<()> {
    if !store.setting::<bool>("demo.seeded")?.unwrap_or(false) {
        return Ok(());
    }
    let Some(chapter) = store.chapter("demo-0", 1)? else {
        return Ok(());
    };
    if store
        .setting::<bool>("demo.images-seeded")?
        .unwrap_or(false)
        && assets
            .complete(store, "demo-0", &chapter.id)
            .unwrap_or(false)
    {
        return Ok(());
    }
    let Some(body) = chapter.body.filter(|body| !body.is_empty()) else {
        return Ok(());
    };
    let source = "demo-illustration.png".to_owned();
    let caption = i18n::text("demo_illustration");
    let mut html = String::new();
    for (index, paragraph) in body.split('\n').enumerate() {
        html.push_str(&format!("<p>{}</p>", escape_html(paragraph)));
        if index == 0 {
            html.push_str(&format!(
                "<img src=\"{}\" alt=\"{}\" />",
                source,
                escape_html(&caption)
            ));
        }
    }
    let image = image::GrayImage::from_fn(960, 540, |x, y| {
        let distant = 360_i64 - (x as i64 - 310).abs() / 3;
        let nearby = 440_i64 - (x as i64 - 720).abs() / 2;
        let sun = (x as i64 - 730).pow(2) + (y as i64 - 100).pow(2) <= 42_i64.pow(2);
        let value = if x < 4 || y < 4 || x >= 956 || y >= 536 {
            0
        } else if y as i64 >= nearby {
            68
        } else if y as i64 >= distant {
            221
        } else if sun {
            170
        } else {
            255
        };
        image::Luma([value])
    });
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageLuma8(image).write_to(&mut encoded, image::ImageFormat::Png)?;
    let bytes = encoded.into_inner();
    use sha2::Digest;
    let digest = hex::encode(sha2::Sha256::digest(&bytes));
    let packet = crate::source::images::DownloadedChapterImages {
        assets: vec![crate::source::images::ImageAsset {
            id: digest.clone(),
            media_type: "image/png".into(),
            extension: "png".into(),
            source_names: vec![source.clone()],
            bytes,
        }],
        source_to_asset: std::collections::BTreeMap::from([(source, digest)]),
    };
    assets.commit_chapter(store, "demo-0", &chapter.id, &html, &body, Some(&packet))?;
    store.set_setting("demo.images-seeded", &true)
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod image_tests {
    use super::*;

    #[test]
    fn image_fixture_commits_a_real_bitmap_without_rewriting_book_text_or_anchors() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().join("library.sqlite")).unwrap();
        let assets = crate::book_assets::BookAssetStore::open(
            directory.path().join("images"),
            64 * 1024 * 1024,
        )
        .unwrap();
        seed(&store).unwrap();
        let chapter = store.chapter("demo-0", 1).unwrap().unwrap();
        let original = chapter.body.clone().unwrap();
        let position = ReadingPosition {
            book_id: "demo-0".into(),
            chapter_index: 1,
            char_offset: 100,
            ..Default::default()
        };
        store.save_position(&position).unwrap();
        seed_images(&store, &assets).unwrap();
        let manifest = assets
            .chapter(&store, "demo-0", &chapter.id)
            .unwrap()
            .unwrap();
        assert!(manifest.complete);
        assert_eq!(manifest.images.len(), 1);
        let image = &manifest.images[0];
        assert!(
            image
                .anchor
                .is_some_and(|offset| offset > 0 && offset < original.chars().count() as u32)
        );
        let path = assets.image_path(image).unwrap().unwrap();
        let decoded = image::open(path).unwrap().to_rgb8();
        assert_eq!(decoded.dimensions(), (960, 540));
        assert!(
            decoded
                .pixels()
                .all(|pixel| pixel[0] == pixel[1] && pixel[1] == pixel[2] && pixel[0] % 17 == 0)
        );
        assert!(
            assets
                .chapter(&store, "demo-0", "demo-chapter-0")
                .unwrap()
                .is_none()
        );
        seed_images(&store, &assets).unwrap();
        assert_eq!(store.chapter_body("demo-0", 1).unwrap().unwrap(), original);
        assert_eq!(store.position("demo-0").unwrap().unwrap(), position);
    }
}
