//! A page's emoji above its title: click it to pick another or remove it; a page without one
//! offers "Add icon".

use gpui::{
    AppContext, ClickEvent, Context, Entity, InteractiveElement, IntoElement, ParentElement,
    Render, Styled, Subscription, Window, div,
};
use scroll_core::id::PageId;
use scroll_ui::{
    Button, ButtonSize, ButtonVariant, Clickable, EmojiPicker, EmojiPickerEvent, PopupAnchor,
    PopupHost, Theme,
};

use crate::workspace::{Workspace, WorkspaceEvent};

/// The hover group of the area above a page's content that reveals "Add icon".
pub const PAGE_HEADER_GROUP: &str = "page-header";

pub struct PageIcon {
    page: PageId,
    workspace: Entity<Workspace>,
    popups: Entity<PopupHost>,
    picker_subscription: Option<Subscription>,
    _subscription: Subscription,
}

/// The popup trigger the picker opens under, from the keyboard or the palette.
const ICON_TRIGGER: &str = "page-icon";

impl PageIcon {
    pub fn new(page: PageId, workspace: Entity<Workspace>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&workspace, |_, _, event: &WorkspaceEvent, cx| {
            if matches!(event, WorkspaceEvent::RecordsChanged(_)) {
                cx.notify();
            }
        });
        PageIcon {
            page,
            workspace,
            popups: cx.new(|_| PopupHost::new()),
            picker_subscription: None,
            _subscription: subscription,
        }
    }

    fn icon(&self, cx: &gpui::App) -> Option<String> {
        self.workspace.read(cx).tree().page(self.page)?.icon.clone()
    }

    /// Opens the emoji picker under the icon, for the keyboard and the palette.
    pub fn open_picker_below_icon(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_picker(PopupAnchor::Below(ICON_TRIGGER.into()), window, cx);
    }

    fn open_picker(&mut self, anchor: PopupAnchor, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspace.read(cx).is_read_only() {
            return;
        }
        let has_icon = self.icon(cx).is_some();
        let picker = self.popups.update(cx, |popups, cx| {
            popups.show(
                anchor,
                |_, cx| {
                    let picker = EmojiPicker::new(cx);
                    if has_icon { picker.removable() } else { picker }
                },
                window,
                cx,
            )
        });
        self.picker_subscription = Some(cx.subscribe(
            &picker,
            |this, _, event: &EmojiPickerEvent, cx| {
                let icon = match event {
                    EmojiPickerEvent::Picked(emoji) => Some(emoji.to_string()),
                    EmojiPickerEvent::Removed => None,
                };
                let page = this.page;
                this.workspace
                    .update(cx, |workspace, cx| workspace.set_icon(page, icon, cx));
            },
        ));
    }
}

impl Render for PageIcon {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let trigger = match self.icon(cx) {
            Some(icon) => {
                let font_size = theme.text.title.size * ICON_SCALE;
                let image_size = font_size * EMOJI_IMAGE_SCALE;
                let icon = div()
                    .id("page-icon")
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(image_size + theme.spacing.sm * 2.0)
                    .rounded(theme.radius.md)
                    .child(
                        div()
                            .relative()
                            .left(-(font_size * EMOJI_IMAGE_OFFSET))
                            .text_size(font_size)
                            .line_height(image_size)
                            .child(icon),
                    );
                Clickable::new(theme.radius.md)
                    .hover(theme.colors.hover)
                    .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                        this.open_picker(PopupAnchor::Point(event.position()), window, cx)
                    }))
                    .apply(icon, window, cx)
                    .into_any_element()
            }
            None if self.workspace.read(cx).is_read_only() => div().into_any_element(),
            // Shown while the pointer is over the page header, like a block's drag handle.
            None => div()
                .invisible()
                .group_hover(PAGE_HEADER_GROUP, |button| button.visible())
                .child(
                    Button::new("add-icon", "Add icon")
                        .size(ButtonSize::Small)
                        .variant(ButtonVariant::Ghost)
                        .on_click(cx.listener(|this, event: &ClickEvent, window, cx| {
                            this.open_picker(PopupAnchor::Point(event.position()), window, cx)
                        })),
                )
                .into_any_element(),
        };
        div()
            .flex()
            .child(
                div()
                    .relative()
                    .child(self.popups.read(cx).trigger(ICON_TRIGGER))
                    .child(trigger),
            )
            .child(self.popups.clone())
    }
}

/// A page's emoji is half as big again as its title.
const ICON_SCALE: f32 = 1.5;
/// Apple Color Emoji, as measured through GPUI's text system at 48px: a glyph's image is
/// 1.25em square, wider than the glyph's 1.08em advance, and starts 0.07em right of the pen.
/// A box sized and centred by the advance therefore shows the image 0.15em right of centre
/// and overhanging the box's right edge. The image is centred vertically in its line box
/// whatever the line height, so a line as tall as the image fits it exactly.
const EMOJI_IMAGE_SCALE: f32 = 1.25;
const EMOJI_IMAGE_OFFSET: f32 = 0.15;
