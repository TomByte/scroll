use gpui::{
    AnyElement, AnyView, App, ClickEvent, ElementId, FontWeight, Hsla, InteractiveElement,
    IntoElement, ParentElement, Pixels, RenderOnce, SharedString, Styled, Window, div,
    prelude::FluentBuilder,
};

use crate::ClickHandler;
use crate::display::icon::{Icon, IconName, IconSize};
use crate::foundation::focus::Clickable;
use crate::foundation::story::stateless_story;
use crate::theme::{TextSize, Theme};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonVariant {
    Primary,
    #[default]
    Secondary,
    Ghost,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ButtonSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl ButtonSize {
    pub(crate) fn height(self, theme: &Theme) -> Pixels {
        match self {
            ButtonSize::Small => theme.size.control_small,
            ButtonSize::Medium => theme.size.control_medium,
            ButtonSize::Large => theme.size.control_large,
        }
    }

    fn horizontal_padding(self, theme: &Theme) -> Pixels {
        match self {
            ButtonSize::Small => theme.spacing.md,
            ButtonSize::Medium => theme.spacing.lg,
            ButtonSize::Large => theme.spacing.xl,
        }
    }

    fn text(self, theme: &Theme) -> TextSize {
        match self {
            ButtonSize::Small => theme.text.small,
            ButtonSize::Medium | ButtonSize::Large => theme.text.ui,
        }
    }
}

/// Fill and text colours for one variant in each interaction state.
pub(crate) struct VariantColors {
    pub background: Hsla,
    pub hover: Hsla,
    pub pressed: Hsla,
    pub text: Hsla,
}

impl ButtonVariant {
    pub(crate) fn colors(self, theme: &Theme) -> VariantColors {
        let colors = &theme.colors;
        match self {
            ButtonVariant::Primary => VariantColors {
                background: colors.primary,
                hover: colors.primary_hover,
                pressed: colors.primary_pressed,
                text: colors.text_on_primary,
            },
            ButtonVariant::Secondary => VariantColors {
                background: colors.control,
                hover: colors.control_hover,
                pressed: colors.control_pressed,
                text: colors.text,
            },
            ButtonVariant::Ghost => VariantColors {
                background: gpui::transparent_black(),
                hover: colors.hover,
                pressed: colors.pressed,
                text: colors.text,
            },
        }
    }
}

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    variant: ButtonVariant,
    size: ButtonSize,
    icon: Option<IconName>,
    badge: Option<usize>,
    is_disabled: bool,
    on_click: Option<ClickHandler>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Button {
            id: id.into(),
            label: label.into(),
            variant: ButtonVariant::default(),
            size: ButtonSize::default(),
            icon: None,
            badge: None,
            is_disabled: false,
            on_click: None,
        }
    }

    /// A count shown after the label, e.g. how many filters apply; nothing for 0.
    pub fn badge(mut self, count: usize) -> Self {
        self.badge = (count > 0).then_some(count);
        self
    }

    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    /// An icon shown before the label.
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn disabled(mut self) -> Self {
        self.is_disabled = true;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let colors = self.variant.colors(&theme);
        let text = self.size.text(&theme);
        let icon_color = match self.variant {
            ButtonVariant::Primary => colors.text,
            ButtonVariant::Secondary | ButtonVariant::Ghost => theme.colors.icon,
        };

        let id = self.id;
        div()
            .id(id.clone())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(theme.spacing.sm)
            .h(self.size.height(&theme))
            .px(self.size.horizontal_padding(&theme))
            .rounded(theme.radius.md)
            .bg(colors.background)
            .text_color(colors.text)
            .text_size(text.size)
            .line_height(text.line_height)
            .font_weight(FontWeight::MEDIUM)
            .whitespace_nowrap()
            .when(self.variant == ButtonVariant::Secondary, |button| {
                button
                    .border(theme.size.border_width)
                    .border_color(theme.colors.border_strong)
                    .shadow(theme.shadow.raised.clone())
            })
            .when_some(self.icon, |button, icon| {
                button.child(Icon::new(icon).size(IconSize::Small).color(icon_color))
            })
            .child(self.label)
            .when_some(self.badge, |button, count| {
                button.child(
                    div()
                        .px(theme.spacing.xs)
                        .rounded(theme.radius.sm)
                        .bg(theme.colors.selected)
                        .text_size(theme.text.caption.size)
                        .line_height(theme.text.caption.line_height)
                        .child(count.to_string()),
                )
            })
            .map(|button| {
                Clickable::new(theme.radius.md)
                    .hover(colors.hover)
                    .pressed(colors.pressed)
                    .when(self.is_disabled, Clickable::disabled)
                    .when_some(self.on_click, Clickable::on_click)
                    .apply(button, window, cx)
            })
    }
}

