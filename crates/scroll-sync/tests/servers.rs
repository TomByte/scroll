//! Checking an address before signing in, and several servers used side by side in one process.

mod common;

use common::{TempDir, TestApp, TestServer, wait_until};
use scroll_api::protocol::PROTOCOL_VERSION;
use scroll_core::id::WorkspaceId;
use scroll_sync::{AccountClient, ServerCheckError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A web server that answers every request with `body`, as something other than Scroll might.
async fn fake_server(body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
    format!("http://{address}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_address_is_checked_before_signing_in() {
    let server = TestServer::start().await;

    let info = AccountClient::new(&server.url(), None)
        .unwrap()
        .server_info()
        .await
        .unwrap();
    assert_eq!(info.name, "Scroll");
    assert_eq!(info.protocol_version, PROTOCOL_VERSION);
    assert!(info.sign_up_requires_invite);

    let wrong_path = AccountClient::new(&format!("{}/not-here", server.url()), None).unwrap();
    assert!(matches!(
        wrong_path.server_info().await,
        Err(ServerCheckError::NotAScrollServer)
    ));
    let other_product = fake_server("<html>a blog</html>").await;
    assert!(matches!(
        AccountClient::new(&other_product, None)
            .unwrap()
            .server_info()
            .await,
        Err(ServerCheckError::NotAScrollServer)
    ));
    let future_server = fake_server(
        r#"{"product":"scroll-server","name":"Future","version":"9.0.0","protocol_version":999,"sign_up_requires_invite":true}"#,
    )
    .await;
    assert!(matches!(
        AccountClient::new(&future_server, None)
            .unwrap()
            .server_info()
            .await,
        Err(ServerCheckError::IncompatibleProtocol { server: 999, .. })
    ));

    let closed_port = TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let nobody_home = AccountClient::new(&format!("http://{closed_port}"), None).unwrap();
    assert!(matches!(
        nobody_home.server_info().await,
        Err(ServerCheckError::Unreachable(_))
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn several_servers_are_used_side_by_side() {
    let first = TestServer::start().await;
    let second = TestServer::start().await;
    let files = TempDir::new();
    let mut apps = Vec::new();
    for (index, server) in [&first, &second].into_iter().enumerate() {
        let accounts = server.accounts();
        let session = server.new_user(&accounts, None).await;
        let membership = accounts
            .create_workspace(&session.session_token, WorkspaceId::generate(), "Team")
            .await
            .unwrap();
        let mut app = TestApp::join(
            files.file(&format!("{index}.scroll")),
            &server.url(),
            &membership,
        );
        app.start(&session.session_token);
        apps.push(app);
    }
    let [first_app, second_app] = &mut apps[..] else {
        unreachable!("two apps were made");
    };

    let first_page = first_app.create_page("On the first server");
    let second_page = second_app.create_page("On the second server");

    wait_until(&mut [first_app, second_app], |apps| {
        apps.iter().all(|app| app.is_settled())
    })
    .await;
    assert!(first_app.store.page(second_page.id).unwrap().is_none());
    assert!(second_app.store.page(first_page.id).unwrap().is_none());
}
