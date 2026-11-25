//! Runtime viewport pagination keeps catalog ordinals and visible selection aligned.

use super::super::underline_projection_tests::{BOOK, Fixture, isolated};
use super::*;
use crate::platform::{DeviceControl, PlatformCapabilities, PlatformError};
use slint::Model;

struct ViewportControl;

impl DeviceControl for ViewportControl {
    fn native_viewport(&self) -> Option<(u32, u32)> {
        Some((1860, 2960))
    }

    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities::default()
    }

    fn brightness(&self) -> Result<u8, PlatformError> {
        Err(PlatformError::Unsupported("fixture"))
    }

    fn set_brightness(&self, _: u8) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported("fixture"))
    }

    fn warmth(&self) -> Result<u8, PlatformError> {
        Err(PlatformError::Unsupported("fixture"))
    }

    fn set_warmth(&self, _: u8) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported("fixture"))
    }

    fn battery_percentage(&self) -> Result<u8, PlatformError> {
        Ok(100)
    }

    fn set_inverted(&self, _: bool) -> Result<(), PlatformError> {
        Err(PlatformError::Unsupported("fixture"))
    }

    fn reset_idle_timer(&self) -> Result<(), PlatformError> {
        Ok(())
    }
}

fn runtime_catalog_case() -> Result<()> {
    let fixture = Fixture::new_for_profile(
        "Offline detail capacity fixture",
        Vec::new(),
        false,
        DeviceProfile::Scribe,
    )?;
    {
        let mut state = fixture.app();
        let desktop = state.platform.clone();
        // This injected metadata never opens a native backend or physical device.
        state.platform = Arc::new(Platform::with_control(
            state.profile,
            true,
            Arc::new(ViewportControl),
        ));
        assert_eq!(state.platform.logical_viewport_size(), (930, 1480));
        assert_eq!(state.detail_page_size("toc"), 24);
        assert_eq!(state.detail_page_size("reviews"), 14);
        assert_eq!(state.detail_page_size("notes"), 13);
        assert_ne!(
            state.detail_page_size("toc"),
            state
                .profile
                .detail_page_size(state.profile.logical_size(), "toc")
        );
        let size = state.detail_page_size("toc");
        let count = size * 2 + 1;
        for ordinal in (2..count).rev() {
            state.store.upsert_chapter(&Chapter {
                id: format!("adaptive-detail-{ordinal}"),
                book_id: BOOK.into(),
                index: 100 + ordinal as u32 * 3,
                title: format!("Sparse chapter {ordinal}"),
                ..Default::default()
            })?;
        }
        let mut book = state.store.book(BOOK)?.unwrap();
        book.total_chapters = count as u32;
        state.store.upsert_book(&book)?;
        let catalog = state.store.chapter_metadata(BOOK)?;
        let current = &catalog[size + 1];
        state.store.save_position(&ReadingPosition {
            book_id: BOOK.into(),
            chapter_index: current.index,
            ..Default::default()
        })?;
        state.detail_id.clear();
        state.load_detail(BOOK)?;
        state.render_detail()?;
        let detail = fixture.ui().global::<DetailModel>();
        assert_eq!(detail.get_page(), 2);
        assert_eq!(detail.get_page_count(), 3);
        let first = detail.get_chapters().iter().collect::<Vec<_>>();
        assert_eq!(first.len(), size);
        assert_eq!(first[0].id.as_str(), catalog[size].id);
        assert!(
            first
                .iter()
                .any(|entry| entry.current && entry.id.as_str() == current.id)
        );
        assert_eq!(state.detail_current_chapter_ordinal(BOOK)?, Some(size + 1));

        state.detail_action("select", "")?;
        state.detail_status = "Transient fixture status".into();
        state.render_detail()?;
        let visible = detail.get_chapters().iter().collect::<Vec<_>>();
        assert_eq!(visible.len(), size);
        assert_eq!(visible[0].id, first[0].id);
        state.detail_action("select-page", "")?;
        assert_eq!(
            state.selected_chapters,
            visible
                .iter()
                .filter(|row| !row.cached)
                .map(|row| row.index as u32)
                .collect::<BTreeSet<_>>()
        );
        let selection = state.selected_chapters.clone();
        state.detail_action("page", "-1")?;
        state.render_detail()?;
        assert_eq!(detail.get_page(), 1);
        assert!(detail.get_selecting());
        assert_eq!(state.selected_chapters, selection);

        state.detail_action("jump", "")?;
        let shell = fixture.ui().global::<ShellModel>();
        assert_eq!(shell.get_dialog_kind(), "jump");
        shell.set_dialog_input("99".into());
        state.shell_action("dialog-primary", "")?;
        state.render_detail()?;
        assert_eq!(detail.get_page(), 3);
        assert!(detail.get_selecting());
        assert_eq!(state.selected_chapters, selection);

        state.detail_status.clear();
        state.detail_action("tab", "intro")?;
        state.render_detail()?;
        assert_eq!(detail.get_tab(), "toc");
        assert_eq!(state.detail_page, 1);
        assert!(state.selected_chapters.is_empty());
        assert!(!state.selecting_chapters);
        assert!(
            detail
                .get_chapters()
                .iter()
                .any(|row| row.current && row.id.as_str() == current.id)
        );

        state.detail_page = usize::MAX;
        state.render_detail()?;
        assert_eq!(detail.get_page(), 3);
        assert_eq!(detail.get_chapters().row_count(), 1);
        assert_eq!(
            detail.get_chapters().row_data(0).unwrap().id.as_str(),
            catalog[size * 2].id
        );
        state.detail_action("page", "1")?;
        state.render_detail()?;
        assert_eq!(detail.get_page(), 3);
        state.platform = desktop;
    }
    fixture.finish()
}

#[test]
fn runtime_detail_capacity_keeps_sparse_location_selection_and_last_page_consistent() -> Result<()>
{
    isolated(
        "app::library::detail_pagination_tests::runtime_detail_capacity_keeps_sparse_location_selection_and_last_page_consistent",
        runtime_catalog_case,
    )
}
