//! Multi-selection shared by resource lists (FR-12): click selects one,
//! ctrl/cmd-click toggles, shift-click extends from the anchor, `ctrl+a`
//! selects every visible key and `Esc` clears. The arrow keys, Home and End
//! step through the list ([`MultiSelect::step`]); with shift they extend.
//! Ctrl/cmd with the arrows moves only the cursor and ctrl+space toggles the
//! row under it, the keyboard's ctrl/cmd-click.

use std::collections::HashSet;
use std::hash::Hash;

use gpui::Modifiers;

/// Where a keyboard step moves the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Prev,
    Next,
    First,
    Last,
}

#[derive(Debug, Clone)]
pub struct MultiSelect<K> {
    selected: HashSet<K>,
    /// Where shift-ranges start.
    anchor: Option<K>,
    /// The key last clicked or stepped to; keyboard steps move from here.
    head: Option<K>,
}

impl<K> Default for MultiSelect<K> {
    fn default() -> Self {
        Self {
            selected: HashSet::new(),
            anchor: None,
            head: None,
        }
    }
}

impl<K: Clone + Eq + Hash> MultiSelect<K> {
    /// Apply a click on `key`. `order` is the visible order of selectable keys
    /// (used for shift-ranges).
    pub fn click(&mut self, key: &K, modifiers: Modifiers, order: &[K]) {
        if modifiers.shift
            && let Some(anchor) = &self.anchor
            && let (Some(a), Some(b)) = (
                order.iter().position(|k| k == anchor),
                order.iter().position(|k| k == key),
            )
        {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            if !modifiers.secondary() {
                self.selected.clear();
            }
            self.selected.extend(
                order
                    .iter()
                    .skip(lo)
                    .take(hi.saturating_sub(lo).saturating_add(1))
                    .cloned(),
            );
            self.head = Some(key.clone());
            return;
        }
        if modifiers.secondary() {
            if !self.selected.remove(key) {
                self.selected.insert(key.clone());
            }
        } else {
            self.selected.clear();
            self.selected.insert(key.clone());
        }
        self.anchor = Some(key.clone());
        self.head = Some(key.clone());
    }

    /// Select exactly `key`, as a plain click does.
    pub fn select_one(&mut self, key: K) {
        self.selected.clear();
        self.selected.insert(key.clone());
        self.anchor = Some(key.clone());
        self.head = Some(key);
    }

    /// The key a keyboard step from the last clicked or stepped-to key lands
    /// on. With nothing to move from, Next starts at the top and Prev at the
    /// bottom.
    fn target(&self, order: &[K], step: Step) -> Option<usize> {
        let last = order.len().checked_sub(1)?;
        let position = |key: Option<&K>| key.and_then(|k| order.iter().position(|o| o == k));
        let from = position(self.head.as_ref()).or_else(|| position(self.anchor.as_ref()));
        Some(match (step, from) {
            (Step::First, _) | (Step::Next, None) => 0,
            (Step::Last, _) | (Step::Prev, None) => last,
            (Step::Next, Some(ix)) => ix.saturating_add(1).min(last),
            (Step::Prev, Some(ix)) => ix.saturating_sub(1),
        })
    }

    /// Move by keyboard through `order` (the visible, selectable keys):
    /// select the neighbour of the last clicked or stepped-to key, or with
    /// `extend` select the range from the anchor to it. Returns the key
    /// moved to, to scroll into view.
    pub fn step(&mut self, order: &[K], step: Step, extend: bool) -> Option<K> {
        let to = self.target(order, step)?;
        let key = order.get(to)?.clone();
        let anchor = self
            .anchor
            .as_ref()
            .and_then(|a| order.iter().position(|o| o == a));
        match anchor.filter(|_| extend) {
            Some(anchor) => {
                let (lo, hi) = if anchor <= to {
                    (anchor, to)
                } else {
                    (to, anchor)
                };
                self.selected = order
                    .iter()
                    .skip(lo)
                    .take(hi.saturating_sub(lo).saturating_add(1))
                    .cloned()
                    .collect();
                self.head = Some(key.clone());
            }
            None => self.select_one(key.clone()),
        }
        Some(key)
    }

    /// Move the cursor (the key steps start from) without changing the
    /// selection, to pick rows apart with [`Self::toggle_head`].
    pub fn move_head(&mut self, order: &[K], step: Step) -> Option<K> {
        let key = order.get(self.target(order, step)?)?.clone();
        self.head = Some(key.clone());
        Some(key)
    }

    /// Add the cursor's key to the selection or drop it, as ctrl/cmd-click
    /// does.
    pub fn toggle_head(&mut self) {
        let Some(key) = self.head.clone() else { return };
        if !self.selected.remove(&key) {
            self.selected.insert(key.clone());
        }
        self.anchor = Some(key);
    }

    /// Whether `key` is under the cursor apart from the selection: moved
    /// to with ctrl/cmd+arrows, or the last toggled of several.
    pub fn is_cursor(&self, key: &K) -> bool {
        self.head.as_ref() == Some(key) && self.single() != Some(key)
    }

