//! Row descriptors backing a virtualized `gpui::list`, synced by splicing
//! only the changed range so scroll position survives updates.

use std::ops::Range;

use gpui::{ListAlignment, ListState, px};

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

    pub fn sync(&mut self, rows: Vec<R>) {
        if let Some((range, count)) = splice_range(&self.rows, &rows) {
            self.state.splice(range, count);
        }
        self.rows = rows;
    }

    pub fn get(&self, ix: usize) -> Option<&R> {
        self.rows.get(ix)
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
    use super::splice_range;

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
