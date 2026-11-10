//! The account client and the engine over `https` and `wss`, through a TLS-terminating front like the reverse
//! proxy a deployment has, with a certificate from a test authority made for each run.

mod common;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;

use common::{TempDir, TestApp, TestServer, wait_for_status};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, SanType,
};
use scroll_core::id::WorkspaceId;
use scroll_sync::{
    AccountClient, AccountError, ConnectionStatus, NewAccount, SyncError, SyncEvent,
};
use time::{Duration, OffsetDateTime};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::rustls::crypto::ring;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

/// Terminates TLS and relays the plain bytes to the server.
struct TlsFront {
    address: SocketAddr,
    /// The authority that signed the front's certificate, in PEM. Not trusted by the system.
    root_certificate: String,
}

impl TlsFront {
    async fn start(upstream: SocketAddr) -> TlsFront {
        let (root_certificate, certificate, key) = certificates();
        let config = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![certificate], key)
            .unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((client, _)) = listener.accept().await {
                tokio::spawn(relay(acceptor.clone(), client, upstream));
            }
        });
        TlsFront {
            address,
            root_certificate,
        }
    }

    fn url(&self) -> String {
        format!("https://localhost:{}", self.address.port())
    }

    fn root(&self) -> Option<&[u8]> {
        Some(self.root_certificate.as_bytes())
    }
}

/// A failed handshake (an untrusting client) just ends the connection.
async fn relay(acceptor: TlsAcceptor, client: TcpStream, upstream: SocketAddr) {
    let Ok(mut client) = acceptor.accept(client).await else {
        return;
    };
    let Ok(mut server) = TcpStream::connect(upstream).await else {
        return;
    };
    let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
}

/// A test authority's certificate (PEM), and a server certificate for `localhost` it signed with its key.
/// Short-lived, as Apple's platform requires of server certificates.
fn certificates() -> (String, CertificateDer<'static>, PrivateKeyDer<'static>) {
    let now = OffsetDateTime::now_utc();
    let authority_key = KeyPair::generate().unwrap();
    let mut authority = CertificateParams::new(Vec::<String>::new()).unwrap();
    authority.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    authority.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    authority
        .distinguished_name
        .push(DnType::CommonName, "Scroll test authority");
    authority.not_before = now - Duration::days(1);
    authority.not_after = now + Duration::days(30);
    let authority_certificate = authority.self_signed(&authority_key).unwrap();
    let issuer = Issuer::new(authority, authority_key);

    let server_key = KeyPair::generate().unwrap();
    let mut server = CertificateParams::new(vec!["localhost".to_owned()]).unwrap();
    server
        .subject_alt_names
        .push(SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)));
    server
        .distinguished_name
        .push(DnType::CommonName, "localhost");
    server.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    server.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    server.not_before = now - Duration::days(1);
    server.not_after = now + Duration::days(30);
    let server_certificate = server.signed_by(&server_key, &issuer).unwrap();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key.serialize_der()));
    (
        authority_certificate.pem(),
        server_certificate.der().clone(),
        key,
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accounts_and_sync_work_over_tls_with_a_trusted_root() {
    let server = TestServer::start().await;
    let front = TlsFront::start(server.address).await;
    let accounts = AccountClient::new(&front.url(), front.root()).unwrap();
    let session = server.new_user(&accounts, None).await;
    let membership = accounts
        .create_workspace(&session.session_token, WorkspaceId::generate(), "Secure")
        .await
        .unwrap();
    let files = TempDir::new();
    let mut app = TestApp::join(files.file("secure.scroll"), &front.url(), &membership);

    app.start_trusting(&session.session_token, front.root());
    app.create_page("Sent over wss");

    wait_for_status(&mut app, ConnectionStatus::Online).await;
    common::wait_until(&mut [&mut app], |apps| apps[0].is_settled()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_untrusted_certificate_is_a_clear_error_not_an_endless_retry() {
    let server = TestServer::start().await;
    let front = TlsFront::start(server.address).await;
    let trusting = AccountClient::new(&front.url(), front.root()).unwrap();
    let session = server.new_user(&trusting, None).await;
    let membership = trusting
        .create_workspace(&session.session_token, WorkspaceId::generate(), "Secure")
        .await
        .unwrap();

    let untrusting = AccountClient::new(&front.url(), None).unwrap();
    let account = NewAccount {
        email: "nobody@example.test".into(),
        password: common::PASSWORD.into(),
        invite_code: Some("unused".into()),
    };
    let refused = untrusting.sign_up(account).await;
    assert!(
        matches!(refused, Err(AccountError::ServerNotTrusted(_))),
        "{refused:?}"
    );

    let files = TempDir::new();
    let mut app = TestApp::join(files.file("secure.scroll"), &front.url(), &membership);
    app.start(&session.session_token);
    wait_for_status(&mut app, ConnectionStatus::ServerNotTrusted).await;
    assert!(
        app.log
            .iter()
            .any(|event| matches!(event, SyncEvent::Error(SyncError::ServerNotTrusted(_))))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checking_an_untrusted_server_says_so() {
    let server = TestServer::start().await;
    let front = TlsFront::start(server.address).await;

    let untrusting = AccountClient::new(&front.url(), None).unwrap();
    let trusting = AccountClient::new(&front.url(), front.root()).unwrap();

    assert!(matches!(
        untrusting.server_info().await,
        Err(scroll_sync::ServerCheckError::ServerNotTrusted(_))
    ));
    assert!(trusting.server_info().await.is_ok());
}
