//! Stories: each component shows itself in the gallery through an associated `story` function.

use gpui::{AnyElement, AnyView, App, AppContext, Context, IntoElement, Render, Window};

/// Builds a component's gallery story once per gallery window.
pub type StoryBuilder = fn(&mut Window, &mut App) -> AnyView;

/// A story with no state of its own: `render` draws it every frame.
pub(crate) fn stateless_story(
    render: fn(&mut Window, &mut App) -> AnyElement,
    cx: &mut App,
) -> AnyView {
    cx.new(|_| StatelessStory { render }).into()
}

struct StatelessStory {
    render: fn(&mut Window, &mut App) -> AnyElement,
}

impl Render for StatelessStory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        (self.render)(window, cx)
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use gpui::{
        AnyView, Context, FocusHandle, InteractiveElement, IntoElement, ParentElement, Render,
        Styled, TestAppContext, VisualTestContext, Window, div,
    };

    use super::StoryBuilder;
    use crate::foundation::focus::tab_navigation;

    /// Focus starts on the window itself, as in an app window, so Tab has somewhere to go from.
    struct StoryWindow {
        story: AnyView,
        focus_handle: FocusHandle,
    }

    impl Render for StoryWindow {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            tab_navigation(div())
                .track_focus(&self.focus_handle)
                .size_full()
                .child(self.story.clone())
        }
    }

    /// Opens `story` in a window on GPUI's headless platform, drawn once.
    pub(crate) fn open_story(
        story: StoryBuilder,
        cx: &mut TestAppContext,
    ) -> (AnyView, &mut VisualTestContext) {
        cx.update(crate::init);
        let (window, cx) = cx.add_window_view(|window, cx| {
            let focus_handle = cx.focus_handle();
            window.focus(&focus_handle);
            StoryWindow {
                story: story(window, cx),
                focus_handle,
            }
        });
        cx.run_until_parked();
        let story = window.read_with(cx, |window, _| window.story.clone());
        (story, cx)
    }
}
