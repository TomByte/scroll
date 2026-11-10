//! What the app knows about workspaces and servers while it runs: the registry (saved after
//! every change) and a session with each server.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{App, Global};
use scroll_sync::{AccountClient, Membership};

use crate::accounts::registry::Registry;

/// A server's state as this run of the app knows it.
pub struct ServerSession {
    pub client: Arc<AccountClient>,
    /// The signed-in person's workspaces there, as last fetched.
    pub workspaces: Vec<Membership>,
    /// Why the last fetch failed, shown in the switcher.
    pub problem: Option<String>,
}

pub struct Accounts {
    data_directory: PathBuf,
    registry: Registry,
    servers: HashMap<String, ServerSession>,
    /// The open workspace is being published, which can take minutes for a large one.
    is_publishing: bool,
}

impl Global for Accounts {}

impl Accounts {
    pub fn install(data_directory: PathBuf, registry: Registry, cx: &mut App) {
        cx.set_global(Accounts {
            data_directory,
            registry,
            servers: HashMap::new(),
            is_publishing: false,
        });
    }

    pub fn get(cx: &App) -> &Accounts {
        cx.global::<Accounts>()
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn data_directory(&self) -> &Path {
        &self.data_directory
    }

    /// The file of workspace `index`.
    pub fn path_of(&self, index: usize) -> PathBuf {
        self.registry.path_of(index, &self.data_directory)
    }

    pub fn is_publishing(&self) -> bool {
        self.is_publishing
    }

    pub fn start_publishing(cx: &mut App) {
        cx.global_mut::<Accounts>().is_publishing = true;
    }

    pub fn finish_publishing(cx: &mut App) {
        cx.global_mut::<Accounts>().is_publishing = false;
    }

    pub fn server(&self, url: &str) -> Option<&ServerSession> {
        self.servers.get(url)
    }

    /// Changes the registry and saves it.
    pub fn edit_registry<R>(cx: &mut App, edit: impl FnOnce(&mut Registry) -> R) -> R {
        let accounts = cx.global_mut::<Accounts>();
        let result = edit(&mut accounts.registry);
        if let Err(error) = accounts.registry.save(&accounts.data_directory) {
            log::error!("could not save the list of workspaces: {error}");
        }
        result
    }

    /// The session with `url`, made with `connect` the first time it is needed.
    pub fn session_with<E>(
        cx: &mut App,
        url: &str,
        connect: impl FnOnce() -> Result<Arc<AccountClient>, E>,
    ) -> Result<Arc<AccountClient>, E> {
        let accounts = cx.global_mut::<Accounts>();
        if let Some(session) = accounts.servers.get(url) {
            return Ok(session.client.clone());
        }
        let client = connect()?;
        accounts.servers.insert(
            url.to_string(),
            ServerSession {
                client: client.clone(),
                workspaces: Vec::new(),
                problem: None,
            },
        );
        Ok(client)
    }

    pub fn update_session(cx: &mut App, url: &str, update: impl FnOnce(&mut ServerSession)) {
        if let Some(session) = cx.global_mut::<Accounts>().servers.get_mut(url) {
            update(session);
        }
    }

    pub fn forget_session(cx: &mut App, url: &str) {
        cx.global_mut::<Accounts>().servers.remove(url);
    }
}

/// Opens workspace `index` of the registry. The app handles it; lower layers dispatch it so
/// they need not know how a workspace is shown.
#[derive(Clone, PartialEq, Eq, Debug, gpui::Action)]
#[action(namespace = scroll, no_json)]
pub struct OpenWorkspace {
    pub index: usize,
}

/// Asks the app to open workspace `index`, once the current event is done.
pub fn request_open_workspace(index: usize, cx: &mut App) {
    cx.defer(move |cx| cx.dispatch_action(&OpenWorkspace { index }));
}
