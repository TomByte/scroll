//! Who else is on the page: telling them where this person's caret is, and showing theirs.

use super::*;
use crate::workspace::presence::color_index;

impl PageView {
    /// Other people on this page right now.
    pub fn people(&self) -> impl Iterator<Item = &Person> {
        self.people.values()
    }

    pub(super) fn send_presence(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.my_name.clone() else {
            return;
        };
        let Some(editor) = self.editor() else {
            return;
        };
        let Some(me) = editor.read(cx).presence(SharedString::default()) else {
            return;
        };
        let state = PresenceState::new(
            name,
            me.head.encode(),
            me.anchor.map(|anchor| anchor.encode()),
        );
        self.workspace
            .read(cx)
            .send_presence(self.page, state.encode());
    }

    pub(super) fn show_person(
        &mut self,
        user: UserId,
        connection: ConnectionId,
        state: &[u8],
        cx: &mut Context<Self>,
    ) {
        let Some(state) = PresenceState::decode(state) else {
            return log::debug!("skipping a presence message this version cannot read");
        };
        let Some(head) = StableCursor::decode(&state.head) else {
            return;
        };
        let is_new = !self.people.contains_key(&connection);
        let color_index = color_index(user);
        let name = SharedString::from(state.name);
        self.people.insert(
            connection,
            Person {
                user,
                name: name.clone(),
                color_index,
            },
        );
        let collaborator = Collaborator {
            name,
            color_index,
            head,
            anchor: state.anchor.as_deref().and_then(StableCursor::decode),
        };
        if let Some(editor) = self.editor() {
            editor.update(cx, |editor, cx| {
                editor.set_collaborator(connection.0.as_u128(), Some(collaborator), cx)
            });
        }
        // Someone just arrived: tell them where this person is without waiting for a move.
        if is_new {
            self.send_presence(cx);
        }
        cx.notify();
    }
}
