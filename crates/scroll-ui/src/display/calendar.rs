use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use gpui::{
    AnyView, App, AppContext, Context, ElementId, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, RenderOnce, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder,
};

use crate::controls::button::ButtonSize;
use crate::controls::icon_button::IconButton;
use crate::display::icon::IconName;
use crate::theme::Theme;

const WEEKDAY_LETTERS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];
const MONTH_NAMES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// A day in the proleptic Gregorian calendar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CalendarDate {
    pub year: i32,
    /// 1 to 12.
    pub month: u8,
    pub day: u8,
}

impl CalendarDate {
    /// Today in the system's local time zone.
    pub fn today() -> CalendarDate {
        let now: libc::time_t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs() as libc::time_t)
            .unwrap_or_default();
        let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
        // SAFETY: `localtime_r` writes a complete `tm` into the buffer when it returns non-null,
        // and reads only `now`. It is the thread-safe form of `localtime`.
        let converted = unsafe { libc::localtime_r(&now, local.as_mut_ptr()) };
        if converted.is_null() {
            return date_from_days_since_epoch(now.div_euclid(86_400));
        }
        // SAFETY: initialised by the successful call above.
        let local = unsafe { local.assume_init() };
        CalendarDate {
            year: local.tm_year + 1900,
            month: (local.tm_mon + 1) as u8,
            day: local.tm_mday as u8,
        }
    }

    fn days_in_month(self) -> u8 {
        let is_leap_year = (self.year % 4 == 0 && self.year % 100 != 0) || self.year % 400 == 0;
        match self.month {
            2 if is_leap_year => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        }
    }
}

/// A month being shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalendarMonth {
    pub year: i32,
    /// 1 to 12.
    pub month: u8,
}

impl CalendarMonth {
    pub fn of(date: CalendarDate) -> CalendarMonth {
        CalendarMonth {
            year: date.year,
            month: date.month,
        }
    }

    pub fn next(self) -> CalendarMonth {
        match self.month {
            12 => CalendarMonth {
                year: self.year + 1,
                month: 1,
            },
            month => CalendarMonth {
                year: self.year,
                month: month + 1,
            },
        }
    }

    pub fn previous(self) -> CalendarMonth {
        match self.month {
            1 => CalendarMonth {
                year: self.year - 1,
                month: 12,
            },
            month => CalendarMonth {
                year: self.year,
                month: month - 1,
            },
        }
    }

    /// "Nov 2026".
    pub fn title(self) -> String {
        let name = MONTH_NAMES[usize::from(self.month.clamp(1, 12)) - 1];
        format!("{name} {}", self.year)
    }

    /// The month's days laid out in weeks starting on Monday; `None` pads the first and last week.
    fn weeks(self) -> Vec<[Option<CalendarDate>; 7]> {
        let first = CalendarDate {
            year: self.year,
            month: self.month,
            day: 1,
        };
        let leading_blanks = usize::from(weekday_from_monday(first));
        let mut cells: Vec<Option<CalendarDate>> = vec![None; leading_blanks];
        cells.extend((1..=first.days_in_month()).map(|day| Some(CalendarDate { day, ..first })));
        cells.resize(cells.len().div_ceil(7) * 7, None);
        cells
            .chunks(7)
            .map(|week| {
                let mut days = [None; 7];
                days.copy_from_slice(week);
                days
            })
            .collect()
    }
}

/// 0 for Monday through 6 for Sunday.
fn weekday_from_monday(date: CalendarDate) -> u8 {
    // 1970-01-01 was a Thursday, three days after a Monday.
    let days = days_since_epoch(date) + 3;
    days.rem_euclid(7) as u8
}

// Howard Hinnant's civil-date algorithms (days_from_civil and civil_from_days).

