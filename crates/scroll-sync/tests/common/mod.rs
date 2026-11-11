//! A real server in-process on `scroll_test`, and a stand-in for the app: its own store connection, live page
//! documents, and an engine whose events it drains. Every test makes its own users, workspaces and files.

#![allow(dead_code)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use deadpool_postgres::Pool;
use scroll_core::id::{PageId, Role, WorkspaceId};
use scroll_core::operation::{Operation, Record, Transaction};
use scroll_core::page_doc::{BlockKind, BlockPlacement, PageDoc};
use scroll_core::position::Position;
use scroll_core::record::Page;
use scroll_core::store::WorkspaceStore;
use scroll_server::{GoogleSignIn, Server, Settings, SignUpMode, test_support};
use scroll_sync::{
    AccountClient, ConnectionStatus, Membership, NewAccount, Session, SyncEngine, SyncError,
    SyncEvent, SyncEvents, create_linked_workspace_file,
};
use tokio::net::{TcpListener, TcpSocket, TcpStream};
use tokio::sync::{broadcast, oneshot};
use tokio::task::JoinHandle;
use uuid::Uuid;

const WAIT_LIMIT: Duration = Duration::from_secs(30);
pub const PASSWORD: &str = "correct horse battery staple";

pub struct TestServer {
    pub address: SocketAddr,
    pub pool: Pool,
    deferred_delivery: Duration,
    /// Google sign-in, with open sign-up; `None` for neither.
    google: Option<GoogleSignIn>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<std::io::Result<()>>>,
}

impl TestServer {
    pub async fn start() -> Self {
        Self::start_with_deferred_delivery(Settings::default().deferred_delivery).await
    }

    /// See `Settings::deferred_delivery`.
    pub async fn start_with_deferred_delivery(deferred_delivery: Duration) -> Self {
        Self::start_configured(deferred_delivery, None).await
    }

    /// Google sign-in against `google` (a fake), and open sign-up.
    pub async fn start_with_google(google: GoogleSignIn) -> Self {
        Self::start_configured(Settings::default().deferred_delivery, Some(google)).await
    }

    async fn start_configured(deferred_delivery: Duration, google: Option<GoogleSignIn>) -> Self {
        let pool = test_support::test_pool().await;
        let mut server = TestServer {
            address: "127.0.0.1:0".parse().unwrap(),
            pool,
            deferred_delivery,
            google,
            stop: None,
            task: None,
        };
        server.listen().await;
        server
    }

    /// Serves on `self.address`, keeping the port it had before so clients can reconnect to it.
    async fn listen(&mut self) {
        let socket = TcpSocket::new_v4().unwrap();
        socket.set_reuseaddr(true).unwrap();
        socket.bind(self.address).unwrap();
        let listener = socket.listen(128).unwrap();
        self.address = listener.local_addr().unwrap();
        let (stop, stopped) = oneshot::channel::<()>();
        let server = Server {
            pool: self.pool.clone(),
            settings: Settings {
                deferred_delivery: self.deferred_delivery,
                sign_up: if self.google.is_some() {
                    SignUpMode::Open
                } else {
                    SignUpMode::Invite
                },
                google: self.google.clone(),
                ..Settings::default()
            },
        };
        let shutdown = async move {
            let _ = stopped.await;
        };
        self.stop = Some(stop);
        self.task = Some(tokio::spawn(server.serve(listener, shutdown)));
    }

    pub async fn shut_down(&mut self) {
        self.stop.take().unwrap().send(()).unwrap();
        self.task.take().unwrap().await.unwrap().unwrap();
    }

