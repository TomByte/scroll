use gpui::Context;
use scroll_core::operation::{FieldChange, Operation, Transaction};
use scroll_core::store::StoreError;

use super::super::TableView;
use crate::TableEvent;
use crate::table_model::changes_row_placement;

impl TableView {
    /// Applies a change on screen at once and queues it for the store. While read-only, only the
    /// view changes apply, for this session, and nothing is written.
    pub(in crate::table_view) fn write(
        &mut self,
        transaction: Transaction,
        cx: &mut Context<Self>,
    ) {
        if !self.can_edit() {
            self.change_view_for_session(transaction);
            cx.notify();
            return;
        }
        self.model.apply(&transaction);
        self.after_schema_change();
        if changes_row_placement(self.model.view(), self.model.database(), &transaction) {
            self.schedule_resort(cx);
        }
        self.background.writes.push_back(transaction);
        self.background.writes_queued += 1;
        self.write_next(cx);
        cx.notify();
    }

    fn change_view_for_session(&mut self, transaction: Transaction) {
        let view = self.model.view().id;
        let view_changes = transaction.operations.into_iter().filter(|operation| {
            matches!(operation, Operation::Set(FieldChange::View { id, .. }) if *id == view)
        });
        self.model.apply(&Transaction::new(view_changes.collect()));
        self.session_view = Some(self.model.view().clone());
        self.after_schema_change();
    }

    fn write_next(&mut self, cx: &mut Context<Self>) {
        if self.background.is_writing {
            return;
        }
        let Some(transaction) = self.background.writes.pop_front() else {
            if std::mem::take(&mut self.background.wants_load) {
                self.load(cx);
            }
            return;
        };
        self.background.is_writing = true;
        let storage = self.storage.clone();
        let background = cx.background_executor().clone();
        cx.spawn(async move |table, cx| {
            let result = background
                .spawn(async move { storage.write(&transaction) })
                .await;
            // The table may have been closed while writing; the write itself has finished.
            let _ = table.update(cx, |table, cx| table.finish_write(result, cx));
        })
        .detach();
    }

    fn finish_write(&mut self, result: Result<(), StoreError>, cx: &mut Context<Self>) {
        self.background.is_writing = false;
        match result {
            Ok(()) => cx.emit(TableEvent::Changed),
            Err(error) => {
                self.show_error(&error);
                self.background.wants_load = true;
            }
        }
        self.write_next(cx);
        cx.notify();
    }
}
