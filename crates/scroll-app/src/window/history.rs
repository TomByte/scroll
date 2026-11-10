//! Back and forward through visited pages, as in a browser.

use scroll_core::id::PageId;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    back: Vec<PageId>,
    current: Option<PageId>,
    forward: Vec<PageId>,
}

impl History {
    pub fn current(&self) -> Option<PageId> {
        self.current
    }

    /// Visiting a new page clears the forward stack; revisiting the current page does nothing.
    pub fn visit(&mut self, page: PageId) {
        if self.current == Some(page) {
            return;
        }
        if let Some(current) = self.current.replace(page) {
            self.back.push(current);
        }
        self.forward.clear();
    }

    pub fn go_back(&mut self) -> Option<PageId> {
        let previous = self.back.pop()?;
        if let Some(current) = self.current.replace(previous) {
            self.forward.push(current);
        }
        Some(previous)
    }

    pub fn go_forward(&mut self) -> Option<PageId> {
        let next = self.forward.pop()?;
        if let Some(current) = self.current.replace(next) {
            self.back.push(current);
        }
        Some(next)
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// Drops a page that no longer exists (e.g. moved to the trash) from both stacks.
    pub fn forget(&mut self, page: PageId) {
        self.back.retain(|candidate| *candidate != page);
        self.forward.retain(|candidate| *candidate != page);
        if self.current == Some(page) {
            self.current = self.back.pop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_and_forward_walk_the_visits() {
        let [a, b, c] = [PageId::generate(), PageId::generate(), PageId::generate()];
        let mut history = History::default();
        history.visit(a);
        history.visit(b);
        history.visit(c);
        assert_eq!(history.go_back(), Some(b));
        assert_eq!(history.go_back(), Some(a));
        assert_eq!(history.go_back(), None);
        assert_eq!(history.go_forward(), Some(b));
        history.visit(a);
        assert!(!history.can_go_forward(), "a new visit clears forward");
        assert_eq!(history.current(), Some(a));
    }

    #[test]
    fn forgetting_the_current_page_falls_back() {
        let [a, b] = [PageId::generate(), PageId::generate()];
        let mut history = History::default();
        history.visit(a);
        history.visit(b);
        history.visit(a);
        history.forget(a);
        assert_eq!(history.current(), Some(b));
        assert!(!history.can_go_back());
    }
}
