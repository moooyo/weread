use super::*;
use crate::book_assets::ChapterManifest;
use crate::reader::layout_budget::{LayoutLease, LayoutVec};
use crate::reader::{AdmittedLayout, IMAGE_CAPTION_HEIGHT, InlineImage, Layout, LayoutBudget};

const PLACEHOLDER_WIDTH: f32 = 480.0;
const PLACEHOLDER_HEIGHT: f32 = 240.0;

pub(super) struct ReaderImageSnapshot {
    manifest: ChapterManifest,
    paths: Vec<Option<PathBuf>>,
    verified: bool,
    _layout: LayoutLease,
}

impl App {
    /// One aligned operation-local verification result. No payload or verified
    /// path is retained in App after its caller finishes this operation.
    pub(super) fn reader_image_snapshot(&self) -> Result<Option<ReaderImageSnapshot>> {
        self.reader_image_snapshot_for(
            &self.reader.book_id,
            self.reader.chapter_index,
            &LayoutBudget::default(),
        )
    }

    pub(super) fn reader_image_snapshot_for(
        &self,
        book_id: &str,
        chapter_index: u32,
        budget: &LayoutBudget,
    ) -> Result<Option<ReaderImageSnapshot>> {
        let Some(uid) = self.store.chapter_uid(book_id, chapter_index)? else {
            return Ok(None);
        };
        let manifest = match self.book_assets.chapter(&self.store, book_id, &uid) {
            Ok(Some(manifest)) => manifest,
            Ok(None) => return Ok(None),
            Err(error) if reader_image_capacity(&error) => return Err(error),
            // Keep the existing optional-picture behavior for an unreadable
            // historical manifest. Capacity refusal is never a missing image.
            Err(_) => return Ok(None),
        };
        let path_bytes = manifest
            .images
            .len()
            .checked_mul(std::mem::size_of::<Option<PathBuf>>())
            .ok_or(crate::reader::LayoutLimitError {
                kind: crate::reader::LayoutLimitKind::Metadata,
                used: 0,
                requested: usize::MAX,
                limit: crate::reader::layout_budget::DEFAULT_LAYOUT_METADATA_BYTES,
            })?;
        let layout = budget.copy_storage(path_bytes)?;
        let (paths, verified) = match self.book_assets.image_paths(&manifest.images) {
            Ok(paths) if paths.len() == manifest.images.len() => (paths, true),
            Ok(_) => (vec![None; manifest.images.len()], false),
            Err(error) if reader_image_capacity(&error) => return Err(error),
            Err(error) => {
                crate::diagnostic!("Reader image verification unavailable: {error:#}");
                (vec![None; manifest.images.len()], false)
            }
        };
        Ok(Some(ReaderImageSnapshot {
            manifest,
            paths,
            verified,
            _layout: layout,
        }))
    }

    /// Resolve only image occurrences whose position is known in the canonical text.
    #[cfg(test)]
    pub(super) fn reader_inline_images(
        &self,
        layout: &Layout,
        snapshot: Option<&ReaderImageSnapshot>,
    ) -> Result<Vec<InlineImage>> {
        Ok(self
            .reader_inline_images_for(
                &self.reader.body,
                layout,
                snapshot,
                &LayoutBudget::default(),
            )?
            .into_parts()
            .0)
    }

