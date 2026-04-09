//! Undo / redo snapshot stacks for the GUI.
//!
//! Scope is deliberately narrow: every snapshot stores the whole
//! layer list plus the selection index, because that's the state the
//! user most often wants to rewind. Viewport, shirt color, workflow
//! choice, and background-removal settings are *not* snapshotted —
//! tweaking those shouldn't pollute the undo stack with noise.
//!
//! The stack is a bounded ring (oldest entries get dropped when we
//! exceed [`HISTORY_CAP`]) to keep memory bounded on long sessions
//! with big layer masks.

use super::state::LayerEntry;

/// Maximum entries kept in the undo or redo stack. Each entry holds
/// a cloned `Vec<LayerEntry>`, and each `LayerEntry` owns two
/// `GrayImage` masks, so memory grows as
/// `HISTORY_CAP * layer_count * 2 * width * height` bytes. At 64
/// entries, 16 layers, 1024² px this is about 2 GiB — hence the cap.
pub const HISTORY_CAP: usize = 64;

/// One saved state the user can rewind to.
#[derive(Clone)]
pub struct Snapshot {
    pub layers: Vec<LayerEntry>,
    pub selected: Option<usize>,
}

/// Paired undo / redo stacks. A new edit pushes onto `undo` and
/// clears `redo`; `undo_pop` moves the top of `undo` onto `redo` and
/// returns the previous state; `redo_pop` does the reverse.
#[derive(Default)]
pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
}

impl History {
    #[allow(dead_code)] // used in tests; kept for future call sites
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot the current layers+selection onto the undo stack.
    /// Clears the redo stack — a new edit invalidates any pending
    /// redos, which matches how every other editor behaves.
    pub fn push(&mut self, snapshot: Snapshot) {
        self.undo.push(snapshot);
        if self.undo.len() > HISTORY_CAP {
            // Drop the oldest entry (front of the vec) so the stack
            // stays bounded. `remove(0)` is O(n) but n ≤ 64, so it's
            // fine compared to the cloned-mask cost we just paid.
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    /// Pop the most recent undo entry. Caller is expected to stash
    /// the *current* state onto the redo stack first (via
    /// [`Self::stash_redo`]) so it can be restored by Redo.
    pub fn undo_pop(&mut self) -> Option<Snapshot> {
        self.undo.pop()
    }

    /// Pop the most recent redo entry.
    pub fn redo_pop(&mut self) -> Option<Snapshot> {
        self.redo.pop()
    }

    /// Push onto the redo stack, used when applying an undo.
    pub fn stash_redo(&mut self, snapshot: Snapshot) {
        self.redo.push(snapshot);
        if self.redo.len() > HISTORY_CAP {
            self.redo.remove(0);
        }
    }

    /// Push onto the undo stack *without* clearing redo. Used when
    /// applying a redo so the inverse Undo still works.
    pub fn stash_undo(&mut self, snapshot: Snapshot) {
        self.undo.push(snapshot);
        if self.undo.len() > HISTORY_CAP {
            self.undo.remove(0);
        }
    }

    #[allow(dead_code)] // will be used to gate Undo/Redo menu items
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    #[allow(dead_code)] // will be used to gate Undo/Redo menu items
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Drop both stacks. Called on Open Image / Open Project so the
    /// user can't accidentally undo across a document boundary.
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(n: usize) -> Snapshot {
        Snapshot {
            layers: Vec::new(),
            selected: Some(n),
        }
    }

    #[test]
    fn push_then_undo_redo_roundtrip() {
        let mut h = History::new();
        h.push(snap(1));
        h.push(snap(2));
        h.push(snap(3));
        assert!(h.can_undo());
        assert!(!h.can_redo());

        // Pretend current state is snap(4). Undo gives us snap(3).
        let prev = h.undo_pop().unwrap();
        h.stash_redo(snap(4));
        assert_eq!(prev.selected, Some(3));
        assert!(h.can_redo());

        // Redo gives us back snap(4).
        let next = h.redo_pop().unwrap();
        h.stash_undo(snap(3));
        assert_eq!(next.selected, Some(4));
        assert!(!h.can_redo());
    }

    #[test]
    fn push_after_undo_clears_redo() {
        let mut h = History::new();
        h.push(snap(1));
        h.push(snap(2));
        let _ = h.undo_pop();
        h.stash_redo(snap(99));
        assert!(h.can_redo());
        // A new edit wipes the redo stack.
        h.push(snap(3));
        assert!(!h.can_redo());
    }

    #[test]
    fn stack_is_capped() {
        let mut h = History::new();
        for i in 0..(HISTORY_CAP + 10) {
            h.push(snap(i));
        }
        // After HISTORY_CAP + 10 pushes, only the last HISTORY_CAP
        // should still be present. The oldest ones got dropped from
        // the front.
        let mut count = 0;
        while h.undo_pop().is_some() {
            count += 1;
        }
        assert_eq!(count, HISTORY_CAP);
    }
}
