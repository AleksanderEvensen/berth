//! Multi-selection shared by resource lists (FR-12): click selects one,
//! ctrl/cmd-click toggles, shift-click extends from the anchor, `ctrl+a`
//! selects every visible key and `Esc` clears.

use std::collections::HashSet;
use std::hash::Hash;

use gpui::Modifiers;

#[derive(Debug, Clone)]
pub struct MultiSelect<K> {
    selected: HashSet<K>,
    anchor: Option<K>,
}

impl<K> Default for MultiSelect<K> {
    fn default() -> Self {
        Self {
            selected: HashSet::new(),
            anchor: None,
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
    }

    /// Select exactly `key`, as a plain click does.
    pub fn select_one(&mut self, key: K) {
        self.selected.clear();
        self.selected.insert(key.clone());
        self.anchor = Some(key);
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
    }

    pub fn clear(&mut self) {
        self.selected.clear();
        self.anchor = None;
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
}