    pub(super) fn reader_inline_images_for(
        &self,
        body: &str,
        layout: &Layout,
        snapshot: Option<&ReaderImageSnapshot>,
        budget: &LayoutBudget,
    ) -> Result<AdmittedLayout<Vec<InlineImage>>> {
        let mut output = LayoutVec::new(budget);
        let Some(snapshot) = snapshot else {
            let (images, owner) = output.into_parts();
            return Ok(AdmittedLayout::new(images, owner));
        };
        budget.work(body.len())?;
        let length =
            u32::try_from(body.chars().count()).map_err(|_| crate::reader::LayoutLimitError {
                kind: crate::reader::LayoutLimitKind::Anchors,
                used: 0,
                requested: usize::MAX,
                limit: u32::MAX as usize,
            })?;
        let mut identities: LayoutVec<&str> = LayoutVec::new(budget);
        for (index, image) in snapshot.manifest.images.iter().enumerate() {
            let Some(anchor) = image.anchor.filter(|anchor| *anchor <= length) else {
                continue;
            };
            if image.id.is_empty() {
                continue;
            }
            let work = image
                .id
                .len()
                .checked_mul(identities.len())
                .and_then(|work| work.checked_mul(2))
                .ok_or(crate::reader::LayoutLimitError {
                    kind: crate::reader::LayoutLimitKind::AllocationWork,
                    used: 0,
                    requested: usize::MAX,
                    limit: crate::reader::layout_budget::DEFAULT_LAYOUT_ALLOCATION_WORK,
                })?;
            budget.work(work)?;
            if identities.iter().any(|prior| *prior == image.id.as_str()) {
                continue;
            }
            output.reserve_one()?;
            identities.push(image.id.as_str())?;
            let (id, owner) = budget.copy(&image.id)?;
            let cached = snapshot.paths[index].is_some();
            let (width, height) =
                intrinsic_geometry(image.width, image.height, cached, layout.width);
            output.push_owned(
                InlineImage {
                    id,
                    anchor,
                    width,
                    height,
                },
                owner,
            )?;
        }
        let buffer = output.buffer_bytes();
        let (images, owner) = output.into_parts();
        Ok(AdmittedLayout::from_vec(images, owner, buffer))
    }

    /// Read rendered image geometry from the same paginator that owns text anchors.
    pub(super) fn render_reader_images(&mut self) -> Result<()> {
        let Some(ui) = self.ui.upgrade() else {
            return Ok(());
        };
        let budget = LayoutBudget::default();
        let _old = budget.hold(self.retained_layout_metadata()?)?;
        let images = self.prepare_reader_image_rows(&budget)?;
        ui.global::<ReaderModel>().set_images(images);
        Ok(())
    }

    pub(super) fn prepare_reader_image_rows(
        &self,
        budget: &LayoutBudget,
    ) -> Result<ModelRc<ReaderImage>> {
        let snapshot = self.reader_image_snapshot_for(
            &self.reader.book_id,
            self.reader.chapter_index,
            budget,
        )?;
        let page = self
            .reader
            .paginator
            .as_ref()
            .map(|page| page.current_page());
        let count = page.map_or(0, |page| page.images.len());
        let fields = page
            .into_iter()
            .flat_map(|page| page.images.iter())
            .filter_map(|placed| {
                snapshot
                    .as_ref()?
                    .manifest
                    .images
                    .iter()
                    .find(|image| image.id == placed.id && image.anchor.is_some())
            })
            .flat_map(|image| [image.id.as_str(), image.caption.as_str()]);
        self.preflight_view_fields::<ReaderImage>(budget, count, fields, 0)?;
        let mut images = LayoutVec::with_capacity(budget, count)?;
        let Some(snapshot) = snapshot else {
            return self.admitted_view_model(images, budget);
        };
        if let Some(page) = self
            .reader
            .paginator
            .as_ref()
            .map(|paginator| paginator.current_page())
        {
            for placed in &page.images {
                let Some((index, image)) = snapshot
                    .manifest
                    .images
                    .iter()
                    .enumerate()
                    .find(|(_, image)| image.id == placed.id && image.anchor.is_some())
                else {
                    continue;
                };
                if !valid_placement(placed.y, placed.width, placed.height) {
                    continue;
                }
                let picture = snapshot.paths[index]
                    .as_ref()
                    .and_then(|path| slint::Image::load_from_path(path).ok());
                let (id, mut owner) = self.admitted_shared_string(&image.id, budget)?;
                let (caption, next) = self.admitted_shared_string(&image.caption, budget)?;
                owner.merge(next);
                images.push_owned(
                    ReaderImage {
                        id,
                        loaded: picture.is_some(),
                        picture: picture.unwrap_or_default(),
                        y: placed.y,
                        width: placed.width,
                        height: placed.height,
                        caption,
                    },
                    owner,
                )?;
            }
        }
        self.admitted_view_model(images, budget)
    }