fn days_since_epoch(date: CalendarDate) -> i64 {
    let year = i64::from(date.year) - i64::from(date.month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(date.month);
    let day_of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(date.day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn date_from_days_since_epoch(days: i64) -> CalendarDate {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    CalendarDate {
        year: year as i32,
        month: month as u8,
        day: day as u8,
    }
}

type PickHandler = Rc<dyn Fn(&CalendarDate, &mut Window, &mut App) + 'static>;
type MonthHandler = Rc<dyn Fn(&CalendarMonth, &mut Window, &mut App) + 'static>;

/// A month of days to click, with buttons to step between months. The caller owns which
/// month is shown and which day is chosen.
#[derive(IntoElement)]
pub struct Calendar {
    id: ElementId,
    month: CalendarMonth,
    selected: Option<CalendarDate>,
    today: CalendarDate,
    on_pick: Option<PickHandler>,
    on_show_month: Option<MonthHandler>,
}

impl Calendar {
    pub fn new(id: impl Into<ElementId>, month: CalendarMonth) -> Self {
        Calendar {
            id: id.into(),
            month,
            selected: None,
            today: CalendarDate::today(),
            on_pick: None,
            on_show_month: None,
        }
    }

    pub fn selected(mut self, date: CalendarDate) -> Self {
        self.selected = Some(date);
        self
    }

    pub fn on_pick(
        mut self,
        handler: impl Fn(&CalendarDate, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_pick = Some(Rc::new(handler));
        self
    }

    /// Called with the month to show when a previous/next button is pressed.
    pub fn on_show_month(
        mut self,
        handler: impl Fn(&CalendarMonth, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_show_month = Some(Rc::new(handler));
        self
    }

    fn render_day(&self, date: CalendarDate, theme: &Theme) -> impl IntoElement {
        let is_selected = self.selected == Some(date);
        let is_today = self.today == date;
        let on_pick = self.on_pick.clone();
        div()
            .id(ElementId::NamedChild(
                Box::new(self.id.clone()),
                format!("{}-{}-{}", date.year, date.month, date.day).into(),
            ))
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .h(theme.size.control_small)
            .rounded(theme.radius.sm)
            .cursor_pointer()
            .text_size(theme.text.small.size)
            .when(is_today, |day| {
                day.font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.colors.accent)
            })
            .when(is_selected, |day| {
                day.bg(theme.colors.accent)
                    .text_color(theme.colors.text_on_accent)
            })
            .when(!is_selected, |day| {
                day.hover(|style| style.bg(theme.colors.hover))
            })
            .child(date.day.to_string())
            .on_click(move |_, window, cx| {
                if let Some(on_pick) = on_pick.as_ref() {
                    on_pick(&date, window, cx);
                }
            })
    }

    fn month_button(&self, icon: IconName, month: CalendarMonth) -> IconButton {
        let on_show_month = self.on_show_month.clone();
        IconButton::new(
            ElementId::NamedChild(Box::new(self.id.clone()), format!("{icon:?}").into()),
            icon,
        )
        .size(ButtonSize::Small)
        .on_click(move |_, window, cx| {
            if let Some(on_show_month) = on_show_month.as_ref() {
                on_show_month(&month, window, cx);
            }
        })
    }
}

impl RenderOnce for Calendar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        let month = self.month;
        let weekday_row = div().flex().children(WEEKDAY_LETTERS.map(|letter| {
            div()
                .flex_1()
                .text_center()
                .text_size(theme.text.caption.size)
                .text_color(theme.colors.text_placeholder)
                .child(letter)
        }));
        let weeks: Vec<_> = month
            .weeks()
            .into_iter()
            .map(|week| {
                div()
                    .flex()
                    .gap(theme.spacing.xxs)
                    .children(week.into_iter().map(|day| match day {
                        Some(date) => self.render_day(date, &theme).into_any_element(),
                        None => div().flex_1().into_any_element(),
                    }))
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(theme.spacing.xxs)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .px(theme.spacing.xs)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(month.title()),
                    )
                    .child(
                        div()
                            .flex()
                            .child(self.month_button(IconName::ArrowLeft, month.previous()))
                            .child(self.month_button(IconName::ArrowRight, month.next())),
                    ),
            )
            .child(weekday_row)
            .children(weeks)
    }
}

impl Calendar {
    pub fn story(_: &mut Window, cx: &mut App) -> AnyView {
        let today = CalendarDate::today();
        cx.new(|_| CalendarStory {
            month: CalendarMonth::of(today),
            selected: today,
        })
        .into()
    }
}

struct CalendarStory {
    month: CalendarMonth,
    selected: CalendarDate,
}

impl Render for CalendarStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::current(window, cx);
        div().w(theme.size.popover_min_width * 1.25).child(
            Calendar::new("calendar", self.month)
                .selected(self.selected)
                .on_pick(cx.listener(|story, date: &CalendarDate, _, cx| {
                    story.selected = *date;
                    cx.notify();
                }))
                .on_show_month(cx.listener(|story, month: &CalendarMonth, _, cx| {
                    story.month = *month;
                    cx.notify();
                })),
        )
    }
}

