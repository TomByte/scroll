use crate::text::TextSelection;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub text: String,
    pub selection: TextSelection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum EditKind {
    Typing,
    Deleting,
    Other,
}

/// Undo history for a plain text field. Runs of typing or deleting collapse into one step
/// so that Cmd+Z removes a word or phrase rather than a single character.
#[derive(Default)]
pub(super) struct EditHistory {
    undo_stack: Vec<Snapshot>,
    redo_stack: Vec<Snapshot>,
    open_run: Option<EditKind>,
}

impl EditHistory {
    /// Records the state from just before an edit of `kind`.
    pub fn record(&mut self, before: Snapshot, kind: EditKind) {
        self.redo_stack.clear();
        let continues_run = kind != EditKind::Other && self.open_run == Some(kind);
        if !continues_run {
            self.undo_stack.push(before);
        }
        self.open_run = Some(kind);
    }

    /// Ends the current typing run, e.g. because the caret moved.
    pub fn close_run(&mut self) {
        self.open_run = None;
    }

    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let previous = self.undo_stack.pop()?;
        self.redo_stack.push(current);
        self.open_run = None;
        Some(previous)
    }

    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let next = self.redo_stack.pop()?;
        self.undo_stack.push(current);
        self.open_run = None;
        Some(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(text: &str) -> Snapshot {
        Snapshot {
            text: text.to_string(),
            selection: TextSelection::caret(text.len()),
        }
    }

    #[test]
    fn consecutive_typing_is_one_undo_step() {
        let mut history = EditHistory::default();
        history.record(snapshot(""), EditKind::Typing);
        history.record(snapshot("a"), EditKind::Typing);
        history.record(snapshot("ab"), EditKind::Typing);
        assert_eq!(history.undo(snapshot("abc")), Some(snapshot("")));
        assert_eq!(history.undo(snapshot("")), None);
    }

    #[test]
    fn moving_the_caret_splits_typing_runs() {
        let mut history = EditHistory::default();
        history.record(snapshot(""), EditKind::Typing);
        history.close_run();
        history.record(snapshot("a"), EditKind::Typing);
        assert_eq!(history.undo(snapshot("ab")), Some(snapshot("a")));
        assert_eq!(history.undo(snapshot("a")), Some(snapshot("")));
    }

    #[test]
    fn redo_restores_and_new_edits_clear_it() {
        let mut history = EditHistory::default();
        history.record(snapshot(""), EditKind::Typing);
        let undone = history.undo(snapshot("hi"));
        assert_eq!(undone, Some(snapshot("")));
        assert_eq!(history.redo(snapshot("")), Some(snapshot("hi")));
        history.undo(snapshot("hi"));
        history.record(snapshot(""), EditKind::Other);
        assert_eq!(history.redo(snapshot("x")), None);
    }
}
