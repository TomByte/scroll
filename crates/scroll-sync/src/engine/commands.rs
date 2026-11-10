//! What the app asks of the engine, followed whether or not it is connected: what is open is remembered, and
//! told to the server when there is one.

use std::collections::VecDeque;
use std::time::Duration;

use scroll_api::limits::MAX_OPEN_DATABASES;
use scroll_api::protocol::ClientMessage;
use scroll_core::id::{DatabaseId, PageId};
use tokio::time::sleep;

use super::connection::Connection;
use super::worker::{Interrupt, Worker};

pub(crate) enum Command {
    OpenPage(PageId),
    ClosePage(PageId),
    OpenDatabase(DatabaseId),
    CloseDatabase(DatabaseId),
    Presence { page_id: PageId, state: Vec<u8> },
    ShutDown,
}

/// The databases the app shows rows of, oldest opened first, kept across reconnects. Never more than the server
/// lets a connection open: opening one more closes the one opened longest ago, whose rows then arrive with
/// other changes, a little later.
#[derive(Default)]
pub(crate) struct OpenDatabases(VecDeque<DatabaseId>);

/// What opening a database changed.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Opened {
    AlreadyOpen,
    New { closed: Option<DatabaseId> },
}

impl OpenDatabases {
    pub(super) fn open(&mut self, database_id: DatabaseId) -> Opened {
        if self.0.contains(&database_id) {
            return Opened::AlreadyOpen;
        }
        let closed = if self.0.len() >= MAX_OPEN_DATABASES {
            self.0.pop_front()
        } else {
            None
        };
        self.0.push_back(database_id);
        Opened::New { closed }
    }

    /// Whether it was open.
    pub(super) fn close(&mut self, database_id: DatabaseId) -> bool {
        let Some(index) = self.0.iter().position(|open| *open == database_id) else {
            return false;
        };
        self.0.remove(index);
        true
    }

    pub(super) fn to_vec(&self) -> Vec<DatabaseId> {
        self.0.iter().copied().collect()
    }
}

impl Worker {
    /// Waits out the reconnect delay, still following the app's commands.
    pub(super) async fn wait_offline(&mut self, delay: Duration) -> Result<(), Interrupt> {
        let deadline = sleep(delay);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                () = &mut deadline => return Ok(()),
                command = self.commands.recv() => self.obey(command, None).await?,
            }
        }
    }

    /// `None` for `command` means the app has dropped the engine. `connection` is `None` while offline.
    pub(super) async fn obey(
        &mut self,
        command: Option<Command>,
        connection: Option<&mut Connection>,
    ) -> Result<(), Interrupt> {
        let Some(command) = command else {
            return Err(Interrupt::ShutDown);
        };
        match command {
            Command::ShutDown => Err(Interrupt::ShutDown),
            Command::OpenPage(page_id) => {
                self.open_pages.insert(page_id);
                match connection {
                    Some(connection) => self.open_on_server(connection, page_id).await,
                    None => Ok(()),
                }
            }
            Command::ClosePage(page_id) => {
                self.forget_page(page_id)?;
                match connection {
                    Some(connection) => self.close_on_server_if_idle(connection, page_id).await,
                    None => Ok(()),
                }
            }
            Command::OpenDatabase(database_id) => {
                let opened = self.open_databases.open(database_id);
                match (opened, connection) {
                    (Opened::New { closed }, Some(connection)) => {
                        if let Some(closed) = closed {
                            self.close_database_on_server(connection, closed).await?;
                        }
                        self.open_database_on_server(connection, database_id).await
                    }
                    _ => Ok(()),
                }
            }
            Command::CloseDatabase(database_id) => {
                let was_open = self.open_databases.close(database_id);
                match connection {
                    Some(connection) if was_open => {
                        self.close_database_on_server(connection, database_id).await
                    }
                    _ => Ok(()),
                }
            }
            Command::Presence { page_id, state } => match connection {
                Some(connection) if connection.open_pages.contains(&page_id) => {
                    connection
                        .send(&ClientMessage::Presence { page_id, state })
                        .await
                }
                _ => Ok(()),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_past_the_servers_limit_closes_the_database_opened_longest_ago() {
        let mut open = OpenDatabases::default();
        let databases: Vec<DatabaseId> = (0..=MAX_OPEN_DATABASES)
            .map(|_| DatabaseId::generate())
            .collect();
        for database_id in &databases[..MAX_OPEN_DATABASES] {
            assert_eq!(open.open(*database_id), Opened::New { closed: None });
        }
        assert_eq!(open.open(databases[1]), Opened::AlreadyOpen);

        let last = databases[MAX_OPEN_DATABASES];
        assert_eq!(
            open.open(last),
            Opened::New {
                closed: Some(databases[0])
            }
        );
        assert_eq!(open.to_vec().len(), MAX_OPEN_DATABASES);
        assert!(!open.close(databases[0]));
        assert!(open.close(last));
    }
}
