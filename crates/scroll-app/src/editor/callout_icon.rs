//! Choosing a callout's emoji: clicking the icon opens the kit's emoji picker under it.

use gpui::{ClickEvent, Context, Window};
use scroll_ui::{EmojiPicker, EmojiPickerEvent, PopupAnchor};

use super::PageEditor;
use crate::editor::document::BlockId;

/// What a callout shows before anyone picks an icon.
pub const DEFAULT_CALLOUT_ICON: &str = "💡";

impl PageEditor {
    pub(super) fn callout_icon(&self, block: BlockId) -> String {
        self.page
            .block_icon(block)
            .unwrap_or_else(|| DEFAULT_CALLOUT_ICON.to_string())
    }

    pub(super) fn open_callout_picker(
        &mut self,
        block: BlockId,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_read_only() {
            return;
        }
        let position = event.position();
        let picker = self.popups.update(cx, |popups, cx| {
            popups.show(
                PopupAnchor::Point(position),
                |_, cx| EmojiPicker::new(cx),
                window,
                cx,
            )
        });
        self.callout_picker_subscription = Some(cx.subscribe(
            &picker,
            move |editor, _, event: &EmojiPickerEvent, cx| {
                if let EmojiPickerEvent::Picked(emoji) = event {
                    editor.set_callout_icon(block, emoji.to_string(), cx);
                }
            },
        ));
    }

    fn set_callout_icon(&mut self, block: BlockId, emoji: String, cx: &mut Context<Self>) {
        let selection = self.selection;
        self.apply_edit(cx, move |page| {
            page.set_block_icon(block, Some(&emoji))?;
            Ok(selection)
        });
    }
}
