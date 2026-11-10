//! Completions offered at the caret while typing: `/` at the start of a word lists block
//! kinds to turn the block into, and `:` lists emoji by name. Typing narrows the list, the
//! arrows move the highlight, Enter (and Tab for emoji) chooses, and Escape closes it; moving
//! the caret out of the typed word closes it too. Typing the closing `:` after an exact emoji
//! name inserts that emoji at once.

use gpui::{
    AnyElement, Context, ElementId, IntoElement, ParentElement, Pixels, Styled, div,
    prelude::FluentBuilder, px,
};
use scroll_ui::emoji::{emoji_named, remember_emoji, search_emoji};
use scroll_ui::{EmptyState, ListItem, Popover, SectionHeader};

use super::slash_commands::{SlashAction, matching_commands};
use super::{PageEditor, PageEditorEvent, Selection};
use crate::editor::document::BlockId;

/// How many emoji the completion lists.
const EMOJI_LIMIT: usize = 8;
/// Fewer typed characters than this match too many emoji to be useful.
const EMOJI_MINIMUM_QUERY: usize = 2;
const SLASH_MENU_WIDTH: Pixels = px(320.0);
const EMOJI_MENU_WIDTH: Pixels = px(280.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CompletionKind {
    /// Block kinds, after `/`.
    Block,
    /// Emoji by name, after `:`.
    Emoji,
}

impl CompletionKind {
    fn opened_by(typed: &str) -> Option<CompletionKind> {
        match typed {
            "/" => Some(CompletionKind::Block),
            ":" => Some(CompletionKind::Emoji),
            _ => None,
        }
    }

    fn trigger(self) -> &'static str {
        match self {
            CompletionKind::Block => "/",
            CompletionKind::Emoji => ":",
        }
    }

    /// Whether `query` can still become a match: block names are free text, emoji names
    /// are one word.
    fn accepts(self, query: &str) -> bool {
        match self {
            CompletionKind::Block => true,
            CompletionKind::Emoji => query.chars().all(|character| {
                character.is_alphanumeric() || matches!(character, '_' | '-' | ':')
            }),
        }
    }

    /// The block list stays up saying nothing matches; the emoji list only shows matches,
    /// since `:` is also ordinary punctuation.
    fn shows_when_empty(self) -> bool {
        self == CompletionKind::Block
    }

    fn width(self) -> Pixels {
        match self {
            CompletionKind::Block => SLASH_MENU_WIDTH,
            CompletionKind::Emoji => EMOJI_MENU_WIDTH,
        }
    }
}

/// An open completion: where its trigger was typed and which row is highlighted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct InlineCompletion {
    pub(super) kind: CompletionKind,
    block: BlockId,
    trigger_offset: usize,
    highlighted: usize,
}

/// One row of the list.
enum Completion {
    Block(super::slash_commands::SlashCommand),
    Emoji(&'static scroll_ui::emoji::Emoji),
}

impl PageEditor {
    /// Reacts to `typed`: `/` or `:` at the start of a word opens a completion. Returns
    /// whether `typed` was such a trigger, so it is not also read as Markdown.
    pub(super) fn open_completion_for(
        &mut self,
        typed: &str,
        text_before_caret: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if typed == ":" && self.complete_closed_emoji_name(cx) {
            return true;
        }
        self.close_completion_if_left();
        let Some(kind) = CompletionKind::opened_by(typed) else {
            return false;
        };
        let caret = self.selection.range.head;
        let is_word_start = text_before_caret[..caret - 1]
            .chars()
            .next_back()
            .is_none_or(char::is_whitespace);
        if is_word_start {
            self.completion = Some(InlineCompletion {
                kind,
                block: self.selection.block,
                trigger_offset: caret - 1,
                highlighted: 0,
            });
        }
        true
    }

    /// Closes the completion once the caret has left the word it completes.
    pub(super) fn close_completion_if_left(&mut self) {
        if self.completion_query().is_none() {
            self.completion = None;
        }
    }

    /// What follows the trigger, while the caret is still in that word.
    fn completion_query(&self) -> Option<String> {
        let completion = self.completion?;
        let text = self.content(completion.block)?.text.as_str();
        let caret = self.selection.range.head;
        let start = completion.trigger_offset;
        let is_still_open = self.selection.block == completion.block
            && caret > start
            && text.get(start..start + 1) == Some(completion.kind.trigger());
        let query = text.get(start + 1..caret).filter(|_| is_still_open)?;
        completion.kind.accepts(query).then(|| query.to_string())
    }

    fn completions(&self) -> Vec<Completion> {
        let (Some(completion), Some(query)) = (self.completion, self.completion_query()) else {
            return Vec::new();
        };
        match completion.kind {
            CompletionKind::Block => matching_commands(&query)
                .into_iter()
                .map(Completion::Block)
                .collect(),
            CompletionKind::Emoji if query.chars().count() >= EMOJI_MINIMUM_QUERY => {
                search_emoji(&query)
                    .into_iter()
                    .take(EMOJI_LIMIT)
                    .map(Completion::Emoji)
                    .collect()
            }
            CompletionKind::Emoji => Vec::new(),
        }
    }

    /// Whether the list is on screen, and so takes the arrows, Enter and Escape.
    pub(super) fn is_completion_showing(&self) -> bool {
        let Some(completion) = self
            .completion
            .filter(|_| self.completion_query().is_some())
        else {
            return false;
        };
        completion.kind.shows_when_empty() || !self.completions().is_empty()
    }

