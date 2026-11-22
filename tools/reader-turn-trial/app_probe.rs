use super::{App, Route};
use crate::ui::{AppWindow, ReaderModel};
use slint::{ComponentHandle, Model};

const BOOK: &str = "pointer-turn-fixture";

impl App {
    pub fn trial_reader_page(
        &self,
        ui: &AppWindow,
    ) -> Option<(i32, i32, i32, i32, i32, i32, i32)> {
        if !self.force_offline
            || self.demo
            || self.jobs.configured()
            || !matches!(&self.route, Route::Reader(id, 0) if id == BOOK)
            || self.reader.book_id != BOOK
            || self.reader.chapter_index != 0
            || self.reader.selection.is_some()
        {
            return None;
        }
        let paginator = self.reader.paginator.as_ref()?;
        let ordinal = paginator.current_index();
        let page = paginator.current_page();
        let first = page.lines.first()?;
        let last = page.lines.last()?;
        let state = ui.global::<ReaderModel>();
        let rows = state.get_lines();
        if rows.row_count() != page.lines.len() || !page.images.is_empty() || page.start >= page.end
            || (ordinal == 0 && !first.text.starts_with("Section 0001"))
        {
            return None;
        }
        for (index, expected) in [(0, first), (rows.row_count() - 1, last)] {
            let actual = rows.row_data(index)?;
            if u32::try_from(actual.start).ok()? != expected.start
                || u32::try_from(actual.end).ok()? != expected.end
                || actual.text.as_str() != expected.text.as_str()
            {
                return None;
            }
        }
        let next = paginator.pages.get(ordinal.checked_add(1)?)?;
        Some((
            i32::try_from(ordinal + 1).ok()?,
            i32::try_from(page.start).ok()?,
            i32::try_from(first.start).ok()?,
            i32::try_from(next.start).ok()?,
            i32::try_from(next.lines.first()?.start).ok()?,
            i32::try_from(paginator.pages.len()).ok()?,
            i32::try_from(page.end).ok()?,
        ))
    }

    pub fn trial_reader_saved_anchor(&self, anchor: i32) -> bool {
        u32::try_from(anchor).ok().is_some_and(|anchor| {
            self.store.position(BOOK).ok().flatten().is_some_and(|saved| {
                saved.book_id == BOOK && saved.chapter_index == 0 && saved.char_offset == anchor
            })
        })
    }
}
