//! App-level settings in `workspaces.json` next to the workspace files: the workspaces on this
//! Mac, the servers the app knows, and who is signed in to each. Nothing secret lives here:
//! session tokens are in the Keychain.

use std::path::{Path, PathBuf};

use scroll_core::id::{UserId, WorkspaceId};
use serde::{Deserialize, Deserializer, Serialize};

use crate::json_file;

const REGISTRY_FILE: &str = "workspaces.json";

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("Scroll cannot read its list of workspaces in {0}")]
    Unreadable(#[from] json_file::UnreadableFile),
}
/// The file the first workspace has always used, so existing data opens unchanged.
const FIRST_WORKSPACE_FILE: &str = "Workspace.scroll";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerLink {
    pub server_url: String,
    pub workspace_id: WorkspaceId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub name: String,
    /// File name inside the data directory.
    pub file: String,
    pub server: Option<ServerLink>,
}

/// Someone signed in to one server. The token is kept under this server and user id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub server_url: String,
    pub email: String,
    /// What other people see on this person's caret: Google's name, or the email's local part.
    pub display_name: String,
    pub user_id: UserId,
}

impl Account {
    /// How menus name the account: "Ada Lovelace (ada@example.com)".
    pub fn label(&self) -> String {
        format!("{} ({})", self.display_name, self.email)
    }
}

/// A server the app has been pointed at, signed in or not.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerEntry {
    /// Normalised, as `scroll_sync::normalize_server_url` returns it.
    pub url: String,
    #[serde(deserialize_with = "account_or_signed_out")]
    pub account: Option<Account>,
}

/// An account saved by an earlier build, without the email or name sign-in now records, reads
/// as signed out: the person signs in again, and the workspace list is kept.
fn account_or_signed_out<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Account>, D::Error> {
    let Some(saved) = Option::<serde_json::Value>::deserialize(deserializer)? else {
        return Ok(None);
    };
    match serde_json::from_value(saved) {
        Ok(account) => Ok(Some(account)),
        Err(error) => {
            log::warn!("signing out an account saved by an earlier version: {error}");
            Ok(None)
        }
    }
}

