//! Colours for code blocks. The grammars load in the background the first time a page shows a
//! code block with a language; until then, and for a language without a grammar, code is
//! plain. A page being opened builds the grammars of its first screen before it is shown.
//!
//! Highlighting is by line and kept per block. After an edit only the lines from the first
//! changed one are parsed again, and only until the parser reaches an unchanged line in the
//! state it was in before, so a keystroke costs about one line. A block shown for the first
//! time is highlighted from the top for at most `FRAME_BUDGET` per frame, so a long one fills
//! in over a few frames instead of holding up one.

use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;
use std::sync::LazyLock;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use gpui::{App, Context, Hsla, Task, TextRun};
use scroll_ui::theme::SyntaxColors;
use syntect::parsing::{
    ParseState, ParsingError, Scope, ScopeError, ScopeStack, SyntaxReference, SyntaxSet,
};
use syntect::util::LinesWithEndings;

use super::PageEditor;
use super::code_language::CodeLanguage;
use super::diagram_block::FIRST_SCREEN_BLOCKS;
use crate::editor::document::{BlockId, BlockKind};

/// How long one block may spend highlighting in one frame (a frame at 120 Hz is 8.3 ms).
const FRAME_BUDGET: Duration = Duration::from_millis(2);
/// Beyond this many bytes a block stays plain: a pasted log or data dump is not code to read.
const MAX_HIGHLIGHTED_LENGTH: usize = 256 * 1024;
/// A longer line (minified code, say) would take more than a frame to parse on its own, so it
/// and the lines after it stay plain.
const MAX_LINE_LENGTH: usize = 2 * 1024;

#[derive(Debug, thiserror::Error)]
enum HighlightError {
    #[error(transparent)]
    Parsing(#[from] ParsingError),
    #[error(transparent)]
    Scope(#[from] ScopeError),
}

static GRAMMARS: OnceLock<SyntaxSet> = OnceLock::new();
static IS_LOADING_GRAMMARS: AtomicBool = AtomicBool::new(false);

/// Starts reading the grammars in the background, once per run of the app; every window is
/// redrawn when they are in.
pub(super) fn start_loading_grammars(cx: &mut App) {
    if GRAMMARS.get().is_some() || IS_LOADING_GRAMMARS.swap(true, Ordering::Relaxed) {
        return;
    }
    let loading = cx.background_executor().spawn(async {
        GRAMMARS.get_or_init(SyntaxSet::load_defaults_newlines);
    });
    cx.spawn(async move |cx| {
        loading.await;
        if let Err(error) = cx.update(|cx| cx.refresh_windows()) {
            log::debug!("the app quit before the code grammars loaded: {error}");
        }
    })
    .detach();
}

/// Builds the grammars of `languages` in the background, reading the grammars first if need
/// be. A grammar builds its patterns as it parses its first line (any line), which would hold
/// up the frame doing it by 2–30 ms, once per language per run; parsing a line with a built
/// grammar is cheap. Built grammars take 5–15 MB each, so only those a page uses are built.
fn build_grammars(languages: Vec<CodeLanguage>, cx: &App) -> Task<()> {
    cx.background_executor().spawn(async move {
        let grammars = GRAMMARS.get_or_init(SyntaxSet::load_defaults_newlines);
        for language in languages {
            let Some(syntax) = language.grammar(grammars) else {
                continue;
            };
            if let Err(error) = LineStart::new(syntax).parse("x = 1\n", grammars) {
                log::warn!("could not build the {} grammar: {error}", syntax.name);
            }
        }
    })
}

impl PageEditor {
    /// Builds the grammars the code blocks on the page's first screen need, in the background,
    /// so its first frame shows them in colour without being held up.
    pub(crate) fn build_first_screen_grammars(&mut self, cx: &mut Context<Self>) {
        let mut languages: Vec<CodeLanguage> = Vec::new();
        for block in self.blocks.iter().take(FIRST_SCREEN_BLOCKS) {
            let language = self.code_language(block.info.id);
            let has_grammar = !matches!(language, CodeLanguage::PlainText | CodeLanguage::Mermaid);
            if block.info.kind == BlockKind::Code && has_grammar && !languages.contains(&language) {
                languages.push(language);
            }
        }
        if languages.is_empty() {
            return;
        }
        self.code_highlights.is_building_grammars = true;
        let building = build_grammars(languages, cx);
        cx.spawn(async move |editor, cx| {
            building.await;
            let built = editor.update(cx, |editor, cx| {
                editor.code_highlights.is_building_grammars = false;
                cx.notify();
            });
            if let Err(error) = built {
                log::debug!("page closed before its code grammars were built: {error}");
            }
        })
        .detach();
    }

    pub(crate) fn are_first_screen_grammars_built(&self) -> bool {
        !self.code_highlights.is_building_grammars
    }
}

/// Loads the grammars on this thread, for tests that need colours at once.
#[cfg(test)]
pub(super) fn load_grammars_now() -> &'static SyntaxSet {
    GRAMMARS.get_or_init(SyntaxSet::load_defaults_newlines)
}

/// What a stretch of code is, for colouring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyntaxToken {
    Keyword,
    String,
    Comment,
    Constant,
    Function,
    Type,
}

