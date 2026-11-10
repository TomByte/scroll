//! Undo and redo of local edits, with the selection restored.

use std::sync::{Arc, Mutex};

use loro::cursor::{AbsolutePosition, Cursor};
use loro::undo::CursorWithPos;
use loro::{LoroDoc, LoroValue, UndoItemMeta, UndoManager};

use super::{PageDoc, PageDocError, lock};

/// Consecutive edits within this window undo as one step, so undo removes a burst of typing, not one letter.
pub(super) const UNDO_MERGE_INTERVAL_MS: i64 = 600;

/// Keeps the selection with each undo step, and hands it back when the step is undone or redone.
pub(super) fn undo_manager(
    doc: &LoroDoc,
    selection_before_edit: &Arc<Mutex<Vec<Cursor>>>,
    restored_selection: &Arc<Mutex<Vec<Cursor>>>,
) -> UndoManager {
    let mut undo = UndoManager::new(doc);
    undo.set_merge_interval(UNDO_MERGE_INTERVAL_MS);
    let selection_before_edit = selection_before_edit.clone();
    undo.set_on_push(Some(Box::new(move |_, _, _| UndoItemMeta {
        value: LoroValue::Null,
        // Loro moves these cursors along with later edits; the position is recomputed on pop.
        cursors: lock(&selection_before_edit)
            .iter()
            .map(|cursor| CursorWithPos {
                cursor: cursor.clone(),
                pos: AbsolutePosition {
                    pos: 0,
                    side: cursor.side,
                },
            })
            .collect(),
    })));
    let restored_selection = restored_selection.clone();
    undo.set_on_pop(Some(Box::new(move |_, _, meta| {
        *lock(&restored_selection) = meta
            .cursors
            .into_iter()
            .map(|cursor| cursor.cursor)
            .collect();
    })));
    undo
}

impl PageDoc {
    /// Runs several edits as one undo step, for example a paste that inserts several blocks.
    /// Loro cannot roll back, so if `edit` fails part way, the edits made before the failure stay.
    pub fn edit_as_one_step<T>(
        &mut self,
        edit: impl FnOnce(&PageDoc) -> Result<T, PageDocError>,
    ) -> Result<T, PageDocError> {
        self.doc.commit();
        self.undo.group_start()?;
        let result = edit(self);
        self.doc.commit();
        self.undo.group_end();
        result
    }

    /// The selection to restore if the next local edit is undone. Call it before each edit with
    /// the selection as it is then.
    pub fn remember_selection(&self, selection: &[Cursor]) {
        *lock(&self.selection_before_edit) = selection.to_vec();
    }

    /// Undoes this peer's last edit step; remote edits are never undone. Returns the selection
    /// remembered before that step, moved along by any edits since, or nothing if there was none.
    pub fn undo(&mut self) -> Result<Vec<Cursor>, PageDocError> {
        self.undo.undo()?;
        Ok(std::mem::take(&mut *lock(&self.restored_selection)))
    }

    pub fn redo(&mut self) -> Result<Vec<Cursor>, PageDocError> {
        self.undo.redo()?;
        Ok(std::mem::take(&mut *lock(&self.restored_selection)))
    }
}
