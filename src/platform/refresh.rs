use std::time::Duration;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RefreshRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefreshKind {
    Full,
    FlashUi,
    Ui,
    Fast,
    FlashPartial,
    Partial,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefreshReason {
    EnterScreen,
    Content,
    Press,
    OverlayOpened(RefreshRect),
    OverlayClosed(RefreshRect),
    Progress,
    ReaderTurn,
    CoversLoaded,
    ImageLoaded(RefreshRect),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RefreshPlan {
    pub kind: RefreshKind,
    /// Logical design coordinates. A native sink must apply the fixed 2x scale.
    pub rect: Option<RefreshRect>,
}

/// Policy is separate from the backend's ability to execute a waveform.
///
/// A sink may enqueue these plans for post-render execution. Queue acceptance
/// must not be reported as a completed hardware update by the caller.
#[derive(Debug)]
pub struct RefreshScheduler {
    content_partials: u32,
    reader_partials: u32,
    reader_full_interval: u32,
    last_progress: Option<Duration>,
    flash_on_navigation: bool,
}

impl Default for RefreshScheduler {
    fn default() -> Self {
        Self {
            content_partials: 0,
            reader_partials: 0,
            reader_full_interval: 6,
            last_progress: None,
            flash_on_navigation: true,
        }
    }
}

impl RefreshScheduler {
    pub fn set_reader_interval(&mut self, pages: u32) -> Result<(), &'static str> {
        if !matches!(pages, 0 | 1 | 6 | 12) {
            return Err("reader refresh interval must be 0, 1, 6, or 12");
        }
        self.reader_full_interval = pages;
        self.reader_partials = 0;
        Ok(())
    }

    pub fn set_flash_on_navigation(&mut self, enabled: bool) {
        self.flash_on_navigation = enabled;
    }

    pub fn plan(&mut self, reason: RefreshReason, now: Duration) -> Option<RefreshPlan> {
        use RefreshKind as Kind;
        let (kind, rect) = match reason {
            RefreshReason::EnterScreen => {
                self.content_partials = 0;
                self.reader_partials = 0;
                (
                    if self.flash_on_navigation {
                        Kind::FlashUi
                    } else {
                        Kind::Ui
                    },
                    None,
                )
            }
            RefreshReason::Content => {
                self.content_partials += 1;
                if self.content_partials >= 6 {
                    self.content_partials = 0;
                    (Kind::Full, None)
                } else {
                    (Kind::Ui, None)
                }
            }
            RefreshReason::Press => (Kind::Fast, None),
            RefreshReason::OverlayOpened(rect) => (Kind::Ui, Some(rect)),
            RefreshReason::OverlayClosed(rect) => (Kind::FlashPartial, Some(rect)),
            RefreshReason::Progress => {
                if self
                    .last_progress
                    .is_some_and(|last| now.saturating_sub(last) < Duration::from_secs(1))
                {
                    return None;
                }
                self.last_progress = Some(now);
                (Kind::Fast, None)
            }
            RefreshReason::ReaderTurn => {
                self.reader_partials += 1;
                if self.reader_full_interval != 0
                    && self.reader_partials >= self.reader_full_interval
                {
                    self.reader_partials = 0;
                    (Kind::Full, None)
                } else {
                    (Kind::Partial, None)
                }
            }
            // Covers arrive independently after navigation. Their changed tiles
            // use the renderer's dirty regions without spending the content or
            // Reader refresh budget on a full-screen flash.
            RefreshReason::CoversLoaded => (Kind::Ui, None),
            RefreshReason::ImageLoaded(rect) => (Kind::Full, Some(rect)),
        };
        Some(RefreshPlan { kind, rect })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_content_updates_require_a_full_refresh() {
        let mut scheduler = RefreshScheduler::default();
        for _ in 0..5 {
            assert_eq!(
                scheduler
                    .plan(RefreshReason::Content, Duration::ZERO)
                    .unwrap()
                    .kind,
                RefreshKind::Ui
            );
        }
        assert_eq!(
            scheduler
                .plan(RefreshReason::Content, Duration::ZERO)
                .unwrap()
                .kind,
            RefreshKind::Full
        );
        assert_eq!(
            scheduler
                .plan(RefreshReason::Content, Duration::ZERO)
                .unwrap()
                .kind,
            RefreshKind::Ui
        );
    }

    #[test]
    fn progress_is_rate_limited_without_delaying_other_interactions() {
        let mut scheduler = RefreshScheduler::default();
        assert!(
            scheduler
                .plan(RefreshReason::Progress, Duration::ZERO)
                .is_some()
        );
        assert!(
            scheduler
                .plan(RefreshReason::Progress, Duration::from_millis(999))
                .is_none()
        );
        assert!(
            scheduler
                .plan(RefreshReason::Press, Duration::from_millis(999))
                .is_some()
        );
        assert!(
            scheduler
                .plan(RefreshReason::Progress, Duration::from_secs(1))
                .is_some()
        );
    }

    #[test]
    fn cover_batches_never_flash_or_consume_the_content_refresh_budget() {
        let mut scheduler = RefreshScheduler::default();
        for _ in 0..5 {
            scheduler.plan(RefreshReason::Content, Duration::ZERO);
        }
        for _ in 0..32 {
            assert_eq!(
                scheduler.plan(RefreshReason::CoversLoaded, Duration::ZERO),
                Some(RefreshPlan {
                    kind: RefreshKind::Ui,
                    rect: None,
                })
            );
        }
        assert_eq!(
            scheduler
                .plan(RefreshReason::Content, Duration::ZERO)
                .unwrap()
                .kind,
            RefreshKind::Full
        );
    }

    #[test]
    fn closing_an_overlay_keeps_its_original_rectangle() {
        let mut scheduler = RefreshScheduler::default();
        let rect = RefreshRect {
            x: 20,
            y: 120,
            width: 592,
            height: 480,
        };
        let plan = scheduler
            .plan(RefreshReason::OverlayClosed(rect), Duration::ZERO)
            .unwrap();
        assert_eq!(plan.kind, RefreshKind::FlashPartial);
        assert_eq!(plan.rect, Some(rect));
    }

    #[test]
    fn disabling_periodic_reader_refresh_does_not_disable_image_refresh() {
        let mut scheduler = RefreshScheduler::default();
        scheduler.set_reader_interval(0).unwrap();
        for _ in 0..30 {
            assert_eq!(
                scheduler
                    .plan(RefreshReason::ReaderTurn, Duration::ZERO)
                    .unwrap()
                    .kind,
                RefreshKind::Partial
            );
        }
        assert_eq!(
            scheduler
                .plan(
                    RefreshReason::ImageLoaded(RefreshRect::default()),
                    Duration::ZERO
                )
                .unwrap()
                .kind,
            RefreshKind::Full
        );
    }
}