    pub(super) fn load_reader_image(&mut self, id: &str) -> Result<()> {
        let Some(snapshot) = self.reader_image_snapshot()? else {
            return Ok(());
        };
        if !snapshot.verified {
            self.show_toast("api_error");
            return Ok(());
        }
        let Some((index, _)) = snapshot
            .manifest
            .images
            .iter()
            .enumerate()
            .find(|(_, image)| image.id == id && image.anchor.is_some())
        else {
            return Ok(());
        };
        if snapshot.paths[index].is_some() {
            self.show_toast("operation_failed");
            return Ok(());
        }
        if self.demo {
            self.show_toast("demo_notice");
            return Ok(());
        }
        if self.force_offline || !self.online {
            self.show_toast("network_unavailable");
            return Ok(());
        }
        if !self.jobs.capabilities().chapter_content || !self.jobs.capabilities().chapter_images {
            self.show_toast("web_login_required");
            return Ok(());
        }
        let book_id = self.reader.book_id.clone();
        self.enqueue_download_options(&book_id, vec![self.reader.chapter_index], true)?;
        Ok(())
    }
}

fn reader_image_capacity(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<crate::cache::InventoryLimitExceeded>()
            .is_some()
            || cause
                .downcast_ref::<crate::cache::InventoryUnavailable>()
                .is_some()
            || cause
                .downcast_ref::<crate::cache::VerificationLimitExceeded>()
                .is_some()
            || cause
                .downcast_ref::<crate::cache::VerificationMetadataLimitExceeded>()
                .is_some()
            || cause
                .downcast_ref::<crate::store::StoreInventoryLimit>()
                .is_some()
            || cause
                .downcast_ref::<crate::store::StoreInventoryUnavailable>()
                .is_some()
            || cause
                .downcast_ref::<crate::reader::LayoutLimitError>()
                .is_some()
    })
}

fn intrinsic_geometry(width: u32, height: u32, cached: bool, available_width: f32) -> (f32, f32) {
    if cached && width > 0 && height > 0 {
        (width as f32 / 2.0, height as f32 / 2.0)
    } else {
        // Unknown dimensions use a useful touch target; loading reflows at a text anchor.
        (
            PLACEHOLDER_WIDTH.min(available_width.max(48.0)),
            PLACEHOLDER_HEIGHT,
        )
    }
}

fn valid_placement(y: f32, width: f32, height: f32) -> bool {
    y.is_finite()
        && y >= 0.0
        && width.is_finite()
        && width > 0.0
        && height.is_finite()
        && height > IMAGE_CAPTION_HEIGHT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uncached_images_have_a_useful_touch_target_and_cached_pixels_use_device_scale() {
        assert_eq!(intrinsic_geometry(0, 0, false, 568.0), (480.0, 240.0));
        assert_eq!(intrinsic_geometry(0, 0, false, 300.0), (300.0, 240.0));
        assert_eq!(intrinsic_geometry(1860, 2480, true, 568.0), (930.0, 1240.0));
        assert_eq!(intrinsic_geometry(0, 2480, true, 568.0), (480.0, 240.0));
    }

    #[test]
    fn invalid_or_empty_image_slots_are_never_sent_to_the_renderer() {
        assert!(valid_placement(0.0, 240.0, 144.0));
        assert!(!valid_placement(-1.0, 240.0, 144.0));
        assert!(!valid_placement(0.0, f32::NAN, 144.0));
        assert!(!valid_placement(0.0, 240.0, IMAGE_CAPTION_HEIGHT));
    }
}