#[cfg(test)]
mod tests {
    use gpui::{KeyUpEvent, Keystroke, TestAppContext};

    use super::*;
    use crate::foundation::story::testing::open_story;

    fn date(year: i32, month: u8, day: u8) -> CalendarDate {
        CalendarDate { year, month, day }
    }

    #[test]
    fn weekdays_match_known_dates() {
        assert_eq!(weekday_from_monday(date(1970, 1, 1)), 3);
        assert_eq!(weekday_from_monday(date(2026, 11, 9)), 0);
        assert_eq!(weekday_from_monday(date(2000, 2, 29)), 1);
        assert_eq!(weekday_from_monday(date(1969, 12, 31)), 2);
    }

    #[test]
    fn days_since_epoch_round_trips() {
        for days in [-800_000, -1, 0, 1, 19_000, 20_401, 2_000_000] {
            assert_eq!(days_since_epoch(date_from_days_since_epoch(days)), days);
        }
        assert_eq!(date_from_days_since_epoch(0), date(1970, 1, 1));
        assert_eq!(date_from_days_since_epoch(20_402), date(2025, 11, 10));
    }

    #[test]
    fn a_month_is_laid_out_in_monday_weeks() {
        // November 2026 starts on a Sunday and has 30 days.
        let weeks = CalendarMonth {
            year: 2026,
            month: 11,
        }
        .weeks();
        assert_eq!(weeks.len(), 6);
        assert_eq!(weeks[0][..6], [None; 6]);
        assert_eq!(weeks[0][6], Some(date(2026, 11, 1)));
        assert_eq!(weeks[5][0], Some(date(2026, 11, 30)));
        assert_eq!(weeks[5][1..], [None; 6]);
    }

    #[test]
    fn months_step_across_years() {
        let december = CalendarMonth {
            year: 2026,
            month: 12,
        };
        assert_eq!(
            december.next(),
            CalendarMonth {
                year: 2027,
                month: 1
            }
        );
        assert_eq!(december.next().previous(), december);
    }

    #[test]
    fn today_is_a_real_date_near_utc_today() {
        let local = CalendarDate::today();
        assert!(
            (1..=12).contains(&local.month) && (1..=local.days_in_month()).contains(&local.day)
        );
        let utc_days = (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            / 86_400) as i64;
        assert!((days_since_epoch(local) - utc_days).abs() <= 1);
    }

    #[gpui::test]
    fn the_next_month_button_shows_the_next_month(cx: &mut TestAppContext) {
        let (story, cx) = open_story(Calendar::story, cx);
        let story = story
            .downcast::<CalendarStory>()
            .expect("the calendar story");
        let shown = story.read_with(cx, |story, _| story.month);
        cx.simulate_keystrokes("tab tab space");
        cx.simulate_event(KeyUpEvent {
            keystroke: Keystroke::parse("space").expect("a key"),
        });
        assert_eq!(story.read_with(cx, |story, _| story.month), shown.next());
    }
}