    /// Select every key in `order` (the visible, selectable keys).
    pub fn select_all(&mut self, order: &[K]) {
        self.selected = order.iter().cloned().collect();
        if self
            .anchor
            .as_ref()
            .is_none_or(|a| !self.selected.contains(a))
        {
            self.anchor = order.first().cloned();
        }
        self.head.clone_from(&self.anchor);
    }

    pub fn clear(&mut self) {
        self.selected.clear();
        self.anchor = None;
        self.head = None;
    }

    pub fn contains(&self, key: &K) -> bool {
        self.selected.contains(key)
    }

    pub fn len(&self) -> usize {
        self.selected.len()
    }

    pub fn is_empty(&self) -> bool {
        self.selected.is_empty()
    }

    /// The single selected key, if exactly one is selected.
    pub fn single(&self) -> Option<&K> {
        if self.selected.len() == 1 {
            self.selected.iter().next()
        } else {
            None
        }
    }

    /// Drop keys that no longer exist.
    pub fn retain(&mut self, exists: impl Fn(&K) -> bool) {
        self.selected.retain(|k| exists(k));
        if self.anchor.as_ref().is_some_and(|a| !exists(a)) {
            self.anchor = None;
        }
        if self.head.as_ref().is_some_and(|h| !exists(h)) {
            self.head = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(shift: bool, secondary: bool) -> Modifiers {
        let mac = cfg!(target_os = "macos");
        Modifiers {
            shift,
            platform: mac && secondary,
            control: !mac && secondary,
            ..Modifiers::default()
        }
    }

    #[test]
    fn click_toggle_and_range() {
        let order: Vec<u32> = (0..6).collect();
        let mut sel = MultiSelect::default();
        sel.click(&1, mods(false, false), &order);
        assert_eq!(sel.single(), Some(&1));

        sel.click(&4, mods(true, false), &order);
        assert_eq!(sel.len(), 4); // 1..=4

        sel.click(&2, mods(false, true), &order);
        assert!(!sel.contains(&2));
        assert_eq!(sel.len(), 3);

        sel.click(&5, mods(false, false), &order);
        assert_eq!(sel.single(), Some(&5));

        sel.retain(|k| *k != 5);
        assert!(sel.is_empty());
    }

    #[test]
    fn select_all_and_clear() {
        let order: Vec<u32> = (0..4).collect();
        let mut sel = MultiSelect::default();
        sel.select_all(&order);
        assert_eq!(sel.len(), 4);

        // The anchor is the first key, so shift-click ranges from there.
        sel.click(&2, mods(true, false), &order);
        assert_eq!(sel.len(), 3);

        sel.clear();
        assert!(sel.is_empty());
        sel.click(&3, mods(true, false), &order);
        assert_eq!(sel.single(), Some(&3));
    }

    #[test]
    fn step_moves_and_extends() {
        let order: Vec<u32> = (0..5).collect();
        let mut sel = MultiSelect::default();
        assert_eq!(sel.step(&[], Step::Next, false), None);

        // Nothing selected: Next starts at the top, Prev at the bottom.
        assert_eq!(sel.step(&order, Step::Next, false), Some(0));
        assert_eq!(sel.single(), Some(&0));
        sel.clear();
        assert_eq!(sel.step(&order, Step::Prev, false), Some(4));

        // Steps clamp at the ends.
        assert_eq!(sel.step(&order, Step::Next, false), Some(4));
        sel.click(&2, mods(false, false), &order);
        assert_eq!(sel.step(&order, Step::Prev, false), Some(1));
        assert_eq!(sel.single(), Some(&1));

        // Shift extends from the anchor and can shrink back past it.
        sel.step(&order, Step::Next, true);
        sel.step(&order, Step::Next, true);
        assert_eq!(sel.len(), 3); // 1..=3
        sel.step(&order, Step::First, true);
        assert_eq!(sel.len(), 2); // 0..=1
        assert!(sel.contains(&0) && sel.contains(&1));

        // A plain step collapses the range onto the moved-to key.
        assert_eq!(sel.step(&order, Step::Last, false), Some(4));
        assert_eq!(sel.single(), Some(&4));
    }

    #[test]
    fn cursor_toggles_rows_apart() {
        let order: Vec<u32> = (0..5).collect();
        let mut sel = MultiSelect::default();
        sel.step(&order, Step::Next, false); // 0
        sel.move_head(&order, Step::Next);
        assert_eq!(sel.move_head(&order, Step::Next), Some(2));
        assert_eq!(sel.single(), Some(&0)); // moving the cursor keeps it
        sel.toggle_head();
        assert!(sel.contains(&0) && !sel.contains(&1) && sel.contains(&2));
        sel.toggle_head();
        assert_eq!(sel.single(), Some(&0));
        // With nothing under the cursor, toggling does nothing.
        sel.clear();
        sel.toggle_head();
        assert!(sel.is_empty());
    }

    #[test]
    fn step_continues_from_a_toggled_key() {
        let order: Vec<u32> = (0..5).collect();
        let mut sel = MultiSelect::default();
        sel.click(&1, mods(false, false), &order);
        sel.click(&3, mods(false, true), &order);
        assert_eq!(sel.step(&order, Step::Next, false), Some(4));
    }
}
