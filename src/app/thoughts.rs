//! Lightweight thought-card planning before preview reads or quote layout.

use super::underline_projection::ProjectedUnderline;
use crate::{domain::Annotation, source::RemoteReview};

pub(super) const CACHE_BYTES: usize = 1024 * 1024;
pub(super) const CACHE_RECORDS: usize = 4096;
pub(super) const PREVIEW_BYTES: usize = 8 * 1024;
pub(super) const QUOTE_BYTES: usize = 16 * 1024;
pub(super) const OPEN_TEXT_BYTES: usize = 64 * 1024;
pub(super) const MAX_VISIBLE_CARDS: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ThoughtView {
    List,
    Note(usize),
    Reviews,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum CardKind {
    Remote,
    Mine,
    Review,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ThoughtCard {
    pub kind: CardKind,
    pub index: usize,
    pub has_quote: bool,
}

impl ThoughtCard {
    pub fn height(self, scribe: bool) -> usize {
        match (self.has_quote, scribe) {
            (false, true) => 112,
            (false, false) => 104,
            (true, true) => 164,
            (true, false) => 144,
        }
    }
}

/// This iterator borrows metadata only. Counting or selecting another page
/// never opens cached review settings and never copies quote/body strings.
pub(super) fn cards<'a>(
    remote: &'a [ProjectedUnderline],
    annotations: &'a [Annotation],
    reviews: &'a [RemoteReview],
    view: ThoughtView,
    popular: bool,
    mine: bool,
    chapter: u32,
    start: u32,
    end: u32,
) -> impl Iterator<Item = ThoughtCard> + Clone + 'a {
    let public = remote
        .iter()
        .filter(move |_| view == ThoughtView::List && popular)
        .map(|range| ThoughtCard {
            kind: CardKind::Remote,
            index: range.index,
            has_quote: true,
        });
    let private = annotations
        .iter()
        .enumerate()
        .filter(move |(index, note)| match view {
            ThoughtView::Note(selected) => *index == selected,
            ThoughtView::List => {
                mine && note.chapter_index == chapter && note.start < end && start < note.end
            }
            ThoughtView::Reviews => false,
        })
        .map(|(index, note)| ThoughtCard {
            kind: CardKind::Mine,
            index,
            has_quote: !note.quote.is_empty(),
        });
    let review = reviews
        .iter()
        .enumerate()
        .filter(move |_| view == ThoughtView::Reviews)
        .map(|(index, _)| ThoughtCard {
            kind: CardKind::Review,
            index,
            has_quote: false,
        });
    public.chain(private).chain(review)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thought_descriptors_count_and_page_without_copying_content() {
        let remote = (0..100)
            .map(|index| ProjectedUnderline {
                index,
                start: 0,
                end: 2,
            })
            .collect::<Vec<_>>();
        let notes = [Annotation {
            start: 0,
            end: 2,
            quote: "local quote".into(),
            ..Default::default()
        }];
        let plan = cards(&remote, &notes, &[], ThoughtView::List, true, true, 0, 0, 3);
        assert_eq!(plan.clone().count(), 101);
        let visible = plan.skip(12).take(3).collect::<Vec<_>>();
        assert_eq!(
            visible.iter().map(|card| card.index).collect::<Vec<_>>(),
            [12, 13, 14]
        );
        assert!(visible.iter().all(|card| card.kind == CardKind::Remote));
    }

    #[test]
    fn opened_thought_views_select_only_their_own_card_family() {
        let remote = [ProjectedUnderline {
            index: 0,
            start: 0,
            end: 1,
        }];
        let notes = [
            Annotation::default(),
            Annotation {
                quote: "note".into(),
                ..Default::default()
            },
        ];
        let reviews = [RemoteReview::default(), RemoteReview::default()];
        let mine = cards(
            &remote,
            &notes,
            &reviews,
            ThoughtView::Note(1),
            true,
            true,
            0,
            0,
            2,
        )
        .collect::<Vec<_>>();
        assert_eq!(mine.len(), 1);
        assert_eq!((mine[0].kind, mine[0].index), (CardKind::Mine, 1));
        assert_eq!(
            cards(
                &remote,
                &notes,
                &reviews,
                ThoughtView::Reviews,
                true,
                true,
                0,
                0,
                2
            )
            .count(),
            2
        );
    }
}