    pub async fn restart(&mut self) {
        self.listen().await;
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    pub fn accounts(&self) -> AccountClient {
        AccountClient::new(&self.url(), None).unwrap()
    }

    pub async fn signup_invite(&self) -> String {
        scroll_server::create_signup_invite(&self.pool)
            .await
            .unwrap()
    }

    /// Signs up with `invite_code` (a fresh sign-up invite if `None`) and signs in.
    pub async fn new_user(&self, accounts: &AccountClient, invite_code: Option<String>) -> Session {
        let invite_code = match invite_code {
            Some(code) => code,
            None => self.signup_invite().await,
        };
        let email = format!("user-{}@example.test", Uuid::now_v7());
        let account = NewAccount {
            email: email.clone(),
            password: PASSWORD.into(),
            invite_code: Some(invite_code),
        };
        accounts.sign_up(account).await.unwrap();
        accounts.sign_in(&email, PASSWORD).await.unwrap()
    }
}

/// An owner and an editor of one new workspace.
pub struct Team {
    pub owner: Session,
    pub editor: Session,
    pub owner_membership: Membership,
    pub editor_membership: Membership,
}

impl Team {
    pub async fn create(server: &TestServer, accounts: &AccountClient) -> Team {
        let owner = server.new_user(accounts, None).await;
        let owner_membership = accounts
            .create_workspace(&owner.session_token, WorkspaceId::generate(), "Team")
            .await
            .unwrap();
        let invite = accounts
            .create_invite(
                &owner.session_token,
                owner_membership.workspace_id,
                Role::Editor,
            )
            .await
            .unwrap();
        let editor = server.new_user(accounts, Some(invite)).await;
        let editor_membership = accounts
            .workspaces(&editor.session_token)
            .await
            .unwrap()
            .remove(0);
        Team {
            owner,
            editor,
            owner_membership,
            editor_membership,
        }
    }
}

/// A directory removed when dropped.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> Self {
        let path = std::env::temp_dir().join(format!("scroll-sync-test-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What the app does: its own store connection, live documents for open pages, and the engine's events.
pub struct TestApp {
    pub path: PathBuf,
    pub store: WorkspaceStore,
    pub engine: Option<SyncEngine>,
    events: Option<SyncEvents>,
    pub docs: HashMap<PageId, PageDoc>,
    pub log: Vec<SyncEvent>,
    pub statuses: Vec<ConnectionStatus>,
    pub event_stream_ended: bool,
}

impl TestApp {
    /// A new file linked to `membership`'s workspace at `server_url`, opened by the app, with no engine yet.
    pub fn join(path: PathBuf, server_url: &str, membership: &Membership) -> TestApp {
        create_linked_workspace_file(&path, server_url, membership).unwrap();
        TestApp::open(path)
    }

    pub fn open(path: PathBuf) -> TestApp {
        let store = WorkspaceStore::open(&path).unwrap();
        TestApp {
            path,
            store,
            engine: None,
            events: None,
            docs: HashMap::new(),
            log: Vec::new(),
            statuses: Vec::new(),
            event_stream_ended: false,
        }
    }

    pub fn start(&mut self, session_token: &str) {
        self.start_trusting(session_token, None);
    }

    /// Starts the engine trusting `root_certificate` (PEM) as well as the system's certificates.
    pub fn start_trusting(&mut self, session_token: &str, root_certificate: Option<&[u8]>) {
        let (engine, events) =
            SyncEngine::start(&self.path, session_token.to_owned(), root_certificate).unwrap();
        self.engine = Some(engine);
        self.events = Some(events);
        self.event_stream_ended = false;
    }

    pub fn stop(&mut self) {
        self.engine.take().unwrap().shut_down();
        self.pump();
        self.events = None;
    }

    pub fn engine(&self) -> &SyncEngine {
        self.engine.as_ref().expect("the engine is running")
    }

    pub fn commit(&mut self, operations: Vec<Operation>) -> Transaction {
        let transaction = Transaction::new(operations);
        self.store.apply_local_transaction(&transaction).unwrap();
        if let Some(engine) = &self.engine {
            engine.local_change_queued();
        }
        transaction
    }

    pub fn create_page(&mut self, title: &str) -> Page {
        let page = new_page(title, None);
        self.commit(vec![Operation::Create(Record::Page(page.clone()))]);
        page
    }

    /// Loads the live document and opens the page in the engine, in one step as the app must.
    pub fn open_page(&mut self, page_id: PageId) {
        let doc = self.store.load_page_doc(page_id).unwrap();
        self.docs.insert(page_id, doc);
        if let Some(engine) = &self.engine {
            engine.open_page(page_id);
        }
    }

    /// Appends a paragraph with `text` to the live document and queues the update.
    pub fn write_paragraph(&mut self, page_id: PageId, text: &str) {
        let doc = self.docs.get_mut(&page_id).expect("the page is open");
        let block = doc
            .insert_block(BlockPlacement::LastChildOf(None), BlockKind::Paragraph)
            .unwrap();
        doc.insert_text(block, 0, text).unwrap();
        let update = doc.take_local_update().unwrap().unwrap();
        self.store
            .record_local_page_update(page_id, &update)
            .unwrap();
        if let Some(engine) = &self.engine {
            engine.local_change_queued();
        }
    }

    pub fn live_text(&self, page_id: PageId) -> String {
        self.docs[&page_id].plain_text().unwrap()
    }

    pub fn stored_text(&self, page_id: PageId) -> String {
        self.store
            .load_page_doc(page_id)
            .unwrap()
            .plain_text()
            .unwrap()
    }

    pub fn is_settled(&self) -> bool {
        self.store.queued_change_count().unwrap() == 0
    }

    /// Applies waiting events the way the app would.
    pub fn pump(&mut self) {
        let Some(events) = &mut self.events else {
            return;
        };
        while let Some(event) = events.try_next() {
            match &event {
                SyncEvent::Status(status) => self.statuses.push(*status),
                SyncEvent::PageCatchUp { page_id, catch_up } => {
                    if let Some(doc) = self.docs.get(page_id) {
                        if let Some(snapshot) = &catch_up.snapshot {
                            doc.import(&snapshot.bytes).unwrap();
                        }
                        for update in &catch_up.updates {
                            doc.import(&update.bytes).unwrap();
                        }
                    }
                }
                SyncEvent::RemotePageUpdate { page_id, update } => {
                    if let Some(doc) = self.docs.get(page_id) {
                        doc.import(&update.bytes).unwrap();
                    }
                }
                SyncEvent::PageEditsRefused { page_id, .. } => {
                    if self.docs.contains_key(page_id) {
                        let reloaded = self.store.load_page_doc(*page_id).unwrap();
                        self.docs.insert(*page_id, reloaded);
                    }
                }
                // Expected when a test checks certificate failures; any other error fails the test.
                SyncEvent::Error(SyncError::ServerNotTrusted(_)) => {}
                SyncEvent::Error(error) => panic!("the engine reported an error: {error}"),
                _ => {}
            }
            self.log.push(event);
        }
    }

    pub fn status(&self) -> Option<ConnectionStatus> {
        self.statuses.last().copied()
    }
}

/// Pumps every app's events until `done` holds for them.
///
/// The engine writes to the store before it reports the change, so a condition on the store alone can hold
/// while the event saying so is still on its way: a test that asserts on `log` or `statuses` waits for them.
pub async fn wait_until(apps: &mut [&mut TestApp], mut done: impl FnMut(&[&mut TestApp]) -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT_LIMIT;
    loop {
        for app in apps.iter_mut() {
            app.pump();
        }
        if done(apps) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for the apps to reach the expected state"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

pub async fn wait_for_status(app: &mut TestApp, status: ConnectionStatus) {
    wait_until(&mut [app], |apps| apps[0].status() == Some(status)).await;
}

pub fn new_page(title: &str, parent: Option<PageId>) -> Page {
    Page {
        id: PageId::generate(),
        parent,
        title: title.to_owned(),
        icon: None,
        position: Position::between(None, None).unwrap(),
        row_of: None,
        is_archived: false,
    }
}

/// A TCP relay between engines and the server that can cut every connection through it at once.
pub struct Proxy {
    pub address: SocketAddr,
    cut: broadcast::Sender<()>,
}

impl Proxy {
    pub async fn start(upstream: SocketAddr) -> Proxy {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (cut, _) = broadcast::channel(16);
        let cut_sender = cut.clone();
        tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                tokio::spawn(relay(client, upstream, cut_sender.subscribe()));
            }
        });
        Proxy { address, cut }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// Drops every connection open through the proxy, as a network failure would.
    pub fn cut_connections(&self) {
        let _ = self.cut.send(());
    }
}

async fn relay(mut client: TcpStream, upstream: SocketAddr, mut cut: broadcast::Receiver<()>) {
    let Ok(mut server) = TcpStream::connect(upstream).await else {
        return;
    };
    tokio::select! {
        _ = tokio::io::copy_bidirectional(&mut client, &mut server) => {}
        _ = cut.recv() => {}
    }
}

/// The workspace's records as a sorted list, for comparing two stores.
pub fn sorted_records(store: &WorkspaceStore) -> Vec<String> {
    let snapshot = store.export_workspace_snapshot().unwrap();
    let mut records: Vec<String> = snapshot
        .records
        .iter()
        .map(|record| format!("{record:?}"))
        .chain(
            snapshot
                .property_values
                .iter()
                .map(|value| format!("{value:?}")),
        )
        .collect();
    records.sort();
    records
}