impl SyntaxToken {
    fn color(self, colors: &SyntaxColors) -> Hsla {
        match self {
            SyntaxToken::Keyword => colors.keyword,
            SyntaxToken::String => colors.string,
            SyntaxToken::Comment => colors.comment,
            SyntaxToken::Constant => colors.constant,
            SyntaxToken::Function => colors.function,
            SyntaxToken::Type => colors.type_name,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HighlightSpan {
    /// UTF-8 byte offsets into the block's text.
    pub range: Range<usize>,
    pub token: SyntaxToken,
}

/// Grammar scopes and what they are drawn as, most specific first. A scope matches any scope
/// it is a prefix of. `None` draws the scope as plain text and stops the search, so operators
/// are not coloured as keywords. Scopes not listed (punctuation, say) take their colour from
/// the scope around them, so a string's quotes are coloured with the string.
static SCOPE_TOKENS: LazyLock<Vec<(Scope, Option<SyntaxToken>)>> = LazyLock::new(|| {
    [
        ("comment", Some(SyntaxToken::Comment)),
        ("string", Some(SyntaxToken::String)),
        ("constant", Some(SyntaxToken::Constant)),
        ("variable.language", Some(SyntaxToken::Constant)),
        ("keyword.operator", None),
        ("keyword", Some(SyntaxToken::Keyword)),
        ("storage", Some(SyntaxToken::Keyword)),
        ("entity.name.function", Some(SyntaxToken::Function)),
        ("support.function", Some(SyntaxToken::Function)),
        ("variable.function", Some(SyntaxToken::Function)),
        ("entity.name", Some(SyntaxToken::Type)),
        ("support.type", Some(SyntaxToken::Type)),
        ("support.class", Some(SyntaxToken::Type)),
    ]
    .into_iter()
    .filter_map(|(name, token)| match Scope::new(name) {
        Ok(scope) => Some((scope, token)),
        Err(error) => {
            log::error!("bad highlight scope {name:?}: {error:?}");
            None
        }
    })
    .collect()
});

/// The token for text inside `stack`: the innermost scope that has one decides.
fn token_of(stack: &ScopeStack) -> Option<SyntaxToken> {
    stack.as_slice().iter().rev().find_map(|scope| {
        SCOPE_TOKENS
            .iter()
            .find(|(prefix, _)| prefix.is_prefix_of(*scope))
            .map(|(_, token)| *token)
    })?
}

/// The parser's state where a line starts.
#[derive(Clone, PartialEq)]
struct LineStart {
    parse: ParseState,
    scopes: ScopeStack,
}

impl LineStart {
    fn new(syntax: &SyntaxReference) -> Self {
        LineStart {
            parse: ParseState::new(syntax),
            scopes: ScopeStack::new(),
        }
    }

    /// Parses `line` (with its newline) from this state: its spans, relative to the line, and
    /// the state the next line starts in.
    fn parse(
        &self,
        line: &str,
        grammars: &SyntaxSet,
    ) -> Result<(Vec<HighlightSpan>, LineStart), HighlightError> {
        let mut next = self.clone();
        let operations = next.parse.parse_line(line, grammars)?;
        let mut spans: Vec<HighlightSpan> = Vec::new();
        let mut push = |range: Range<usize>, token: Option<SyntaxToken>| {
            let Some(token) = token.filter(|_| !range.is_empty()) else {
                return;
            };
            match spans.last_mut() {
                Some(last) if last.token == token && last.range.end == range.start => {
                    last.range.end = range.end;
                }
                _ => spans.push(HighlightSpan { range, token }),
            }
        };
        let mut position = 0;
        for (offset, operation) in operations {
            push(position..offset, token_of(&next.scopes));
            position = offset;
            next.scopes.apply(&operation)?;
        }
        push(position..line.len(), token_of(&next.scopes));
        Ok((spans, next))
    }
}

struct Line {
    text: String,
    start: LineStart,
    /// Relative to the line.
    spans: Vec<HighlightSpan>,
}

/// How much of a block has been highlighted.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Reach {
    Everything,
    /// The top part; the rest follows in later frames.
    InProgress,
    /// The top part; the rest stays plain (too long, or the grammar failed).
    Stopped,
}

/// What a block is highlighted with, and until when in this frame.
struct Parser<'a> {
    language: CodeLanguage,
    syntax: &'a SyntaxReference,
    grammars: &'a SyntaxSet,
    deadline: Instant,
}

/// One block's highlighting and what it was made from.
struct Highlighted {
    language: CodeLanguage,
    text: String,
    lines: Vec<Line>,
    /// Where the line after the last highlighted one starts.
    after_last: LineStart,
    reach: Reach,
    spans: Rc<[HighlightSpan]>,
}

impl Highlighted {
    /// Highlights `text` within `deadline`, keeping what it can of `previous`, the same block's
    /// highlighting in the same language before an edit (or a frame that ran out of time).
    fn new(previous: Option<Highlighted>, text: &str, parser: &Parser) -> Highlighted {
        let syntax = parser.syntax;
        let new_lines: Vec<&str> = LinesWithEndings::from(text).collect();
        let (mut lines, old_after_last, old_reach) = match previous {
            Some(previous) => (previous.lines, previous.after_last, previous.reach),
            None => (Vec::new(), LineStart::new(syntax), Reach::InProgress),
        };
        let prefix = lines
            .iter()
            .zip(&new_lines)
            .take_while(|(old, new)| old.text == **new)
            .count();
        // With every old line highlighted, those after the last change can keep their colours
        // once parsing reaches them in the state they started in before.
        let suffix = if old_reach == Reach::Everything {
            lines[prefix..]
                .iter()
                .rev()
                .zip(new_lines[prefix..].iter().rev())
                .take_while(|(old, new)| old.text == **new)
                .count()
        } else {
            0
        };
        let mut tail = lines.split_off(lines.len() - suffix);
        let mut state = match lines.get(prefix) {
            Some(line) => line.start.clone(),
            None if prefix == lines.len() => old_after_last.clone(),
            None => LineStart::new(syntax),
        };
        lines.truncate(prefix);
        let suffix_start = new_lines.len() - suffix;
        let mut reach = Reach::Everything;
        for (index, line) in new_lines.iter().enumerate().skip(prefix) {
            if index >= suffix_start {
                let reused = index - suffix_start;
                if tail[reused].start == state {
                    lines.extend(tail.drain(reused..));
                    state = old_after_last;
                    break;
                }
            }
            if line.len() > MAX_LINE_LENGTH {
                reach = Reach::Stopped;
                break;
            }
            // At least one line per frame, so a block always finishes.
            if index > prefix && Instant::now() >= parser.deadline {
                reach = Reach::InProgress;
                break;
            }
            match state.parse(line, parser.grammars) {
                Ok((spans, next)) => {
                    lines.push(Line {
                        text: line.to_string(),
                        start: state,
                        spans,
                    });
                    state = next;
                }
                Err(error) => {
                    log::warn!("could not highlight a {} block: {error}", syntax.name);
                    reach = Reach::Stopped;
                    break;
                }
            }
        }
        let spans = block_spans(&lines);
        Highlighted {
            language: parser.language,
            text: text.to_string(),
            lines,
            after_last: state,
            reach,
            spans,
        }
    }
}

/// The lines' spans, relative to the whole block.
fn block_spans(lines: &[Line]) -> Rc<[HighlightSpan]> {
    let mut line_start = 0;
    let mut spans = Vec::new();
    for line in lines {
        spans.extend(line.spans.iter().map(|span| HighlightSpan {
            range: line_start + span.range.start..line_start + span.range.end,
            token: span.token,
        }));
        line_start += line.text.len();
    }
    spans.into()
}

/// Each code block's highlighting, kept until its text or language changes.
#[derive(Default)]
pub(super) struct CodeHighlights {
    // Filled while painting, which only reads the editor.
    blocks: RefCell<HashMap<BlockId, Highlighted>>,
    /// While the grammars the first screen needs are built (`build_first_screen_grammars`).
    is_building_grammars: bool,
}

impl CodeHighlights {
    /// The spans for `block`'s `text` in `language`, highlighting what changed (or what is
    /// left) first. None while the grammars load, for plain text, or past the length
    /// highlighted.
    pub(super) fn spans(
        &self,
        block: BlockId,
        language: CodeLanguage,
        text: &str,
    ) -> Rc<[HighlightSpan]> {
        let mut blocks = self.blocks.borrow_mut();
        if let Some(cached) = blocks.get(&block)
            && cached.language == language
            && cached.text == text
            && cached.reach != Reach::InProgress
        {
            return cached.spans.clone();
        }
        let previous = blocks
            .remove(&block)
            .filter(|previous| previous.language == language);
        let Some(grammars) = GRAMMARS.get() else {
            return Rc::from([]);
        };
        let Some(syntax) = language.grammar(grammars) else {
            return Rc::from([]);
        };
        if text.len() > MAX_HIGHLIGHTED_LENGTH {
            return Rc::from([]);
        }
        let parser = Parser {
            language,
            syntax,
            grammars,
            deadline: Instant::now() + FRAME_BUDGET,
        };
        let highlighted = Highlighted::new(previous, text, &parser);
        let spans = highlighted.spans.clone();
        blocks.insert(block, highlighted);
        spans
    }

