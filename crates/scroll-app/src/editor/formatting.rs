//! Bold, italic, strikethrough, code and links: toggling marks on the selection (or on what is
//! typed next), and the prompt for a link's address.

use gpui::{
    AnyElement, AppContext, Context, Entity, Focusable, IntoElement, ParentElement, Pixels, Point,
    Styled, Subscription, Window, div,
};
use scroll_ui::{Popover, TextInput, TextInputEvent, Theme};

use super::rich_text::MarkKind;
use super::{
    PageEditor, Selection, ToggleBold, ToggleCode, ToggleItalic, ToggleLink, ToggleStrikethrough,
};
use crate::editor::document::BlockKind;

const LINK_PROMPT_WIDTH: gpui::Pixels = gpui::px(300.0);

/// The link being entered for a selection after Cmd+K.
pub(super) struct LinkPrompt {
    input: Entity<TextInput>,
    target: Selection,
    position: Point<Pixels>,
    _subscription: Subscription,
}

impl PageEditor {
    pub(super) fn toggle_bold(&mut self, _: &ToggleBold, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_mark(MarkKind::Bold, cx);
    }

    pub(super) fn toggle_italic(
        &mut self,
        _: &ToggleItalic,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_mark(MarkKind::Italic, cx);
    }

    pub(super) fn toggle_strikethrough(
        &mut self,
        _: &ToggleStrikethrough,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_mark(MarkKind::Strikethrough, cx);
    }

    pub(super) fn toggle_code(&mut self, _: &ToggleCode, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_mark(MarkKind::Code, cx);
    }

    /// With a selection, adds or removes `kind` across it. With only a caret, flips the
    /// mark for the text typed next. Code blocks carry no marks.
    pub(super) fn toggle_mark(&mut self, kind: MarkKind, cx: &mut Context<Self>) {
        let selection = self.selection;
        let range = selection.range.range();
        if self.kind_of(selection.block) == BlockKind::Code {
            return;
        }
        let Some(content) = self.content(selection.block) else {
            return;
        };
        if range.is_empty() {
            let mut marks = self
                .pending_marks
                .clone()
                .unwrap_or_else(|| content.marks_at(range.start));
            marks.set(kind, !marks.has(kind));
            self.pending_marks = Some(marks);
            cx.notify();
            return;
        }
        let is_marked = content.is_marked_throughout(&range, kind);
        self.apply_edit(cx, |page| {
            if is_marked {
                page.remove_mark(selection.block, range.clone(), kind)?;
            } else {
                page.add_mark(selection.block, range.clone(), kind)?;
            }
            Ok(selection)
        });
    }

    /// Removes the link under the selection, or asks for a URL to link it to.
    pub(super) fn toggle_link(
        &mut self,
        _: &ToggleLink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = self.selection;
        let range = selection.range.range();
        let Some(content) = self.content(selection.block) else {
            return;
        };
        if range.is_empty() || self.kind_of(selection.block) == BlockKind::Code {
            return;
        }
        if content.is_marked_throughout(&range, MarkKind::Link) {
            self.apply_edit(cx, |page| {
                page.remove_mark(selection.block, range.clone(), MarkKind::Link)?;
                Ok(selection)
            });
            return;
        }
        self.open_link_prompt(selection, window, cx);
    }

    pub(super) fn apply_link(&mut self, target: Selection, url: &str, cx: &mut Context<Self>) {
        let range = target.range.range();
        self.apply_edit(cx, |page| {
            page.add_link(target.block, range, url)?;
            Ok(target)
        });
    }
}

impl PageEditor {
    pub(super) fn open_link_prompt(
        &mut self,
        target: Selection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = self.caret_anchor(target.block, target.range.range().start);
        let input = cx.new(|cx| TextInput::new(cx).placeholder("Paste or type a link"));
        let subscription = cx.subscribe_in(&input, window, |editor, input, event, window, cx| {
            if *event != TextInputEvent::Submitted {
                return;
            }
            let url = input.read(cx).text().trim().to_string();
            let target = editor.link_prompt.as_ref().map(|prompt| prompt.target);
            editor.close_link_prompt(window, cx);
            if let Some(target) = target.filter(|_| !url.is_empty()) {
                editor.apply_link(target, &url, cx);
            }
        });
        window.focus(&input.focus_handle(cx));
        self.link_prompt = Some(LinkPrompt {
            input,
            target,
            position,
            _subscription: subscription,
        });
        cx.notify();
    }

    pub(super) fn close_link_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.link_prompt = None;
        window.focus(&self.focus_handle);
        cx.notify();
    }
}

impl PageEditor {
    pub(super) fn render_link_prompt(
        &self,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let prompt = self.link_prompt.as_ref()?;
        let editor = cx.entity().downgrade();
        let popover = Popover::new(prompt.position)
            .on_dismiss(move |window, cx| {
                if let Err(error) =
                    editor.update(cx, |editor, cx| editor.close_link_prompt(window, cx))
                {
                    log::warn!("link prompt outlived its editor: {error}");
                }
            })
            .child(
                div()
                    .w(LINK_PROMPT_WIDTH)
                    .p(theme.spacing.xs)
                    .child(prompt.input.clone()),
            );
        Some(popover.into_any_element())
    }
}
