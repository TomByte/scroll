//! The platform text input protocol (typing, IME composition, dictation). Every range the
//! platform sends is UTF-16 within the selected block as displayed, which includes any
//! composition in progress. Composed text stays in the view until the input method commits
//! it; only then is it written to the document.

use std::ops::Range;

use gpui::{Bounds, Context, EntityInputHandler, Pixels, Point, UTF16Selection, Window, point};
use scroll_ui::text::{utf8_range_to_utf16, utf8_to_utf16, utf16_range_to_utf8};

use super::{Composition, PageEditor};

impl PageEditor {
    fn display_text(&self) -> String {
        self.display_content(self.selection.block).text.clone()
    }

    /// The document range an edit applies to when the platform names none.
    fn implicit_edit_range(&self) -> Range<usize> {
        self.selection.range.range()
    }

    /// Writes the composition (if any) to the document as typed text.
    fn commit_composition(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(composition) = self.composition.take() else {
            return;
        };
        let at = composition.offset;
        self.type_text(at..at, text, cx);
    }
}

impl EntityInputHandler for PageEditor {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let text = self.display_text();
        let range = utf16_range_to_utf8(&text, &range_utf16);
        actual_range.replace(utf8_range_to_utf16(&text, &range));
        Some(text[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let text = self.display_text();
        let selection = self.display_selection();
        Some(UTF16Selection {
            range: utf8_range_to_utf16(&text, &selection.range()),
            reversed: selection.is_reversed(),
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        let marked = self.display_marked_range()?;
        Some(utf8_range_to_utf16(&self.display_text(), &marked))
    }

    /// The input method accepted the composition as it stands.
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .composition
            .as_ref()
            .map(|composition| composition.text.clone());
        if let Some(text) = text {
            self.commit_composition(&text, cx);
        }
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composition.is_some() {
            return self.commit_composition(new_text, cx);
        }
        let range = range_utf16
            .map(|range| utf16_range_to_utf8(self.selected_text(), &range))
            .unwrap_or_else(|| self.implicit_edit_range());
        self.type_text(range, new_text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_read_only() {
            return;
        }
        if self.composition.is_none() {
            // Starting to compose over a selection removes it first; that part is a real edit.
            let range = range_utf16
                .map(|range| utf16_range_to_utf8(self.selected_text(), &range))
                .unwrap_or_else(|| self.implicit_edit_range());
            if !range.is_empty() {
                self.type_text(range.clone(), "", cx);
            }
            self.composition = Some(Composition {
                block: self.selection.block,
                offset: range.start,
                text: String::new(),
                selected: 0..0,
            });
        }
        let selected = new_selected_range_utf16
            .map(|selected| utf16_range_to_utf8(new_text, &selected))
            .unwrap_or(new_text.len()..new_text.len());
        if new_text.is_empty() {
            self.composition = None;
        } else if let Some(composition) = self.composition.as_mut() {
            composition.text = new_text.to_string();
            composition.selected = selected;
        }
        self.is_caret_visible = true;
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let range = utf16_range_to_utf8(&self.display_text(), &range_utf16);
        let layout = self.block_state(self.selection.block)?.layout.as_ref()?;
        let start = layout.caret_origin(range.start);
        let end = layout.caret_origin(range.end);
        let right = if end.y == start.y { end.x } else { start.x };
        Some(Bounds::from_corners(
            start,
            point(right, start.y + layout.line_height()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let layout = self.block_state(self.selection.block)?.layout.as_ref()?;
        let offset = layout.offset_for_point(position);
        Some(utf8_to_utf16(&self.display_text(), offset))
    }
}
