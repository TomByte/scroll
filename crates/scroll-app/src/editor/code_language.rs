//! A code block's language: the label at its top right, the menu that picks one, and the
//! grammar each one is highlighted with.

use gpui::{
    AnyElement, ClickEvent, Context, Div, ElementId, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Styled, Window, div, prelude::FluentBuilder,
};
use scroll_ui::{Clickable, Menu, MenuItem, PopupAnchor, Theme};
use syntect::parsing::{SyntaxReference, SyntaxSet};

use super::PageEditor;
use super::syntax;
use crate::editor::document::BlockId;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CodeLanguage {
    PlainText,
    Bash,
    C,
    Cpp,
    CSharp,
    Css,
    Diff,
    Go,
    Html,
    Java,
    JavaScript,
    Json,
    Markdown,
    Mermaid,
    Php,
    Python,
    Ruby,
    Rust,
    Sql,
    TypeScript,
    Xml,
    Yaml,
}

impl CodeLanguage {
    /// In the order the menu lists them.
    pub const ALL: &[CodeLanguage] = &[
        CodeLanguage::PlainText,
        CodeLanguage::Bash,
        CodeLanguage::C,
        CodeLanguage::Cpp,
        CodeLanguage::CSharp,
        CodeLanguage::Css,
        CodeLanguage::Diff,
        CodeLanguage::Go,
        CodeLanguage::Html,
        CodeLanguage::Java,
        CodeLanguage::JavaScript,
        CodeLanguage::Json,
        CodeLanguage::Markdown,
        CodeLanguage::Mermaid,
        CodeLanguage::Php,
        CodeLanguage::Python,
        CodeLanguage::Ruby,
        CodeLanguage::Rust,
        CodeLanguage::Sql,
        CodeLanguage::TypeScript,
        CodeLanguage::Xml,
        CodeLanguage::Yaml,
    ];

    /// The id stored in the page; plain text stores none. Never change one: pages hold them.
    pub fn id(self) -> Option<&'static str> {
        Some(match self {
            CodeLanguage::PlainText => return None,
            CodeLanguage::Bash => "bash",
            CodeLanguage::C => "c",
            CodeLanguage::Cpp => "cpp",
            CodeLanguage::CSharp => "csharp",
            CodeLanguage::Css => "css",
            CodeLanguage::Diff => "diff",
            CodeLanguage::Go => "go",
            CodeLanguage::Html => "html",
            CodeLanguage::Java => "java",
            CodeLanguage::JavaScript => "javascript",
            CodeLanguage::Json => "json",
            CodeLanguage::Markdown => "markdown",
            CodeLanguage::Mermaid => "mermaid",
            CodeLanguage::Php => "php",
            CodeLanguage::Python => "python",
            CodeLanguage::Ruby => "ruby",
            CodeLanguage::Rust => "rust",
            CodeLanguage::Sql => "sql",
            CodeLanguage::TypeScript => "typescript",
            CodeLanguage::Xml => "xml",
            CodeLanguage::Yaml => "yaml",
        })
    }

    /// The language a stored id names; one this version does not know is plain text.
    pub fn from_id(id: Option<&str>) -> CodeLanguage {
        CodeLanguage::ALL
            .iter()
            .copied()
            .find(|language| language.id() == id)
            .unwrap_or(CodeLanguage::PlainText)
    }

    pub fn label(self) -> &'static str {
        match self {
            CodeLanguage::PlainText => "Plain text",
            CodeLanguage::Bash => "Bash",
            CodeLanguage::C => "C",
            CodeLanguage::Cpp => "C++",
            CodeLanguage::CSharp => "C#",
            CodeLanguage::Css => "CSS",
            CodeLanguage::Diff => "Diff",
            CodeLanguage::Go => "Go",
            CodeLanguage::Html => "HTML",
            CodeLanguage::Java => "Java",
            CodeLanguage::JavaScript => "JavaScript",
            CodeLanguage::Json => "JSON",
            CodeLanguage::Markdown => "Markdown",
            CodeLanguage::Mermaid => "Mermaid",
            CodeLanguage::Php => "PHP",
            CodeLanguage::Python => "Python",
            CodeLanguage::Ruby => "Ruby",
            CodeLanguage::Rust => "Rust",
            CodeLanguage::Sql => "SQL",
            CodeLanguage::TypeScript => "TypeScript",
            CodeLanguage::Xml => "XML",
            CodeLanguage::Yaml => "YAML",
        }
    }

    /// The grammar it is highlighted with, by its name in the bundled set. TypeScript uses
    /// JavaScript's, the closest the set has; the set has none for Mermaid.
    pub(super) fn grammar(self, grammars: &SyntaxSet) -> Option<&SyntaxReference> {
        let name = match self {
            CodeLanguage::PlainText | CodeLanguage::Mermaid => return None,
            CodeLanguage::Bash => "Bourne Again Shell (bash)",
            CodeLanguage::JavaScript | CodeLanguage::TypeScript => "JavaScript",
            language => language.label(),
        };
        grammars.find_syntax_by_name(name)
    }
}

