//! Edits the view's filter: one row per condition, each a property, a kind and an argument.
//! Arguments apply as you type, after a short pause, and at once on Enter.

use std::time::Duration;

use gpui::{
    App, AppContext, Context, DismissEvent, ElementId, Entity, EventEmitter, FocusHandle,
    Focusable, InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Task, Window, div, prelude::FluentBuilder,
};
use scroll_core::record::{FilterCondition, Property};
use scroll_ui::{
    ButtonSize, EmptyState, FieldError, Icon, IconButton, IconName, IconSize, ListItem,
    SectionHeader, SegmentedControl, TextInput, TextInputEvent, Theme,
};

use crate::conditions::{ConditionKind, argument_text};
use crate::values::type_icon;

#[derive(Debug, Clone, PartialEq)]
pub enum FilterEditorEvent {
    /// The conditions that are complete and valid; rows still being filled in are left out.
    Changed(Vec<FilterCondition>),
}

struct FilterRow {
    property: Property,
    kind: ConditionKind,
    argument: Entity<TextInput>,
    error: Option<SharedString>,
    _argument_subscription: Subscription,
}

/// How long typing in a filter value must pause before the filter applies.
const TYPING_PAUSE: Duration = Duration::from_millis(300);

pub struct FilterEditor {
    rows: Vec<FilterRow>,
    pending_apply: Option<Task<()>>,
    properties: Vec<Property>,
    focus_handle: FocusHandle,
}

impl EventEmitter<FilterEditorEvent> for FilterEditor {}
impl EventEmitter<DismissEvent> for FilterEditor {}

impl Focusable for FilterEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl FilterEditor {
    pub fn new(
        filter: &[FilterCondition],
        properties: Vec<Property>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut editor = FilterEditor {
            rows: Vec::new(),
            pending_apply: None,
            properties,
            focus_handle: cx.focus_handle(),
        };
        for condition in filter {
            let Some(property) = editor.property(condition) else {
                continue;
            };
            let argument = argument_text(&condition.condition, &property);
            editor.push_row(
                property,
                ConditionKind::of(&condition.condition),
                &argument,
                cx,
            );
        }
        editor
    }

    fn property(&self, condition: &FilterCondition) -> Option<Property> {
        self.properties
            .iter()
            .find(|property| property.id == condition.property)
            .cloned()
    }

    fn push_row(
        &mut self,
        property: Property,
        kind: ConditionKind,
        argument: &str,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx).placeholder("Value");
            input.set_text(argument, cx);
            input
        });
        let subscription = cx.subscribe(&input, |editor, _, event: &TextInputEvent, cx| {
            match event {
                TextInputEvent::Submitted => editor.apply(cx),
                TextInputEvent::Changed => editor.apply_after_pause(cx),
                // Escape also reaches the popover, which dismisses the editor.
                TextInputEvent::Cancelled => {}
            }
        });
        self.rows.push(FilterRow {
            property,
            kind,
            argument: input,
            error: None,
            _argument_subscription: subscription,
        });
    }

    /// Applies once typing pauses; each keystroke restarts the wait.
    fn apply_after_pause(&mut self, cx: &mut Context<Self>) {
        self.pending_apply = Some(cx.spawn(async move |editor, cx| {
            cx.background_executor().timer(TYPING_PAUSE).await;
            // The editor may have been closed while waiting.
            let _ = editor.update(cx, |editor, cx| editor.apply(cx));
        }));
    }

    /// Builds every complete row; invalid ones show their error and are not applied.
    fn apply(&mut self, cx: &mut Context<Self>) {
        self.pending_apply = None;
        let mut conditions = Vec::new();
        for row in &mut self.rows {
            let argument = row.argument.read(cx).text().to_string();
            row.error = None;
            if row.kind.takes_argument() && argument.trim().is_empty() {
                continue;
            }
            match row.kind.build(&argument, &row.property) {
                Ok(condition) => conditions.push(FilterCondition {
                    property: row.property.id,
                    condition,
                }),
                Err(error) => row.error = Some(error.to_string().into()),
            }
        }
        cx.emit(FilterEditorEvent::Changed(conditions));
        cx.notify();
    }

    fn add_row(&mut self, property: Property, window: &mut Window, cx: &mut Context<Self>) {
        let kind = ConditionKind::for_type(property.property_type)[0];
        self.push_row(property, kind, "", cx);
        if let Some(row) = self.rows.last() {
            window.focus(&row.argument.focus_handle(cx));
        }
        self.apply(cx);
    }

    fn set_kind(&mut self, index: usize, kind: ConditionKind, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.get_mut(index) {
            row.kind = kind;
        }
        self.apply(cx);
    }

    fn remove_row(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.rows.len() {
            self.rows.remove(index);
        }
        self.apply(cx);
    }

    fn render_row(&self, index: usize, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement {
        let row = &self.rows[index];
        let kinds = ConditionKind::for_type(row.property.property_type);
        let selected_kind = kinds.iter().position(|kind| *kind == row.kind).unwrap_or(0);
        let kind_choice = kinds
            .iter()
            .fold(
                SegmentedControl::new(ElementId::Name(format!("kinds-{index}").into())),
                |control, kind| control.option(kind.label()),
            )
            .selected(selected_kind)
            .size(ButtonSize::Small)
            .on_select(cx.listener(move |editor, choice: &usize, _, cx| {
                editor.set_kind(index, kinds[*choice], cx)
            }));
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.xs)
            .px(theme.spacing.sm)
            .when(index > 0, |row_element| {
                // A line between conditions, so each reads as one unit.
                row_element
                    .mt(theme.spacing.xs)
                    .pt(theme.spacing.sm)
                    .border_t(theme.size.border_width)
                    .border_color(theme.colors.border)
            })
            .py(theme.spacing.xs)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(theme.spacing.sm)
                    .child(Icon::new(type_icon(row.property.property_type)).size(IconSize::Small))
                    .child(div().flex_1().truncate().child(row.property.name.clone()))
                    .child(
                        IconButton::new(
                            ElementId::Name(format!("remove-filter-{index}").into()),
                            IconName::Close,
                        )
                        .size(ButtonSize::Small)
                        .tooltip("Remove filter")
                        .on_click(
                            cx.listener(move |editor, _, _, cx| editor.remove_row(index, cx)),
                        ),
                    ),
            )
            .child(kind_choice)
            .when(row.kind.takes_argument(), |row_element| {
                row_element.child(row.argument.clone())
            })
            .when_some(row.error.clone(), |row_element, error| {
                row_element.child(FieldError::new(error))
            })
    }
}

impl Render for FilterEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let rows: Vec<_> = (0..self.rows.len())
            .map(|index| self.render_row(index, &theme, cx).into_any_element())
            .collect();
        let properties = self.properties.clone();
        div()
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .child(SectionHeader::new("Filter"))
            .when(rows.is_empty(), |panel| {
                panel.child(EmptyState::new("Every row is shown.").inline())
            })
            .children(rows)
            .child(SectionHeader::new("Add a filter"))
            .children(properties.into_iter().map(|property| {
                let id = property.id;
                let name = property.name.clone();
                let property_type = property.property_type;
                ListItem::new(ElementId::Name(format!("add-filter-{}", id.0).into()), name)
                    .start_slot(Icon::new(type_icon(property_type)).size(IconSize::Small))
                    .on_click(cx.listener(move |editor, _, window, cx| {
                        editor.add_row(property.clone(), window, cx)
                    }))
            }))
    }
}
