//! Processing belongs to one completely published picture scene, not preferences.

use super::*;
use crate::platform::ImageRegion;
use crate::preferences::ImageDithering;
use crate::reader::layout_budget::{LayoutBudget, LayoutLease, LayoutModelRegistration};
use sha2::{Digest, Sha256};

pub(super) struct PreparedReaderImages {
    pub(super) rows: ModelRc<ReaderImage>,
    pub(super) scene: Option<PresentedImageScene>,
}

enum SceneModels {
    Reader {
        primary: ModelRc<ReaderImage>,
        software: ModelRc<ReaderImage>,
    },
    Article {
        primary: ModelRc<ArticleBlock>,
        software: ModelRc<ArticleBlock>,
    },
}

pub(super) struct PresentedImageScene {
    requested: ImageDithering,
    effective: ImageDithering,
    models: SceneModels,
    regions: Rc<Vec<ImageRegion>>,
    using_primary: bool,
    reader_geometry: Option<(bool, bool, i32)>,
    _identity: String,
    _storage: LayoutLease,
    _registration: LayoutModelRegistration,
}

#[derive(Default)]
pub(super) struct ImageProcessingState {
    scene: Option<PresentedImageScene>,
    sleep: bool,
    applied: Option<Rc<Vec<ImageRegion>>>,
}

pub(super) fn article_cache_key(article: &str, id: &str, url: &str) -> String {
    let mut hash = Sha256::new();
    for field in [article, id, url] {
        hash.update((field.len() as u64).to_be_bytes());
        hash.update(field.as_bytes());
    }
    format!("article-image:gray8-v2:{}", hex::encode(hash.finalize()))
}

/// A physical image box is eligible only when no clipping or fractional edge
/// can move software fallback pixels into nearby text, captions, or controls.
pub(super) fn clean_region(
    image: (f64, f64, f64, f64),
    clip: (f64, f64, f64, f64),
    scale: f64,
) -> Option<ImageRegion> {
    let (x, y, width, height) = image;
    let (left, top, clip_width, clip_height) = clip;
    if ![
        x,
        y,
        width,
        height,
        left,
        top,
        clip_width,
        clip_height,
        scale,
    ]
    .iter()
    .all(|value| value.is_finite())
        || width <= 0.0
        || height <= 0.0
        || scale <= 0.0
        || x < left - 0.0001
        || y < top - 0.0001
        || x + width > left + clip_width + 0.0001
        || y + height > top + clip_height + 0.0001
    {
        return None;
    }
    let exact = |value: f64| {
        let value = value * scale;
        (value >= 0.0 && value <= f64::from(u32::MAX) && (value - value.round()).abs() < 0.0001)
            .then_some(value.round() as u32)
    };
    let region = ImageRegion {
        x: exact(x)?,
        y: exact(y)?,
        width: exact(width)?,
        height: exact(height)?,
    };
    (region.width > 0 && region.height > 0).then_some(region)
}

impl App {
    pub(super) fn image_processing_choices(&self) -> (ImageDithering, ImageDithering) {
        let requested = self.prefs.image_dithering;
        // Hardware describes the prepared Gray8 encoding. Availability belongs
        // to the actual submission, and can become known after cold startup.
        (requested, requested)
    }

    pub(super) fn image_viewport(&self) -> ((u32, u32), f64) {
        let (width, height) = self.platform.logical_viewport_size();
        let scale = f64::from(
            self.ui
                .upgrade()
                .map(|ui| ui.window().scale_factor())
                .unwrap_or_else(|| self.platform.scale_factor()),
        );
        (
            (
                (f64::from(width) * scale).round() as u32,
                (f64::from(height) * scale).round() as u32,
            ),
            scale,
        )
    }

    pub(super) fn reader_image_box(
        &self,
        image: &ReaderImage,
    ) -> ((f64, f64, f64, f64), (f64, f64, f64, f64)) {
        let (width, height) = self.platform.logical_viewport_size();
        let scribe = self.profile.is_scribe();
        let text_width = f64::from(width)
            - if scribe && self.reader.notes_panel {
                300.0
            } else {
                0.0
            };
        let heading = self
            .reader
            .paginator
            .as_ref()
            .is_some_and(|p| p.current_index() == 0);
        let top = if heading {
            if scribe { 204.0 } else { 164.0 }
        } else if scribe {
            76.0
        } else {
            54.0
        };
        let margin = f64::from(super::reading::reader_margin(
            scribe,
            self.reader.preferences.margin,
        ));
        let body = (
            margin,
            top,
            (text_width - margin * 2.0).max(1.0),
            (f64::from(height) - top - 48.0).max(1.0),
        );
        let image_width = body.2.min(f64::from(image.width));
        (
            (
                body.0 + (body.2 - image_width) / 2.0,
                top + f64::from(image.y),
                image_width,
                (f64::from(image.height) - 24.0).max(1.0),
            ),
            body,
        )
    }