    /// Whether `block` still has lines to highlight in a later frame.
    pub(super) fn is_highlighting(&self, block: BlockId) -> bool {
        self.blocks
            .borrow()
            .get(&block)
            .is_some_and(|highlighted| highlighted.reach == Reach::InProgress)
    }
}

/// `runs` recoloured by `spans`: runs are split where a span starts or ends, and keep
/// everything but their colour, so the text measures and wraps exactly as it would plain.
pub(super) fn colour_runs(
    runs: Vec<TextRun>,
    spans: &[HighlightSpan],
    colors: &SyntaxColors,
) -> Vec<TextRun> {
    if spans.is_empty() {
        return runs;
    }
    let mut coloured = Vec::with_capacity(runs.len() + spans.len() * 2);
    let mut spans = spans.iter().peekable();
    let mut run_start = 0;
    for run in runs {
        let run_end = run_start + run.len;
        let mut position = run_start;
        while position < run_end {
            while spans.next_if(|span| span.range.end <= position).is_some() {}
            let (end, color) = match spans.peek() {
                Some(span) if span.range.start <= position => {
                    (span.range.end.min(run_end), span.token.color(colors))
                }
                Some(span) => (span.range.start.min(run_end), run.color),
                None => (run_end, run.color),
            };
            coloured.push(TextRun {
                len: end - position,
                color,
                ..run.clone()
            });
            position = end;
        }
        run_start = run_end;
    }
    coloured
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use gpui::{font, hsla};

    use super::*;

    fn grammars() -> &'static SyntaxSet {
        load_grammars_now()
    }

