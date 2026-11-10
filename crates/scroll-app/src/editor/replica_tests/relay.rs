//! An in-process stand-in for the sync server: forwards one editor's Loro updates and caret
//! to another after an artificial delay, as a network would.

use std::time::Duration;

use gpui::{Context, Entity, SharedString, Subscription};

use crate::editor::{PageEditor, PageEditorEvent};

pub struct RelayLink {
    pub to: Entity<PageEditor>,
    /// How the receiving pane labels the sender's caret.
    pub sender_name: SharedString,
    pub latency: Duration,
}

/// Forwards everything `from` emits to `link.to` until the subscription is dropped.
pub fn relay<Owner: 'static>(
    from: &Entity<PageEditor>,
    link: RelayLink,
    cx: &mut Context<Owner>,
) -> Subscription {
    cx.subscribe(from, move |_, sender, event, cx| match event {
        PageEditorEvent::Edited => {
            let updates = sender.update(cx, |sender, _| sender.take_outgoing_updates());
            deliver(&link, cx, move |receiver, cx| {
                receiver.apply_remote_updates(&updates, cx)
            });
        }
        PageEditorEvent::CreateDatabase => {}
        PageEditorEvent::CaretMoved => {
            let Some(presence) = sender.read(cx).presence(link.sender_name.clone()) else {
                return;
            };
            deliver(&link, cx, move |receiver, cx| {
                receiver.set_collaborator(0, Some(presence), cx);
            });
        }
    })
}

/// Sends `from`'s current caret to `link.to` straight away, so each pane shows the other's
/// caret before anyone has moved.
pub fn introduce<Owner: 'static>(
    from: &Entity<PageEditor>,
    link: &RelayLink,
    cx: &mut Context<Owner>,
) {
    let Some(presence) = from.read(cx).presence(link.sender_name.clone()) else {
        return;
    };
    link.to.update(cx, |receiver, cx| {
        receiver.set_collaborator(0, Some(presence), cx);
    });
}

fn deliver<Owner: 'static>(
    link: &RelayLink,
    cx: &mut Context<Owner>,
    apply: impl FnOnce(&mut PageEditor, &mut Context<PageEditor>) + 'static,
) {
    let receiver = link.to.downgrade();
    let latency = link.latency;
    cx.spawn(async move |_, cx| {
        cx.background_executor().timer(latency).await;
        if let Err(error) = receiver.update(cx, apply) {
            log::warn!("relay target was dropped before delivery: {error}");
        }
    })
    .detach();
}