    fn image_scene(
        &self,
        models: SceneModels,
        requested: ImageDithering,
        effective: ImageDithering,
        regions: Vec<ImageRegion>,
        identity: String,
        budget: &LayoutBudget,
    ) -> Result<PresentedImageScene> {
        let bytes = std::mem::size_of::<PresentedImageScene>()
            .checked_add(identity.capacity())
            .and_then(|bytes| {
                bytes.checked_add(
                    regions.capacity() * std::mem::size_of::<ImageRegion>()
                        + std::mem::size_of::<Vec<ImageRegion>>()
                        + 2 * std::mem::size_of::<usize>(),
                )
            })
            .ok_or(crate::reader::LayoutLimitError {
                kind: crate::reader::LayoutLimitKind::Metadata,
                used: 0,
                requested: usize::MAX,
                limit: crate::reader::layout_budget::DEFAULT_LAYOUT_METADATA_BYTES,
            })?;
        let storage = budget.copy_storage(bytes)?;
        let registration = self.model_layout.register(storage.bytes())?;
        let reader_geometry = matches!(&models, SceneModels::Reader { .. }).then(|| {
            (
                self.reader
                    .paginator
                    .as_ref()
                    .is_some_and(|p| p.current_index() == 0),
                self.reader.notes_panel,
                super::reading::reader_margin(
                    self.profile.is_scribe(),
                    self.reader.preferences.margin,
                ) as i32,
            )
        });
        Ok(PresentedImageScene {
            requested,
            effective,
            models,
            regions: Rc::new(regions),
            using_primary: true,
            reader_geometry,
            _identity: identity,
            _storage: storage,
            _registration: registration,
        })
    }

    pub(super) fn prepared_reader_scene(
        &self,
        primary: ModelRc<ReaderImage>,
        software: ModelRc<ReaderImage>,
        requested: ImageDithering,
        effective: ImageDithering,
        regions: Vec<ImageRegion>,
        identity: String,
        budget: &LayoutBudget,
    ) -> Result<PreparedReaderImages> {
        use slint::Model;
        let scene = if primary.iter().any(|row| row.loaded) {
            Some(self.image_scene(
                SceneModels::Reader {
                    primary: primary.clone(),
                    software,
                },
                requested,
                effective,
                regions,
                identity,
                budget,
            )?)
        } else {
            None
        };
        Ok(PreparedReaderImages {
            rows: primary,
            scene,
        })
    }