    pub(super) fn completion_kind(&self) -> Option<CompletionKind> {
        self.completion.map(|completion| completion.kind)
    }

    /// Moves the highlight; false when no list is showing.
    pub(super) fn move_completion_highlight(
        &mut self,
        step: isize,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.is_completion_showing() {
            return false;
        }
        let count = self.completions().len();
        if let Some(completion) = self.completion.as_mut()
            && count > 0
        {
            completion.highlighted =
                (completion.highlighted as isize + step).rem_euclid(count as isize) as usize;
            cx.notify();
        }
        true
    }

    /// Chooses the highlighted row; false when no list is showing. A block list with nothing
    /// matching just closes.
    pub(super) fn choose_highlighted_completion(&mut self, cx: &mut Context<Self>) -> bool {
        if !self.is_completion_showing() {
            return false;
        }
        let highlighted = self
            .completion
            .map_or(0, |completion| completion.highlighted);
        self.choose_completion(highlighted, cx);
        true
    }

    fn choose_completion(&mut self, index: usize, cx: &mut Context<Self>) {
        let rows = self.completions();
        let Some(completion) = self.completion.take() else {
            return;
        };
        let typed = completion.trigger_offset..self.selection.range.head;
        match rows.into_iter().nth(index) {
            Some(Completion::Block(command)) => match command.action {
                SlashAction::TurnInto(kind) => {
                    self.convert_block(completion.block, kind, typed, cx)
                }
                SlashAction::MermaidDiagram => self.convert_to_diagram(completion.block, typed, cx),
                SlashAction::CreateDatabase => {
                    self.delete_range(typed, cx);
                    cx.emit(PageEditorEvent::CreateDatabase);
                }
            },
            Some(Completion::Emoji(emoji)) => {
                self.replace_with_emoji(completion.block, typed, emoji.emoji, cx)
            }
            None => cx.notify(),
        }
    }

    /// Closes the list; returns whether one was open.
    pub(super) fn close_completion(&mut self, cx: &mut Context<Self>) -> bool {
        let was_open = self.completion.take().is_some();
        cx.notify();
        was_open
    }

    /// After `:` closes an exact name (`:red_heart:`), replaces it with the emoji.
    fn complete_closed_emoji_name(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(completion) = self
            .completion
            .filter(|completion| completion.kind == CompletionKind::Emoji)
        else {
            return false;
        };
        let Some(query) = self.completion_query() else {
            return false;
        };
        let Some(emoji) = query.strip_suffix(':').and_then(emoji_named) else {
            return false;
        };
        self.completion = None;
        let typed = completion.trigger_offset..self.selection.range.head;
        self.replace_with_emoji(completion.block, typed, emoji.emoji, cx);
        true
    }

    /// Replaces the typed `:query` with `emoji` and remembers it as recently used.
    fn replace_with_emoji(
        &mut self,
        block: BlockId,
        typed: std::ops::Range<usize>,
        emoji: &'static str,
        cx: &mut Context<Self>,
    ) {
        remember_emoji(emoji, cx);
        self.apply_edit(cx, move |page| {
            page.delete_text(block, typed.clone())?;
            page.insert_text(block, typed.start, emoji)?;
            Ok(Selection::caret(block, typed.start + emoji.len()))
        });
    }

    pub(super) fn render_completion(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let completion = self.completion.filter(|_| self.is_completion_showing())?;
        let rows: Vec<AnyElement> = self
            .completions()
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let id = ElementId::named_usize("completion", index);
                let item = match row {
                    Completion::Block(command) => ListItem::new(id, command.label)
                        .icon(command.icon)
                        .end_slot(command.hint),
                    Completion::Emoji(emoji) => ListItem::new(id, emoji.name).emoji(emoji.emoji),
                };
                item.emphasized()
                    .when(index == completion.highlighted, ListItem::highlighted)
                    .on_click(
                        cx.listener(move |editor, _, _, cx| editor.choose_completion(index, cx)),
                    )
                    .into_any_element()
            })
            .collect();
        let is_block = completion.kind == CompletionKind::Block;
        let editor = cx.entity().downgrade();
        let (position, corner) = self.completion_position(completion);
        Some(
            Popover::new(position)
                .anchor(corner)
                .on_dismiss(move |_, cx| {
                    if let Err(error) = editor.update(cx, |editor, cx| editor.close_completion(cx))
                    {
                        log::debug!("a completion outlived its editor: {error}");
                    }
                })
                .child(
                    div()
                        .w(completion.kind.width())
                        .when(is_block, |list| list.child(SectionHeader::new("Blocks")))
                        .when(rows.is_empty(), |list| {
                            list.child(EmptyState::new("No matching blocks").inline())
                        })
                        .children(rows),
                )
                .into_any_element(),
        )
    }

    /// Under the trigger, or above its line near the bottom of the view so the list does not
    /// cover what is being typed.
    fn completion_position(
        &self,
        completion: InlineCompletion,
    ) -> (gpui::Point<Pixels>, gpui::Corner) {
        let below_caret = self.caret_anchor(completion.block, completion.trigger_offset);
        let opens_above = self
            .viewport
            .is_some_and(|viewport| below_caret.y > viewport.center().y);
        match self
            .block_state(completion.block)
            .and_then(|state| state.layout.as_ref())
        {
            Some(layout) if opens_above => (
                gpui::point(below_caret.x, below_caret.y - layout.line_height()),
                gpui::Corner::BottomLeft,
            ),
            _ => (below_caret, gpui::Corner::TopLeft),
        }
    }
}