impl ServerEntry {
    /// The host, which is how people name their server.
    pub fn name(&self) -> &str {
        let without_scheme = self
            .url
            .split_once("://")
            .map_or(self.url.as_str(), |(_, rest)| rest);
        without_scheme.split('/').next().unwrap_or(without_scheme)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Registry {
    pub workspaces: Vec<WorkspaceEntry>,
    /// Index into `workspaces` of the one open last.
    pub current: usize,
    pub servers: Vec<ServerEntry>,
}

impl Registry {
    /// The registry on disk, or one listing just the first workspace when there is none yet.
    /// A file that exists but cannot be read is an error, so it is never saved over.
    pub fn load(directory: &Path) -> Result<Registry, RegistryError> {
        let mut registry: Registry =
            json_file::read(&directory.join(REGISTRY_FILE))?.unwrap_or_default();
        if registry.workspaces.is_empty() {
            registry.workspaces.push(WorkspaceEntry {
                name: "My workspace".into(),
                file: FIRST_WORKSPACE_FILE.into(),
                server: None,
            });
        }
        registry.current = registry.current.min(registry.workspaces.len() - 1);
        Ok(registry)
    }

    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        json_file::save(&directory.join(REGISTRY_FILE), self)
    }

    pub fn current(&self) -> &WorkspaceEntry {
        &self.workspaces[self.current]
    }

    pub fn path_of(&self, index: usize, directory: &Path) -> PathBuf {
        directory.join(&self.workspaces[index].file)
    }

    /// A file name no workspace has, for a new workspace's file.
    pub fn new_workspace_file() -> String {
        format!("{}.scroll", uuid::Uuid::now_v7())
    }

    /// Adds a workspace kept in `file` and returns its index.
    pub fn add_workspace(&mut self, name: &str, file: String, server: Option<ServerLink>) -> usize {
        self.workspaces.push(WorkspaceEntry {
            name: name.to_string(),
            file,
            server,
        });
        self.workspaces.len() - 1
    }

    /// The local copy of a server workspace, if this Mac has one.
    pub fn index_of_server_workspace(
        &self,
        server_url: &str,
        workspace_id: WorkspaceId,
    ) -> Option<usize> {
        self.workspaces.iter().position(|entry| {
            entry.server.as_ref().is_some_and(|link| {
                link.server_url == server_url && link.workspace_id == workspace_id
            })
        })
    }

    pub fn server(&self, url: &str) -> Option<&ServerEntry> {
        self.servers.iter().find(|server| server.url == url)
    }

    pub fn account(&self, server_url: &str) -> Option<&Account> {
        self.server(server_url)?.account.as_ref()
    }

    /// Remembers `url` (already normalised); does nothing if it is known.
    pub fn add_server(&mut self, url: &str) {
        if self.server(url).is_none() {
            self.servers.push(ServerEntry {
                url: url.to_string(),
                account: None,
            });
        }
    }

    pub fn set_account(&mut self, server_url: &str, account: Option<Account>) {
        self.add_server(server_url);
        if let Some(server) = self
            .servers
            .iter_mut()
            .find(|server| server.url == server_url)
        {
            server.account = account;
        }
    }

    pub fn remove_server(&mut self, url: &str) {
        self.servers.retain(|server| server.url != url);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_account_from_an_earlier_build_reads_as_signed_out_and_keeps_the_workspaces() {
        let earlier = r#"{
            "workspaces": [{"name": "Team", "file": "Team.scroll", "server": null}],
            "current": 0,
            "servers": [{
                "url": "http://localhost:8080",
                "account": {
                    "server_url": "http://localhost:8080",
                    "email": null,
                    "user_id": "0190a0b0-0000-7000-8000-000000000001"
                }
            }]
        }"#;
        let registry: Registry = serde_json::from_str(earlier).unwrap();
        assert_eq!(registry.workspaces[0].name, "Team");
        assert_eq!(registry.servers[0].account, None);
    }

    #[test]
    fn a_fresh_registry_lists_the_first_workspace_and_round_trips() {
        let directory =
            std::env::temp_dir().join(format!("scroll-registry-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut registry = Registry::load(&directory).unwrap();
        assert_eq!(registry.workspaces.len(), 1);
        assert_eq!(registry.current().file, FIRST_WORKSPACE_FILE);

        let url = "http://localhost:8080";
        let workspace_id = WorkspaceId::generate();
        let link = ServerLink {
            server_url: url.into(),
            workspace_id,
        };
        let index = registry.add_workspace("Team", Registry::new_workspace_file(), Some(link));
        registry.current = index;
        registry.set_account(
            url,
            Some(Account {
                server_url: url.into(),
                email: "ada@example.com".into(),
                display_name: "Ada".into(),
                user_id: UserId::generate(),
            }),
        );
        registry.add_server("https://scroll.example.com");
        registry.save(&directory).unwrap();
        let loaded = Registry::load(&directory).unwrap();
        assert_eq!(loaded, registry);
        assert_eq!(
            loaded.index_of_server_workspace(url, workspace_id),
            Some(index)
        );
        assert_eq!(
            loaded.index_of_server_workspace("https://other.example", workspace_id),
            None
        );
        assert!(loaded.account(url).is_some());
        assert!(loaded.account("https://scroll.example.com").is_none());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_server_is_named_by_its_host() {
        let server = ServerEntry {
            url: "https://scroll.example.com/team".into(),
            account: None,
        };
        assert_eq!(server.name(), "scroll.example.com");
    }

    #[test]
    fn an_unreadable_registry_is_an_error_and_stays_on_disk() {
        let directory =
            std::env::temp_dir().join(format!("scroll-registry-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(REGISTRY_FILE);
        std::fs::write(&path, "{ not json").unwrap();

        assert!(matches!(
            Registry::load(&directory),
            Err(RegistryError::Unreadable(_))
        ));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ not json");
    }
}
