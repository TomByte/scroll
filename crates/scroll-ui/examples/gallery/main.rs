//! Every scroll-ui component's story, in a light window and a dark window side by side.
//!
//! `cargo run -p scroll-ui --example gallery`
//! Pass `--keyboard-focus` to start with keyboard focus on the first control, to review focus
//! rings, and `--from <Name>` to start the gallery at that component.

use gpui::{
    AnyView, App, AppContext, Application, Bounds, Context, FocusHandle, FontWeight,
    InteractiveElement, IntoElement, KeyBinding, ParentElement, Render, StatefulInteractiveElement,
    Styled, Window, WindowBounds, WindowOptions, actions, div, point, px, size,
};
use scroll_ui::actions::FocusNext;
use scroll_ui::theme::force_window_appearance;
use scroll_ui::*;

/// One line per component: its gallery heading and its story.
const STORIES: &[(&str, StoryBuilder)] = &[
    ("Button", Button::story),
    ("IconButton", IconButton::story),
    ("Checkbox", Checkbox::story),
    ("SegmentedControl", SegmentedControl::story),
    ("TextInput", TextInput::story),
    ("FormField", FormField::story),
    ("FieldError", FieldError::story),
    ("Combobox", Combobox::story),
    ("ListItem", ListItem::story),
    ("TreeItem", TreeItem::story),
    ("SectionHeader", SectionHeader::story),
    ("EmptyState", EmptyState::story),
    ("Tabs", Tabs::story),
    ("Calendar", Calendar::story),
    ("Scrollbar", Scrollbar::story),
    ("Avatar", Avatar::story),
    ("Pill", Pill::story),
    ("PillRow", PillRow::story),
    ("Icon", Icon::story),
    ("Divider", Divider::story),
    ("DropIndicator", DropIndicator::story),
    ("DragPreview", DragPreview::story),
    ("ResizeHandle", ResizeHandle::story),
    ("TitlebarDragArea", TitlebarDragArea::story),
    ("Popover", Popover::story),
    ("PopupHost", PopupHost::story),
    ("Menu", Menu::story),
    ("Tooltip", Tooltip::story),
    ("ErrorBubble", ErrorBubble::story),
    ("Toast", Toast::story),
    ("ToastHost", ToastHost::story),
    ("Modal", Modal::story),
    ("Dialog", Dialog::story),
    ("ModalLayer", ModalLayer::story),
    ("EmojiPicker", EmojiPicker::story),
];

actions!(gallery, [Quit]);

const WINDOW_SIZE: (f32, f32) = (1250.0, 1080.0);
const WINDOW_GAP: f32 = 16.0;
const WINDOW_MARGIN: f32 = 24.0;
const STORY_WIDTH: f32 = 380.0;

/// Where focus starts in each gallery window.
#[derive(Clone, Copy)]
enum StartFocus {
    Window,
    FirstControl,
}

fn main() {
    let arguments: Vec<String> = std::env::args().collect();
    let first_story = arguments
        .iter()
        .position(|argument| argument == "--from")
        .and_then(|index| arguments.get(index + 1))
        .and_then(|name| STORIES.iter().position(|(story, _)| story == name))
        .unwrap_or(0);
    let start_focus = if std::env::args().any(|argument| argument == "--keyboard-focus") {
        StartFocus::FirstControl
    } else {
        StartFocus::Window
    };
    Application::new()
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            scroll_ui::init(cx);
            cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
            cx.on_action(|_: &Quit, cx| cx.quit());
            for (index, appearance) in [Appearance::Light, Appearance::Dark]
                .into_iter()
                .enumerate()
            {
                let start = Start {
                    focus: start_focus,
                    first_story,
                };
                open_gallery_window(appearance, index, start, cx);
            }
            cx.activate(true);
        });
}

/// How each gallery window starts, from the command line.
#[derive(Clone, Copy)]
struct Start {
    focus: StartFocus,
    first_story: usize,
}

fn open_gallery_window(appearance: Appearance, index: usize, start: Start, cx: &mut App) {
    let (width, height) = WINDOW_SIZE;
    let origin = point(
        px(WINDOW_MARGIN + index as f32 * (width + WINDOW_GAP)),
        px(WINDOW_MARGIN),
    );
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            origin,
            size(px(width), px(height)),
        ))),
        ..WindowOptions::default()
    };
    let opened = cx.open_window(options, |window, cx| {
        force_window_appearance(appearance, window, cx);
        let gallery = cx.new(|cx| Gallery::new(appearance, start.first_story, window, cx));
        if let StartFocus::FirstControl = start.focus {
            window.on_next_frame(|window, cx| window.dispatch_action(Box::new(FocusNext), cx));
        }
        gallery
    });
    if let Err(error) = opened {
        eprintln!("could not open gallery window: {error:#}");
        cx.quit();
    }
}

struct Gallery {
    appearance: Appearance,
    stories: Vec<(&'static str, AnyView)>,
    focus_handle: FocusHandle,
}

impl Gallery {
    fn new(
        appearance: Appearance,
        first_story: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let stories = STORIES[first_story..]
            .iter()
            .map(|(name, story)| (*name, story(window, cx)))
            .collect();
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle);
        Gallery {
            appearance,
            stories,
            focus_handle,
        }
    }
}

impl Render for Gallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let title = match self.appearance {
            Appearance::Light => "scroll-ui · light",
            Appearance::Dark => "scroll-ui · dark",
        };
        let stories = self.stories.iter().map(|(name, story)| {
            div()
                .flex()
                .flex_col()
                .min_w(px(STORY_WIDTH))
                .child(SectionHeader::new(*name))
                .child(div().px(theme.spacing.sm).child(story.clone()))
        });
        tab_navigation(div())
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .bg(theme.colors.surface)
            .text_color(theme.colors.text)
            .font_family(theme.font.ui.clone())
            .text_size(theme.text.ui.size)
            .child(
                div()
                    .id("gallery-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .p(theme.spacing.xxl)
                    .flex()
                    .flex_col()
                    .gap(theme.spacing.xl)
                    .child(
                        div()
                            .flex()
                            .justify_between()
                            .items_baseline()
                            .child(
                                div()
                                    .text_size(theme.text.heading_2.size)
                                    .line_height(theme.text.heading_2.line_height)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_size(theme.text.small.size)
                                    .text_color(theme.colors.text_placeholder)
                                    .child("Tab moves focus · rings show for keyboard focus only"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_x(theme.spacing.xl)
                            .gap_y(theme.spacing.xxl)
                            .children(stories),
                    ),
            )
    }
}