impl PageEditor {
    pub(super) fn code_language(&self, block: BlockId) -> CodeLanguage {
        CodeLanguage::from_id(self.page.block_language(block).as_deref())
    }

    /// A code block's frame around its `text`, with its language at the top right.
    pub(super) fn render_code_block(
        &self,
        block: BlockId,
        text: Div,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::current(window, cx);
        let language = self.code_language(block);
        if language != CodeLanguage::PlainText {
            syntax::start_loading_grammars(cx);
        }
        if self.code_highlights.is_highlighting(block) {
            window.request_animation_frame();
        }
        let label = self.language_label(block, window, cx);
        code_frame(text, label, &theme)
    }

    /// The block's language at the top right of its frame; choosing it opens the language
    /// menu.
    pub(super) fn language_label(
        &self,
        block: BlockId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = Theme::current(window, cx);
        let language = self.code_language(block);
        div()
            .id(ElementId::Name(format!("code-language-{block}").into()))
            .px(theme.spacing.sm)
            .rounded(theme.radius.sm)
            .text_size(theme.text.caption.size)
            .line_height(theme.text.caption.line_height)
            .text_color(theme.colors.text_placeholder)
            // A click here picks a language; it must not also move the caret into the code.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(language.label())
            .when(!self.is_read_only(), |label| {
                Clickable::new(theme.radius.sm)
                    .hover(theme.colors.hover)
                    .on_click(cx.listener(move |editor, event: &ClickEvent, window, cx| {
                        editor.open_language_menu(block, event, window, cx)
                    }))
                    .apply(label, window, cx)
            })
            .into_any_element()
    }

    fn open_language_menu(
        &mut self,
        block: BlockId,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = self.code_language(block);
        let editor = cx.entity().downgrade();
        self.popups.update(cx, |popups, cx| {
            popups.show(
                PopupAnchor::Point(event.position()),
                |_, cx| {
                    Menu::new(cx).items(CodeLanguage::ALL.iter().map(|&language| {
                        let editor = editor.clone();
                        let item = MenuItem::new(language.label(), move |_, cx| {
                            let chosen = editor.update(cx, |editor, cx| {
                                editor.set_code_language(block, language, cx)
                            });
                            if let Err(error) = chosen {
                                log::debug!(
                                    "page closed before its code language was set: {error}"
                                );
                            }
                        });
                        if language == current {
                            item.checked()
                        } else {
                            item
                        }
                    }))
                },
                window,
                cx,
            );
        });
    }

    fn set_code_language(
        &mut self,
        block: BlockId,
        language: CodeLanguage,
        cx: &mut Context<Self>,
    ) {
        let selection = self.selection;
        if language == CodeLanguage::Mermaid {
            // The code being written stays in view as it becomes a diagram's source.
            self.diagram_being_edited = Some(block);
        }
        self.apply_edit(cx, move |page| {
            page.set_block_language(block, language.id())?;
            Ok(selection)
        });
    }
}

/// The code block's background around `text`, with `controls` at its top right.
pub(super) fn code_frame(text: Div, controls: impl IntoElement, theme: &Theme) -> AnyElement {
    div()
        .relative()
        .flex_1()
        .min_w_0()
        .p(theme.spacing.lg)
        .rounded(theme.radius.md)
        .bg(theme.colors.code_background)
        .child(text)
        .child(
            div()
                .absolute()
                .top(theme.spacing.xs)
                .right(theme.spacing.xs)
                .child(controls),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui::{Hsla, TestAppContext};

    use super::*;
    use crate::editor::document::{BlockKind, PageDocument};
    use crate::editor::{Selection, init};

    #[test]
    fn stored_ids_read_back_as_their_language() {
        for language in CodeLanguage::ALL {
            assert_eq!(CodeLanguage::from_id(language.id()), *language);
        }
        assert_eq!(
            CodeLanguage::from_id(Some("cobol")),
            CodeLanguage::PlainText
        );
    }

    #[gpui::test]
    fn a_code_block_with_a_language_paints_coloured_runs(cx: &mut TestAppContext) {
        cx.update(|cx| {
            scroll_ui::init(cx);
            init(cx);
        });
        syntax::load_grammars_now();
        let page = PageDocument::create(1, &["fn main() {}"]).unwrap();
        let block = page.ensure_block().unwrap();
        page.set_kind(block, BlockKind::Code).unwrap();
        let (editor, cx) =
            cx.add_window_view(|_, cx| PageEditor::new(page, Selection::caret(block, 0), cx));
        let paint_colours = |cx: &mut gpui::VisualTestContext| -> Vec<Hsla> {
            editor.update_in(cx, |editor, window, cx| {
                let theme = Theme::current(window, cx);
                let paint = editor.block_paint(0, &theme, false);
                paint.runs.iter().map(|run| run.color).collect()
            })
        };
        assert_eq!(paint_colours(cx).len(), 1, "plain text is one run");

        editor.update(cx, |editor, cx| {
            editor.set_code_language(block, CodeLanguage::Rust, cx)
        });
        let keyword = cx.update(|window, cx| Theme::current(window, cx).syntax.keyword);
        assert!(paint_colours(cx).contains(&keyword), "`fn` is a keyword");
    }
}
