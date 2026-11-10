use std::time::Duration;

use gpui::Context;

use super::super::TableView;

/// How long after an edit a sorted or filtered view puts the edited row in its new place.
const RESORT_DELAY: Duration = Duration::from_millis(800);

impl TableView {
    /// After an edit to a value the view sorts or filters on, re-runs the query shortly, once the
    /// person is no longer editing, so the row moves to where it now belongs.
    pub(super) fn schedule_resort(&mut self, cx: &mut Context<Self>) {
        self.background.resort = Some(cx.spawn(async move |table, cx| {
            cx.background_executor().timer(RESORT_DELAY).await;
            // The table may have been closed in the meantime.
            let _ = table.update(cx, |table, cx| table.resort_when_idle(cx));
        }));
    }

    fn resort_when_idle(&mut self, cx: &mut Context<Self>) {
        if self.text_edit.is_some() || self.is_picking_a_value(cx) {
            self.schedule_resort(cx);
            return;
        }
        self.load(cx);
    }
}