impl Button {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        stateless_story(render_story, cx)
    }
}

fn render_story(window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = Theme::current(window, cx);
    let variants = [
        (ButtonVariant::Primary, "Primary"),
        (ButtonVariant::Secondary, "Secondary"),
        (ButtonVariant::Ghost, "Ghost"),
    ];
    let sizes = [ButtonSize::Small, ButtonSize::Medium, ButtonSize::Large];
    let rows = variants.into_iter().map(|(variant, label)| {
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(theme.spacing.md)
            .children(sizes.into_iter().enumerate().map(|(index, size)| {
                Button::new(SharedString::from(format!("{label}-{index}")), label)
                    .variant(variant)
                    .size(size)
            }))
            .child(
                Button::new(SharedString::from(format!("{label}-icon")), "New page")
                    .variant(variant)
                    .icon(IconName::Plus),
            )
            .child(
                Button::new(SharedString::from(format!("{label}-badge")), "Filter")
                    .variant(variant)
                    .badge(2),
            )
            .child(
                Button::new(SharedString::from(format!("{label}-disabled")), "Disabled")
                    .variant(variant)
                    .disabled(),
            )
    });
    div()
        .flex()
        .flex_col()
        .gap(theme.spacing.md)
        .children(rows)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{
        AppContext, Context, Entity, FocusHandle, Focusable, IntoElement, KeyUpEvent, Keystroke,
        ParentElement, Render, TestAppContext, VisualTestContext, Window, div,
    };

    use super::Button;
    use crate::controls::text_input::TextInput;
    use crate::foundation::focus::{is_keyboard_focus_visible, tab_navigation};

    /// A field followed by two buttons, as in the sign-in dialog.
    struct Form {
        field: Entity<TextInput>,
        presses: Rc<RefCell<Vec<&'static str>>>,
    }

    impl Render for Form {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let button = |id: &'static str| {
                let presses = self.presses.clone();
                Button::new(id, id).on_click(move |_, _, _| presses.borrow_mut().push(id))
            };
            tab_navigation(div())
                .child(self.field.clone())
                .child(button("first"))
                .child(button("second"))
        }
    }

    fn open_form(
        cx: &mut TestAppContext,
    ) -> (
        FocusHandle,
        Rc<RefCell<Vec<&'static str>>>,
        &mut VisualTestContext,
    ) {
        cx.update(crate::init);
        let presses = Rc::new(RefCell::new(Vec::new()));
        let form_presses = presses.clone();
        let (form, cx) = cx.add_window_view(|_, cx| Form {
            field: cx.new(TextInput::new),
            presses: form_presses,
        });
        let field = form.read_with(cx, |form, cx| form.field.focus_handle(cx));
        cx.update(|window, _| window.focus(&field));
        cx.run_until_parked();
        (field, presses, cx)
    }

    /// Presses and releases `key`: GPUI clicks a focused element on key up, which
    /// `simulate_keystrokes` alone does not send.
    fn press(key: &str, cx: &mut VisualTestContext) {
        cx.simulate_keystrokes(key);
        let keystroke = Keystroke::parse(key).expect("test keys are valid keystrokes");
        cx.simulate_event(KeyUpEvent { keystroke });
    }

    #[gpui::test]
    fn tab_reaches_each_button_and_space_or_return_presses_it(cx: &mut TestAppContext) {
        let (field, presses, cx) = open_form(cx);

        cx.simulate_keystrokes("tab");
        press("space", cx);
        assert_eq!(*presses.borrow(), ["first"]);
        assert!(
            cx.update(|_, cx| is_keyboard_focus_visible(cx)),
            "the focused button shows its ring after Tab"
        );

        cx.simulate_keystrokes("tab");
        press("enter", cx);
        assert_eq!(*presses.borrow(), ["first", "second"]);

        cx.simulate_keystrokes("shift-tab shift-tab");
        assert!(cx.update(|window, _| field.is_focused(window)));
        assert_eq!(presses.borrow().len(), 2, "moving focus presses nothing");
    }

    #[test]
    fn a_zero_count_shows_no_badge() {
        assert_eq!(Button::new("filter", "Filter").badge(0).badge, None);
        assert_eq!(Button::new("filter", "Filter").badge(3).badge, Some(3));
    }

    #[gpui::test]
    fn the_story_renders(cx: &mut TestAppContext) {
        crate::foundation::story::testing::open_story(Button::story, cx);
    }
}
