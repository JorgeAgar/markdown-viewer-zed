//! The tab list: an ordered list of items plus the index of the active one.
//!
//! GUI-free and generic so the rules (dedupe, which tab becomes active after
//! a close, wrap-around) can be unit-tested without a window. `Viewer` stores
//! its `Tab`s in a `Tabs<Tab>`.

/// Ordered tabs and the active index.
///
/// Invariant: when the list isn't empty, `active < len()`. When it's empty,
/// `active` is 0.
pub struct Tabs<T> {
    items: Vec<T>,
    active: usize,
}

impl<T> Default for Tabs<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            active: 0,
        }
    }
}

impl<T> Tabs<T> {
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.items.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.items.iter_mut()
    }

    /// Index of the active tab (0 when there are no tabs).
    pub fn active_index(&self) -> usize {
        self.active
    }

    pub fn active(&self) -> Option<&T> {
        self.items.get(self.active)
    }

    pub fn active_mut(&mut self) -> Option<&mut T> {
        self.items.get_mut(self.active)
    }

    /// Index of the first tab matching `pred`.
    pub fn position(&self, pred: impl FnMut(&T) -> bool) -> Option<usize> {
        self.items.iter().position(pred)
    }

    /// Makes tab `ix` active. Out-of-range indices are ignored.
    pub fn activate(&mut self, ix: usize) {
        if ix < self.items.len() {
            self.active = ix;
        }
    }

    /// Appends a tab and makes it active.
    pub fn push(&mut self, item: T) {
        self.items.push(item);
        self.active = self.items.len() - 1;
    }

    /// Activates the first tab matching `pred`. Returns whether there was one
    /// (opening a file that's already open just switches to its tab).
    pub fn activate_first(&mut self, pred: impl FnMut(&T) -> bool) -> bool {
        match self.position(pred) {
            Some(ix) => {
                self.active = ix;
                true
            }
            None => false,
        }
    }

    /// Removes tab `ix` and returns it (`None` if out of range).
    ///
    /// The active tab stays the same if it's still open. When the active tab
    /// itself is closed, the tab that slides into its place (the one to its
    /// right) becomes active, or the new last tab if it was the last one.
    pub fn close(&mut self, ix: usize) -> Option<T> {
        if ix >= self.items.len() {
            return None;
        }
        let item = self.items.remove(ix);
        if self.active > ix || self.active >= self.items.len() {
            self.active = self.active.saturating_sub(1);
        }
        Some(item)
    }

    /// Activates the next tab, wrapping around. No-op without tabs.
    pub fn next(&mut self) {
        if !self.items.is_empty() {
            self.active = (self.active + 1) % self.items.len();
        }
    }

    /// Activates the previous tab, wrapping around. No-op without tabs.
    pub fn prev(&mut self) {
        if !self.items.is_empty() {
            self.active = (self.active + self.items.len() - 1) % self.items.len();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(items: &[&'static str], active: usize) -> Tabs<&'static str> {
        let mut t = Tabs::default();
        for i in items {
            t.push(*i);
        }
        t.activate(active);
        assert_eq!(t.active_index(), active, "test setup");
        t
    }

    fn names(t: &Tabs<&'static str>) -> Vec<&'static str> {
        t.iter().copied().collect()
    }

    #[test]
    fn empty() {
        let mut t: Tabs<&str> = Tabs::default();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
        assert_eq!(t.active_index(), 0);
        assert_eq!(t.active(), None);
        assert_eq!(t.active_mut(), None);
        t.next();
        t.prev();
        t.activate(0);
        t.activate(5);
        assert_eq!(t.active_index(), 0);
        assert_eq!(t.close(0), None);
        assert_eq!(t.active_index(), 0);
    }

    #[test]
    fn push_activates_the_new_tab() {
        let mut t = Tabs::default();
        t.push("a");
        assert_eq!((t.active_index(), t.active()), (0, Some(&"a")));
        t.push("b");
        assert_eq!((t.active_index(), t.active()), (1, Some(&"b")));
        t.push("c");
        assert_eq!((t.active_index(), t.active()), (2, Some(&"c")));
        assert_eq!(t.len(), 3);
        assert!(!t.is_empty());
    }

    #[test]
    fn activate_ignores_out_of_range() {
        let mut t = tabs(&["a", "b", "c"], 1);
        t.activate(3);
        assert_eq!(t.active_index(), 1);
        t.activate(usize::MAX);
        assert_eq!(t.active_index(), 1);
        t.activate(2);
        assert_eq!(t.active_index(), 2);
        t.activate(0);
        assert_eq!(t.active_index(), 0);
    }

    #[test]
    fn activate_first_dedupes() {
        let mut t = tabs(&["a", "b", "c"], 2);
        assert!(t.activate_first(|x| *x == "a"));
        assert_eq!(t.active(), Some(&"a"));
        assert!(!t.activate_first(|x| *x == "d"));
        assert_eq!(
            t.active(),
            Some(&"a"),
            "no match leaves the active tab alone"
        );
        assert_eq!(t.len(), 3);
        // The first match wins.
        let mut t = tabs(&["a", "x", "x"], 0);
        assert!(t.activate_first(|x| *x == "x"));
        assert_eq!(t.active_index(), 1);
        // Re-opening the active tab keeps it active.
        assert!(t.activate_first(|x| *x == "x"));
        assert_eq!(t.active_index(), 1);
        let mut empty: Tabs<&str> = Tabs::default();
        assert!(!empty.activate_first(|_| true));
    }

    #[test]
    fn position() {
        let t = tabs(&["a", "b", "b"], 0);
        assert_eq!(t.position(|x| *x == "b"), Some(1));
        assert_eq!(t.position(|x| *x == "z"), None);
    }

    /// Every (len, active, closed) combination up to 5 tabs, checked against
    /// the rule written out by hand.
    #[test]
    #[allow(clippy::needless_range_loop, clippy::if_same_then_else)]
    fn close_exhaustive() {
        const ALL: [&str; 5] = ["a", "b", "c", "d", "e"];
        for len in 1..=ALL.len() {
            for active in 0..len {
                for ix in 0..len + 2 {
                    let mut t = tabs(&ALL[..len], active);
                    let was_active = ALL[active];
                    let closed = t.close(ix);
                    if ix >= len {
                        assert_eq!(closed, None, "len {len} active {active} close {ix}");
                        assert_eq!(t.len(), len);
                        assert_eq!(t.active_index(), active);
                        continue;
                    }
                    assert_eq!(closed, Some(ALL[ix]));
                    let mut expect: Vec<_> = ALL[..len].to_vec();
                    expect.remove(ix);
                    assert_eq!(names(&t), expect);
                    let want = if expect.is_empty() {
                        0
                    } else if ix < active {
                        active - 1 // same tab, shifted left
                    } else if ix > active {
                        active // unaffected
                    } else if active < expect.len() {
                        active // the right neighbor slides in
                    } else {
                        active - 1 // closed the last tab: its left neighbor
                    };
                    assert_eq!(
                        t.active_index(),
                        want,
                        "len {len} active {active} close {ix}"
                    );
                    if ix != active {
                        assert_eq!(t.active(), Some(&was_active));
                    }
                    if !t.is_empty() {
                        assert!(t.active_index() < t.len());
                    }
                }
            }
        }
    }

    #[test]
    fn close_examples() {
        // Before the active tab.
        let mut t = tabs(&["a", "b", "c"], 2);
        t.close(0);
        assert_eq!((names(&t), t.active()), (vec!["b", "c"], Some(&"c")));
        // The active tab, in the middle: the right neighbor takes over.
        let mut t = tabs(&["a", "b", "c"], 1);
        t.close(1);
        assert_eq!((names(&t), t.active()), (vec!["a", "c"], Some(&"c")));
        // The active tab, last: the left neighbor takes over.
        let mut t = tabs(&["a", "b", "c"], 2);
        t.close(2);
        assert_eq!((names(&t), t.active()), (vec!["a", "b"], Some(&"b")));
        // After the active tab.
        let mut t = tabs(&["a", "b", "c"], 0);
        t.close(2);
        assert_eq!((names(&t), t.active()), (vec!["a", "b"], Some(&"a")));
        // The only tab.
        let mut t = tabs(&["a"], 0);
        assert_eq!(t.close(0), Some("a"));
        assert!(t.is_empty());
        assert_eq!((t.active_index(), t.active()), (0, None));
        // Out of range.
        let mut t = tabs(&["a", "b"], 1);
        assert_eq!(t.close(2), None);
        assert_eq!(t.close(usize::MAX), None);
        assert_eq!((names(&t), t.active_index()), (vec!["a", "b"], 1));
    }

    #[test]
    fn close_all_one_by_one() {
        let mut t = tabs(&["a", "b", "c", "d"], 1);
        while !t.is_empty() {
            let ix = t.active_index();
            t.close(ix);
            if !t.is_empty() {
                assert!(t.active_index() < t.len());
            }
        }
        assert_eq!(t.active_index(), 0);
        t.push("z");
        assert_eq!(t.active(), Some(&"z"));
    }

    #[test]
    fn next_prev_wrap_around() {
        let mut t = tabs(&["a", "b", "c"], 0);
        let mut seen = Vec::new();
        for _ in 0..4 {
            t.next();
            seen.push(t.active_index());
        }
        assert_eq!(seen, [1, 2, 0, 1]);
        let mut seen = Vec::new();
        for _ in 0..4 {
            t.prev();
            seen.push(t.active_index());
        }
        assert_eq!(seen, [0, 2, 1, 0]);
    }

    #[test]
    fn next_prev_single_tab() {
        let mut t = tabs(&["a"], 0);
        t.next();
        assert_eq!(t.active_index(), 0);
        t.prev();
        assert_eq!(t.active_index(), 0);
    }

    #[test]
    fn next_prev_two_tabs() {
        let mut t = tabs(&["a", "b"], 1);
        t.next();
        assert_eq!(t.active_index(), 0);
        t.next();
        assert_eq!(t.active_index(), 1);
        t.prev();
        assert_eq!(t.active_index(), 0);
        t.prev();
        assert_eq!(t.active_index(), 1);
    }

    #[test]
    fn active_mut_and_iter_mut() {
        let mut t: Tabs<String> = Tabs::default();
        t.push("a".into());
        t.push("b".into());
        t.activate(0);
        t.active_mut().unwrap().push('!');
        for s in t.iter_mut() {
            s.push('?');
        }
        assert_eq!(t.iter().cloned().collect::<Vec<_>>(), ["a!?", "b?"]);
    }
}
