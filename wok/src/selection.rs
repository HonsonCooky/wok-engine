//! The editor's selection: an ordered, duplicate-free set of placements, the last-added member
//! carrying authority.
//!
//! v2 multi-select turns the single `Option<Selection>` into a set so Ctrl+click and the marquee
//! have something to grow (the UX that drives them is a later brief; this type ships first, holding
//! zero or one item, so the editor's behavior is unchanged). Membership is insertion order with no
//! duplicates, and the last-added member is the *primary* - the one the inspector edits and the
//! camera frames - so selecting an item, or toggling it back in, makes it primary.
//!
//! It is deliberately a thin `Vec` wrapper rather than a hash set: an editor selection is a handful
//! of items, occasionally more, never enough for a linear `contains` to cost anything next to the
//! per-frame slicing around it, and the vec keeps insertion order for free, which is exactly what
//! the primary rule needs. The field is private so the no-duplicates and ordering invariants hold
//! by construction.

use crate::model::Selection;

/// An ordered, duplicate-free set of selected placements. The last element is the primary
/// selection; the empty set is no selection at all.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectionSet {
    items: Vec<Selection>,
}

impl SelectionSet {
    /// The empty selection.
    pub fn new() -> SelectionSet {
        SelectionSet { items: Vec::new() }
    }

    /// Nothing is selected.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// How many placements are selected.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Is this placement in the selection?
    pub fn contains(&self, sel: Selection) -> bool {
        self.items.contains(&sel)
    }

    /// Is `sel` the one and only member? The reposition drag arms only on a sole selection, so a
    /// left-press on one of several selected placements does nothing until group-move (part 2).
    pub fn is_only(&self, sel: Selection) -> bool {
        self.items.len() == 1 && self.items[0] == sel
    }

    /// The primary selection - the last-added member - or `None` when the set is empty. What the
    /// inspector edits and the camera frames.
    pub fn primary(&self) -> Option<Selection> {
        self.items.last().copied()
    }

    /// The members in insertion order, primary last.
    pub fn iter(&self) -> impl Iterator<Item = Selection> + '_ {
        self.items.iter().copied()
    }

    /// Replace the whole selection with one item: clear, then add it as the sole (and primary)
    /// member. The plain single-click select.
    pub fn replace(&mut self, sel: Selection) {
        self.items.clear();
        self.items.push(sel);
    }

    /// Clear the selection.
    pub fn clear(&mut self) {
        self.items.clear();
    }

    /// Keep only the members `keep` accepts, in order. Prunes selections whose placement no longer
    /// resolves after a delete or an external reload.
    pub fn retain(&mut self, keep: impl FnMut(&Selection) -> bool) {
        self.items.retain(keep);
    }

    /// Toggle a placement in or out, keeping the set ordered and duplicate-free: an absent item is
    /// added as the new primary, a present one is removed (and the member now last becomes primary).
    /// The Ctrl+click verb.
    pub fn toggle(&mut self, sel: Selection) {
        match self.items.iter().position(|s| *s == sel) {
            Some(i) => {
                self.items.remove(i);
            }
            None => self.items.push(sel),
        }
    }

    /// Add several items in order, optionally clearing first: `add == false` replaces the selection,
    /// `add == true` extends it, and either way items already present are skipped so the set stays
    /// duplicate-free. Used to reselect a duplicated group; the marquee will extend with it too.
    pub fn extend(&mut self, items: impl IntoIterator<Item = Selection>, add: bool) {
        if !add {
            self.items.clear();
        }
        for sel in items {
            if !self.items.contains(&sel) {
                self.items.push(sel);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wok_scene::{ChunkCoord, InstanceId};

    fn sel(id: u32) -> Selection {
        Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(id) }
    }

    #[test]
    fn empty_set_has_no_primary_and_contains_nothing() {
        let set = SelectionSet::new();
        assert!(set.is_empty());
        assert_eq!(set.primary(), None);
        assert!(!set.contains(sel(0)));
    }

    #[test]
    fn replace_holds_exactly_one_item_as_primary() {
        let mut set = SelectionSet::new();
        set.replace(sel(3));
        assert!(!set.is_empty());
        assert!(set.contains(sel(3)));
        assert_eq!(set.primary(), Some(sel(3)));
        assert_eq!(set.iter().collect::<Vec<_>>(), vec![sel(3)]);

        // Replace discards the prior selection rather than growing the set.
        set.replace(sel(5));
        assert!(!set.contains(sel(3)));
        assert_eq!(set.primary(), Some(sel(5)));
        assert_eq!(set.iter().collect::<Vec<_>>(), vec![sel(5)]);
    }

    #[test]
    fn clear_empties_the_set() {
        let mut set = SelectionSet::new();
        set.replace(sel(2));
        set.clear();
        assert!(set.is_empty());
        assert_eq!(set.primary(), None);
    }

    #[test]
    fn toggle_adds_then_removes_and_moves_primary_to_the_last_member() {
        let mut set = SelectionSet::new();
        set.toggle(sel(1));
        set.toggle(sel(2));
        // Last added is primary; both are members, in order.
        assert_eq!(set.iter().collect::<Vec<_>>(), vec![sel(1), sel(2)]);
        assert_eq!(set.primary(), Some(sel(2)));

        // Toggling the primary back out hands primary to what remains.
        set.toggle(sel(2));
        assert!(!set.contains(sel(2)));
        assert_eq!(set.primary(), Some(sel(1)));
    }

    #[test]
    fn extend_replaces_or_adds_and_never_duplicates() {
        let mut set = SelectionSet::new();
        // add == false replaces, keeping order, dropping repeats within the batch.
        set.extend([sel(1), sel(2), sel(1)], false);
        assert_eq!(set.iter().collect::<Vec<_>>(), vec![sel(1), sel(2)]);

        // add == true extends, skipping members already present; the last new item is primary.
        set.extend([sel(2), sel(3)], true);
        assert_eq!(set.iter().collect::<Vec<_>>(), vec![sel(1), sel(2), sel(3)]);
        assert_eq!(set.primary(), Some(sel(3)));

        // add == false again replaces the whole set.
        set.extend([sel(9)], false);
        assert_eq!(set.iter().collect::<Vec<_>>(), vec![sel(9)]);
    }

    #[test]
    fn is_only_and_len_track_a_sole_member() {
        let mut set = SelectionSet::new();
        assert_eq!(set.len(), 0);
        assert!(!set.is_only(sel(1)));

        set.replace(sel(1));
        assert_eq!(set.len(), 1);
        assert!(set.is_only(sel(1)), "the sole member");
        assert!(!set.is_only(sel(2)), "a different placement is not the sole member");

        set.toggle(sel(2));
        assert_eq!(set.len(), 2);
        assert!(!set.is_only(sel(1)), "no member is sole once two are selected");
        assert!(!set.is_only(sel(2)));
    }

    #[test]
    fn retain_prunes_in_place_and_keeps_order() {
        let mut set = SelectionSet::new();
        set.extend([sel(1), sel(2), sel(3)], false);
        set.retain(|s| s.id != InstanceId(2));
        assert_eq!(set.iter().collect::<Vec<_>>(), vec![sel(1), sel(3)]);
        assert_eq!(set.primary(), Some(sel(3)));
    }
}
