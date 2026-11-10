use gpui::{
    App, Hsla, IntoElement, ParentElement, Pixels, Radians, RenderOnce, Styled, Transformation,
    Window, div, prelude::FluentBuilder, svg,
};

use crate::foundation::story::stateless_story;
use crate::theme::Theme;
use gpui::{AnyElement, AnyView};

/// Declares every icon once: its variant and its file in `assets/icons/` (without `.svg`).
/// From that come `IconName`, `IconName::ALL`, the asset path and the embedded source.
macro_rules! icons {
    ($($name:ident => $file:literal,)+) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum IconName {
            $($name,)+
        }

        impl IconName {
            pub const ALL: &[IconName] = &[$(IconName::$name,)+];

            pub fn path(self) -> &'static str {
                match self {
                    $(IconName::$name => concat!("icons/", $file, ".svg"),)+
                }
            }

            pub(crate) fn svg_source(self) -> &'static str {
                match self {
                    $(IconName::$name => include_str!(concat!("../../assets/icons/", $file, ".svg")),)+
                }
            }
        }
    };
}

icons! {
    Archive => "archive",
    Calendar => "calendar",
    ArrowLeft => "arrow_left",
    ArrowLeftRight => "arrow_left_right",
    ArrowRight => "arrow_right",
    Callout => "callout",
    Check => "check",
    CheckSquare => "check_square",
    ChevronDown => "chevron_down",
    ChevronRight => "chevron_right",
    Close => "close",
    Code => "code",
    Diagram => "diagram",
    Ellipsis => "ellipsis",
    Grip => "grip",
    Hash => "hash",
    Heading => "heading",
    Link => "link",
    List => "list",
    ListNumbered => "list_numbered",
    Minus => "minus",
    Moon => "moon",
    Page => "page",
    Pencil => "pencil",
    Plus => "plus",
    Quote => "quote",
    Restore => "restore",
    Search => "search",
    Select => "select",
    Sidebar => "sidebar",
    Sun => "sun",
    Table => "table",
    Text => "text",
    Trash => "trash",
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IconSize {
    XSmall,
    Small,
    #[default]
    Medium,
    Large,
}

impl IconSize {
    pub(crate) fn pixels(self, theme: &Theme) -> Pixels {
        match self {
            IconSize::XSmall => theme.size.icon_xsmall,
            IconSize::Small => theme.size.icon_small,
            IconSize::Medium => theme.size.icon_medium,
            IconSize::Large => theme.size.icon_large,
        }
    }
}

#[derive(IntoElement)]
pub struct Icon {
    name: IconName,
    size: IconSize,
    color: Option<Hsla>,
    rotation: Option<Radians>,
}

impl Icon {
    pub fn new(name: IconName) -> Self {
        Icon {
            name,
            size: IconSize::default(),
            color: None,
            rotation: None,
        }
    }

    pub fn size(mut self, size: IconSize) -> Self {
        self.size = size;
        self
    }

    /// Defaults to the theme's icon colour.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    /// Turns the icon clockwise about its centre, e.g. a chevron opening.
    pub fn rotate(mut self, angle: Radians) -> Self {
        self.rotation = Some(angle);
        self
    }
}

impl RenderOnce for Icon {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let size = self.size.pixels(&theme);
        svg()
            .path(self.name.path())
            .flex_none()
            .size(size)
            .text_color(self.color.unwrap_or(theme.colors.icon))
            .when_some(self.rotation, |icon, angle| {
                icon.with_transformation(Transformation::rotate(angle))
            })
    }
}

impl Icon {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        stateless_story(render_story, cx)
    }
}

fn render_story(window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = Theme::current(window, cx);
    div()
        .flex()
        .flex_wrap()
        .gap(theme.spacing.md)
        .children(
            IconName::ALL
                .iter()
                .map(|name| Icon::new(*name).size(IconSize::Medium)),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[gpui::test]
    fn the_story_renders(cx: &mut gpui::TestAppContext) {
        crate::foundation::story::testing::open_story(Icon::story, cx);
    }
}