    pub(super) fn publish_reader_images(&mut self, images: PreparedReaderImages) -> Result<()> {
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<ReaderModel>().set_images(images.rows);
        }
        self.replace_image_scene(images.scene);
        self.sync_presented_image_processing()
    }

    pub(super) fn prepare_article_image_scene(
        &self,
        mut blocks: Vec<ArticleBlock>,
    ) -> Result<(ModelRc<ArticleBlock>, Option<PresentedImageScene>)> {
        if !blocks.iter().any(|row| row.kind == "image" && row.loaded) {
            return Ok((model(blocks), None));
        }
        let (requested, mut effective) = self.image_processing_choices();
        let (viewport, scale) = self.image_viewport();
        let mut raster = super::image_raster::RasterBudget::new(viewport)?;
        let (width, height) = self.platform.logical_viewport_size();
        let margin = if self.profile.is_scribe() { 72.0 } else { 32.0 };
        let top = if self.profile.is_scribe() { 80.0 } else { 54.0 };
        let footer = if self.profile.is_scribe() { 56.0 } else { 48.0 };
        let clip = (
            margin,
            top,
            f64::from(width) - margin * 2.0,
            f64::from(height) - top - footer,
        );
        let budget = LayoutBudget::default();
        let _old = budget.hold(self.retained_layout_metadata()?)?;
        let hardware = effective == ImageDithering::Hardware;
        let bytes = blocks
            .iter()
            .try_fold(
                blocks
                    .capacity()
                    .checked_mul(std::mem::size_of::<ArticleBlock>())
                    .and_then(|bytes| {
                        bytes.checked_add(
                            std::mem::size_of::<AdmittedViewModel<ArticleBlock>>()
                                + 2 * std::mem::size_of::<usize>(),
                        )
                    })
                    .unwrap_or(usize::MAX),
                |used, row| {
                    used.checked_add(
                        row.id.len() + row.kind.len() + row.text.len() + row.meta.len(),
                    )
                },
            )
            .ok_or_else(|| anyhow::anyhow!("Article image scene metadata overflow"))?;
        let primary_owner = budget.copy_storage(bytes)?;
        let software_owner = if hardware {
            Some(budget.copy_storage(bytes)?)
        } else {
            None
        };
        let mut fallback = Vec::with_capacity(if hardware { blocks.len() } else { 0 });
        budget.preflight(blocks.len() * std::mem::size_of::<ImageRegion>(), 0)?;
        let mut regions = Vec::with_capacity(if hardware { blocks.len() } else { 0 });
        let mut clean = true;
        let mut any = false;
        let mut offset = 0.0;
        let mut digest = Sha256::new();
        digest.update(self.article.id.as_bytes());
        for block in &mut blocks {
            let mut software = None;
            if block.kind == "image" && block.loaded {
                let path = self.article.loaded_images.get(block.id.as_str());
                let frame = (
                    (clip.2 * scale).round().max(1.0) as u32,
                    (f64::from(block.frame_height) * scale).round().max(1.0) as u32,
                );
                if let Some(path) = path {
                    let picture = super::image_raster::prepare_picture(
                        path,
                        frame,
                        super::image_raster::PictureFit::Cover,
                        effective,
                        &mut raster,
                    )?;
                    block.picture = picture.primary;
                    software = Some(picture.software);
                    any = true;
                    budget.work(path.as_os_str().as_encoded_bytes().len())?;
                    digest.update(path.as_os_str().as_encoded_bytes());
                    digest.update(frame.0.to_be_bytes());
                    digest.update(frame.1.to_be_bytes());
                    if hardware {
                        if let Some(region) = clean_region(
                            (clip.0, top + offset, clip.2, f64::from(block.frame_height)),
                            clip,
                            scale,
                        ) {
                            regions.push(region);
                        } else {
                            clean = false;
                        }
                    }
                }
            }
            offset += f64::from(block.height);
            if hardware {
                let mut row = block.clone();
                if let Some(picture) = software {
                    row.picture = picture;
                }
                fallback.push(row);
            }
        }
        let primary = self.admitted_shared_article_model(blocks, primary_owner, &budget)?;
        if !any {
            return Ok((primary, None));
        }
        let software = if hardware {
            self.admitted_shared_article_model(fallback, software_owner.unwrap(), &budget)?
        } else {
            primary.clone()
        };
        let primary = if hardware && !clean {
            effective = ImageDithering::Software;
            regions.clear();
            software.clone()
        } else {
            primary
        };
        digest.update(effective.as_str().as_bytes());
        let scene = self.image_scene(
            SceneModels::Article {
                primary: primary.clone(),
                software,
            },
            requested,
            effective,
            regions,
            hex::encode(digest.finalize()),
            &budget,
        )?;
        Ok((primary, Some(scene)))
    }

    fn admitted_shared_article_model(
        &self,
        rows: Vec<ArticleBlock>,
        owner: LayoutLease,
        _budget: &LayoutBudget,
    ) -> Result<ModelRc<ArticleBlock>> {
        let registration = self.model_layout.register(owner.bytes())?;
        Ok(Rc::new(AdmittedViewModel {
            rows: VecModel::from(rows),
            _storage: owner,
            _registration: registration,
        })
        .into())
    }

    pub(super) fn publish_article_images(
        &mut self,
        rows: ModelRc<ArticleBlock>,
        scene: Option<PresentedImageScene>,
    ) -> Result<()> {
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<ArticleModel>().set_blocks(rows);
        }
        self.replace_image_scene(scene);
        self.sync_presented_image_processing()
    }

    fn replace_image_scene(&mut self, scene: Option<PresentedImageScene>) {
        if let (Some(old), Some(ui)) = (&self.image_processing.scene, self.ui.upgrade()) {
            let reader = scene.as_ref().map_or_else(
                || ui.global::<ShellModel>().get_route() == "reader",
                |new| matches!(&new.models, SceneModels::Reader { .. }),
            );
            match (&old.models, reader) {
                (SceneModels::Reader { .. }, false) => {
                    ui.global::<ReaderModel>().set_images(ModelRc::default())
                }
                (SceneModels::Article { .. }, true) => {
                    ui.global::<ArticleModel>().set_blocks(ModelRc::default())
                }
                _ => {}
            }
        }
        self.image_processing.scene = scene;
    }

    pub(super) fn suspend_image_processing_for_sleep(&mut self) -> Result<()> {
        self.image_processing.sleep = true;
        self.sync_presented_image_processing()
    }

    pub(super) fn restore_image_processing_after_sleep(&mut self) -> Result<()> {
        self.image_processing.sleep = false;
        self.sync_presented_image_processing()
    }

    pub(super) fn clear_presented_image_processing(&mut self) -> Result<()> {
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<ReaderModel>().set_images(ModelRc::default());
            ui.global::<ArticleModel>().set_blocks(ModelRc::default());
        }
        self.image_processing.scene = None;
        self.sync_presented_image_processing()
    }

    /// Getter-only on idle ticks. Vectors are copied only when the submitted
    /// picture regions change; pending preferences never select a model here.
    pub(super) fn sync_presented_image_processing(&mut self) -> Result<()> {
        let Some(ui) = self.ui.upgrade() else {
            return Ok(());
        };
        let shell = ui.global::<ShellModel>();
        let reader = ui.global::<ReaderModel>();
        let article = ui.global::<ArticleModel>();
        let obstructed = self.image_processing.sleep
            || ui.global::<SleepScreenModel>().get_active()
            || !shell.get_dialog_kind().is_empty()
            || !shell.get_toast().is_empty();
        let scene_active =
            self.image_processing
                .scene
                .as_ref()
                .is_some_and(|scene| match &scene.models {
                    SceneModels::Reader { .. } => shell.get_route() == "reader",
                    SceneModels::Article { .. } => shell.get_route() == "article",
                });
        if let Some(scene) = self.image_processing.scene.as_mut() {
            let overlay = obstructed
                || match &scene.models {
                    SceneModels::Reader { .. } => {
                        reader.get_menu()
                            || reader.get_toc_open()
                            || reader.get_thought_open()
                            || reader.get_selection_open()
                            || reader.get_writing_thought()
                            || reader.get_sync_conflict()
                            || reader.get_access_notice()
                            || scene.reader_geometry
                                != Some((
                                    reader.get_show_heading(),
                                    reader.get_notes_panel(),
                                    reader.get_margin(),
                                ))
                    }
                    SceneModels::Article { .. } => {
                        article.get_menu() || article.get_list_open() || article.get_qr_open()
                    }
                };
            let primary = scene_active
                && !overlay
                && scene.effective == ImageDithering::Hardware
                && !scene.regions.is_empty()
                && self.platform.hardware_y4_available();
            let use_primary = scene.effective != ImageDithering::Hardware || primary;
            if scene.using_primary != use_primary {
                match &scene.models {
                    SceneModels::Reader { primary, software } => {
                        reader.set_images(if use_primary {
                            primary.clone()
                        } else {
                            software.clone()
                        })
                    }
                    SceneModels::Article { primary, software } => {
                        article.set_blocks(if use_primary {
                            primary.clone()
                        } else {
                            software.clone()
                        })
                    }
                }
                scene.using_primary = use_primary;
            }
            let regions = if primary {
                scene.regions.as_slice()
            } else {
                &[]
            };
            let applied = self
                .image_processing
                .applied
                .as_ref()
                .map_or(&[][..], |old| old.as_slice());
            if applied != regions {
                self.platform.set_hardware_y4_for_frame(regions)?;
                self.image_processing.applied = primary.then(|| scene.regions.clone());
            } else if primary
                && self
                    .image_processing
                    .applied
                    .as_ref()
                    .is_some_and(|old| !Rc::ptr_eq(old, &scene.regions))
            {
                self.image_processing.applied = Some(scene.regions.clone());
            }
        } else if self.image_processing.applied.is_some() {
            self.platform.set_hardware_y4_for_frame(&[])?;
            self.image_processing.applied = None;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "image_processing_tests.rs"]
mod tests;