    /// A parser with all the time it needs.
    fn parser(language: CodeLanguage) -> Parser<'static> {
        Parser {
            language,
            syntax: language.grammar(grammars()).expect("a grammar"),
            grammars: grammars(),
            deadline: Instant::now() + Duration::from_secs(60),
        }
    }

    fn tokens(text: &str, language: CodeLanguage) -> Vec<(&str, SyntaxToken)> {
        Highlighted::new(None, text, &parser(language))
            .spans
            .iter()
            .map(|span| (&text[span.range.clone()], span.token))
            .collect()
    }

    #[test]
    fn every_language_but_plain_text_and_mermaid_has_a_grammar() {
        for language in CodeLanguage::ALL {
            let has_grammar = language.grammar(grammars()).is_some();
            let is_plain = matches!(language, CodeLanguage::PlainText | CodeLanguage::Mermaid);
            assert_eq!(has_grammar, !is_plain, "{language:?}");
        }
    }

    #[test]
    fn scopes_map_onto_the_themes_tokens() {
        let rust = tokens(
            "// note\nfn main() { let x: u32 = 42; say(\"hi\"); }\n",
            CodeLanguage::Rust,
        );
        assert!(rust.contains(&("// note\n", SyntaxToken::Comment)));
        assert!(rust.contains(&("fn", SyntaxToken::Keyword)));
        assert!(rust.contains(&("main", SyntaxToken::Function)));
        assert!(rust.contains(&("42", SyntaxToken::Constant)));
        assert!(rust.contains(&("\"hi\"", SyntaxToken::String)));
        assert!(
            !rust.iter().any(|(text, _)| text.contains('=')),
            "operators stay plain"
        );

        let python = tokens("def greet():\n    return True\n", CodeLanguage::Python);
        assert!(python.contains(&("def", SyntaxToken::Keyword)));
        assert!(python.contains(&("True", SyntaxToken::Constant)));
    }

    #[test]
    fn a_block_is_highlighted_again_only_when_its_text_or_language_changes() {
        grammars();
        let highlights = CodeHighlights::default();
        let block = BlockId::new(1, 1);
        let first = highlights.spans(block, CodeLanguage::Rust, "let x = 1;");
        let again = highlights.spans(block, CodeLanguage::Rust, "let x = 1;");
        assert!(
            Rc::ptr_eq(&first, &again),
            "unchanged text reuses its spans"
        );

        let edited = highlights.spans(block, CodeLanguage::Rust, "let x = 12;");
        assert!(!Rc::ptr_eq(&first, &edited));
        assert!(edited.iter().any(|span| span.range == (8..10)));

        let plain = highlights.spans(block, CodeLanguage::PlainText, "let x = 12;");
        assert!(plain.is_empty(), "plain text has no colours");
    }

    #[test]
    fn an_edit_reparses_from_the_changed_line_and_matches_a_fresh_highlight() {
        let before = "fn a() {}\nfn b() {}\nfn c() {}\n";
        let rust = parser(CodeLanguage::Rust);
        let previous = Highlighted::new(None, before, &rust);
        let kept_line = previous.lines[2].start.clone();

        // Opening a comment changes how every later line parses.
        let commented = "fn a() {}\n/* fn b() {}\nfn c() {}\n";
        let edited = Highlighted::new(Some(previous), commented, &rust);
        let fresh = Highlighted::new(None, commented, &rust);
        assert_eq!(edited.spans, fresh.spans);
        assert!(edited.lines[2].start != kept_line, "the change carried on");

        // Typing within one line leaves the others as they were.
        let typed = "fn a() {}\nfn bb() {}\nfn c() {}\n";
        let fresh_typed = Highlighted::new(None, typed, &rust);
        let edited = Highlighted::new(Some(fresh), typed, &rust);
        assert_eq!(edited.spans, fresh_typed.spans);
    }

    #[test]
    fn a_long_block_is_highlighted_over_several_frames() {
        let text = "let x = 1;\n".repeat(50_000);
        let rush = Parser {
            deadline: Instant::now(),
            ..parser(CodeLanguage::Rust)
        };
        let first_frame = Highlighted::new(None, &text, &rush);
        assert!(first_frame.reach == Reach::InProgress);
        assert_eq!(
            first_frame.lines.len(),
            1,
            "one line per frame at the least"
        );
        let second_frame = Highlighted::new(Some(first_frame), &text, &rush);
        assert_eq!(
            second_frame.lines.len(),
            2,
            "it carries on where it stopped"
        );
    }

    #[test]
    fn runs_are_split_and_recoloured_without_changing_their_length() {
        let colors = SyntaxColors {
            keyword: hsla(0.0, 1.0, 0.5, 1.0),
            string: hsla(0.3, 1.0, 0.5, 1.0),
            comment: hsla(0.5, 0.0, 0.5, 1.0),
            constant: hsla(0.6, 1.0, 0.5, 1.0),
            function: hsla(0.8, 1.0, 0.5, 1.0),
            type_name: hsla(0.1, 1.0, 0.5, 1.0),
        };
        let text_color = hsla(0.0, 0.0, 0.1, 1.0);
        let run = |len| TextRun {
            len,
            font: font("Menlo"),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        // "let x = 1;" split by an IME underline at 4..5.
        let runs = vec![run(4), run(1), run(5)];
        let spans = [
            HighlightSpan {
                range: 0..3,
                token: SyntaxToken::Keyword,
            },
            HighlightSpan {
                range: 8..9,
                token: SyntaxToken::Constant,
            },
        ];
        let coloured = colour_runs(runs, &spans, &colors);
        let shape: Vec<(usize, Hsla)> = coloured.iter().map(|run| (run.len, run.color)).collect();
        assert_eq!(
            shape,
            vec![
                (3, colors.keyword),
                (1, text_color),
                (1, text_color),
                (3, text_color),
                (1, colors.constant),
                (1, text_color),
            ]
        );
    }

    /// The costs that decide `FRAME_BUDGET` and how a keystroke feels, in a release build:
    /// `cargo test --release -p scroll-app highlighting_costs -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn highlighting_costs() {
        let started = Instant::now();
        grammars();
        println!("loading the grammars: {:?}", started.elapsed());
        let line = "    let total: u64 = values.iter().map(|v| v * 2).sum(); // \"sum\"\n";
        let text = line.repeat(16 * 1024 / line.len());
        let rust = parser(CodeLanguage::Rust);
        let median = |mut timings: Vec<Duration>| {
            timings.sort();
            timings[timings.len() / 2]
        };
        let whole = median(
            (0..10)
                .map(|_| {
                    let started = Instant::now();
                    Highlighted::new(None, &text, &rust);
                    started.elapsed()
                })
                .collect(),
        );
        println!("highlighting {} bytes from scratch: {whole:?}", text.len());
        let middle = text.len() / 2 / line.len() * line.len() + 8;
        let mut typed = text.clone();
        typed.insert(middle, 'x');
        let keystroke = median(
            (0..10)
                .map(|_| {
                    let previous = Highlighted::new(None, &text, &rust);
                    let started = Instant::now();
                    Highlighted::new(Some(previous), &typed, &rust);
                    started.elapsed()
                })
                .collect(),
        );
        println!("a keystroke in the middle of it: {keystroke:?}");
    }
}
