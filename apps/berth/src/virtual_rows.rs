//! Row descriptors backing a virtualized `gpui::list`, synced by splicing
//! only the changed range so scroll position survives updates.

use std::ops::Range;

use gpui::{ListAlignment, ListOffset, ListState, px};

pub struct VirtualRows<R> {
    pub state: ListState,
    rows: Vec<R>,
}

impl<R: PartialEq> VirtualRows<R> {
    pub fn new() -> Self {
        Self {
            state: ListState::new(0, ListAlignment::Top, px(400.0)),
            rows: Vec::new(),
        }
    }

    /// Returns whether any rows changed.
    pub fn sync(&mut self, rows: Vec<R>) -> bool {
        let splice = splice_range(&self.rows, &rows);
        if let Some((range, count)) = splice.clone() {
            self.state.splice(range, count);
        }
        self.rows = rows;
        splice.is_some()
    }

    /// Scroll row `ix` into view. Rows spliced in since the last layout have
    /// no measured height yet, so measured bounds can't be trusted after a
    /// change; unless the row is known to be fully visible it is pinned to
    /// the top, which needs no measurements.
    pub fn reveal(&self, ix: usize, rows_changed: bool) {
        let viewport = self.state.viewport_bounds();
        let visible = !rows_changed
            && self
                .state
                .bounds_for_item(ix)
                .is_some_and(|b| b.top() >= viewport.top() && b.bottom() <= viewport.bottom());
        if !visible {
            self.state.scroll_to(ListOffset {
                item_ix: ix,
                offset_in_item: px(0.0),
            });
        }
    }

    /// Scroll just far enough to show row `ix` (keyboard steps, where the
    /// rows haven't changed since the last layout).
    pub fn scroll_into_view(&self, ix: usize) {
        self.state.scroll_to_reveal_item(ix);
    }

    pub fn get(&self, ix: usize) -> Option<&R> {
        self.rows.get(ix)
    }

    pub fn position(&self, f: impl Fn(&R) -> bool) -> Option<usize> {
        self.rows.iter().position(f)
    }

    pub const fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// The minimal splice turning `old` into `new`: the changed middle between
/// the common prefix and suffix. `None` when nothing changed.
pub fn splice_range<R: PartialEq>(old: &[R], new: &[R]) -> Option<(Range<usize>, usize)> {
    if old == new {
        return None;
    }
    let prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()).saturating_sub(prefix);
    let suffix = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    Some((
        prefix..old.len().saturating_sub(suffix),
        new.len().saturating_sub(prefix).saturating_sub(suffix),
    ))
}

#[cfg(test)]
mod tests {
    use super::{VirtualRows, splice_range};

    #[test]
    fn reveal_pins_unmeasured_rows_to_the_top() {
        let mut rows = VirtualRows::new();
        assert!(rows.sync((0..50).collect()));
        assert!(!rows.sync((0..50).collect()));
        rows.reveal(30, false);
        assert_eq!(rows.state.logical_scroll_top().item_ix, 30);
        assert!(rows.sync((0..80).collect()));
        rows.reveal(70, true);
        assert_eq!(rows.state.logical_scroll_top().item_ix, 70);
    }

    #[test]
    fn splice_covers_only_the_changed_middle() {
        let old = ["a", "b", "c", "d"];
        assert_eq!(splice_range(&old, &old), None);
        assert_eq!(splice_range(&old, &["a", "c", "d"]), Some((1..2, 0)));
        assert_eq!(
            splice_range(&old, &["a", "b", "x", "c", "d"]),
            Some((2..2, 1))
        );
        assert_eq!(splice_range(&old, &["x"]), Some((0..4, 1)));
        assert_eq!(splice_range(&[], &["a"]), Some((0..0, 1)));
    }

    #[test]
    fn splice_handles_repeated_rows_without_overlap() {
        assert_eq!(splice_range(&["a", "a"], &["a"]), Some((1..2, 0)));
    }
}
