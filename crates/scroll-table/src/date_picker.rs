//! Picks a date cell's value: type a date, or click one in a month calendar.

use gpui::{
    App, AppContext, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Window, div,
    prelude::FluentBuilder,
};
use scroll_core::record::Date;
use scroll_ui::{
    Button, ButtonSize, ButtonVariant, Calendar, CalendarDate, CalendarMonth, FieldError,
    TextInput, TextInputEvent, Theme,
};

use crate::values::{format_date, parse_date};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatePickerEvent {
    /// `None` clears the cell.
    Picked(Option<Date>),
}

pub struct DatePicker {
    month: CalendarMonth,
    selected: Option<Date>,
    input: Entity<TextInput>,
    error: Option<SharedString>,
    _input_subscription: Subscription,
}

impl EventEmitter<DatePickerEvent> for DatePicker {}
impl EventEmitter<DismissEvent> for DatePicker {}

impl Focusable for DatePicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl DatePicker {
    pub fn new(selected: Option<Date>, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx).placeholder("2026-11-09 or 9 Nov 2026");
            if let Some(date) = selected {
                input.set_text(format_date(date), cx);
            }
            input
        });
        let subscription = cx.subscribe(&input, |picker, _, event: &TextInputEvent, cx| {
            if *event == TextInputEvent::Submitted {
                picker.pick_typed(cx);
            }
        });
        DatePicker {
            month: CalendarMonth::of(selected.map_or_else(CalendarDate::today, calendar_date)),
            selected,
            input,
            error: None,
            _input_subscription: subscription,
        }
    }

    /// Starts with `text` already typed in the date field.
    pub fn set_typed_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.set_text(text, cx));
    }

    fn pick_typed(&mut self, cx: &mut Context<Self>) {
        let text = self.input.read(cx).text().trim().to_string();
        if text.is_empty() {
            cx.emit(DatePickerEvent::Picked(None));
            return;
        }
        match parse_date(&text) {
            Ok(date) => cx.emit(DatePickerEvent::Picked(Some(date))),
            Err(error) => {
                self.error = Some(error.to_string().into());
                cx.notify();
            }
        }
    }

    fn show_month(&mut self, month: CalendarMonth, cx: &mut Context<Self>) {
        self.month = month;
        cx.notify();
    }
}

impl Render for DatePicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let picked = cx.entity().downgrade();
        let shown = cx.entity().downgrade();
        let calendar = Calendar::new("calendar", self.month)
            .when_some(self.selected, |calendar, date| {
                calendar.selected(calendar_date(date))
            })
            .on_pick(move |date, _, cx| {
                if let Some(picker) = picked.upgrade() {
                    let date = core_date(*date);
                    picker.update(cx, |_, cx| cx.emit(DatePickerEvent::Picked(Some(date))));
                }
            })
            .on_show_month(move |month, _, cx| {
                if let Some(picker) = shown.upgrade() {
                    picker.update(cx, |picker, cx| picker.show_month(*month, cx));
                }
            });
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.sm)
            .p(theme.spacing.xs)
            .child(self.input.clone())
            .when_some(self.error.clone(), |panel, error| {
                panel.child(FieldError::new(error))
            })
            .child(calendar)
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(
                        Button::new("today", "Today")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Small)
                            .on_click(cx.listener(|_, _, _, cx| {
                                let today = core_date(CalendarDate::today());
                                cx.emit(DatePickerEvent::Picked(Some(today)))
                            })),
                    )
                    .child(
                        Button::new("clear-date", "Clear")
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Small)
                            .on_click(
                                cx.listener(|_, _, _, cx| cx.emit(DatePickerEvent::Picked(None))),
                            ),
                    ),
            )
    }
}

fn calendar_date(date: Date) -> CalendarDate {
    CalendarDate {
        year: date.year,
        month: date.month,
        day: date.day,
    }
}

fn core_date(date: CalendarDate) -> Date {
    Date {
        year: date.year,
        month: date.month,
        day: date.day,
    }
}
