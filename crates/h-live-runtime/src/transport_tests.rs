use crate::{
    net,
    proxy::{ProxyRecord, Route},
    transport::Transport,
};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_util::sync::CancellationToken;

async fn header(socket: &mut tokio::net::TcpStream) -> String {
    let mut bytes = vec![];
    let mut next = [0];
    while !bytes.ends_with(b"\r\n\r\n") {
        assert!(bytes.len() < 16000);
        socket.read_exact(&mut next).await.unwrap();
        bytes.push(next[0]);
    }
    String::from_utf8(bytes).unwrap()
}
fn local_proxy(address: std::net::SocketAddr) -> Arc<Route> {
    // Deliberate test-only bypass: production imports reject loopback proxies.
    let mut record =
        ProxyRecord::from_line("8.8.8.8:8080:fixture:synthetic-secret", "http").unwrap();
    record.endpoint = address.to_string();
    Route::new(Some(record)).unwrap()
}
#[tokio::test]
async fn authenticated_connect_is_bounded_and_never_sends_guest_cookie_to_proxy() {
    let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let route = local_proxy(server.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = server.accept().await.unwrap();
        let request = header(&mut socket).await;
        assert!(request.starts_with("CONNECT webcast100-ws-web-lq.douyin.com:443 HTTP/1.1"));
        assert!(request.contains("Proxy-Authorization: Basic "));
        assert!(!request.contains("ttwid"));
        socket
            .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
    });
    let visitor = net::Visitor {
        cookie: "ttwid=synthetic-visitor".into(),
        did: "333".into(),
    };
    let result = net::connect(route, &visitor, "222", &CancellationToken::new()).await;
    assert!(matches!(result, Err(net::Error::Status(407))));
    task.await.unwrap();
}
#[tokio::test]
async fn websocket_rejects_untrusted_tls_after_connect() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certificate = rcgen::generate_simple_self_signed(vec!["wrong.invalid".into()]).unwrap();
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(certificate.signing_key.serialize_der());
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate.cert.der().clone()], key.into())
        .unwrap();
    let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let route = local_proxy(server.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = server.accept().await.unwrap();
        let _ = header(&mut socket).await;
        socket
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .unwrap();
        let result = tokio_rustls::TlsAcceptor::from(Arc::new(config))
            .accept(socket)
            .await;
        assert!(result.is_err());
    });
    let visitor = net::Visitor {
        cookie: "ttwid=synthetic-visitor".into(),
        did: "333".into(),
    };
    let result = tokio::time::timeout(
        Duration::from_secs(8),
        net::connect(route, &visitor, "222", &CancellationToken::new()),
    )
    .await
    .unwrap();
    assert!(matches!(result, Err(net::Error::Network)));
    task.await.unwrap();
}
#[tokio::test]
async fn http_rejects_untrusted_tls_after_connect() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certificate = rcgen::generate_simple_self_signed(vec!["wrong.invalid".into()]).unwrap();
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(certificate.signing_key.serialize_der());
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate.cert.der().clone()], key.into())
        .unwrap();
    let server = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let route = local_proxy(server.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = server.accept().await.unwrap();
        let _ = header(&mut socket).await;
        socket
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await
            .unwrap();
        assert!(tokio_rustls::TlsAcceptor::from(Arc::new(config))
            .accept(socket)
            .await
            .is_err());
    });
    assert!(route
        .client
        .get("https://live.douyin.com/")
        .send()
        .await
        .is_err());
    task.await.unwrap();
}
#[tokio::test]
async fn subscriptions_refuse_internal_or_non_https_targets() {
    for u in [
        "http://example.com/",
        "https://127.0.0.1/",
        "https://169.254.169.254/",
        "https://user:synthetic-secret@example.com/",
        "https://example.com:8443/",
    ] {
        assert!(
            crate::proxy::fetch_subscription(u, &CancellationToken::new())
                .await
                .is_err()
        );
    }
}
#[cfg(feature = "test-support")]
#[tokio::test]
async fn synthetic_packets_include_a_decodable_gift_before_collection_filter() {
    let simulation = crate::testing::Simulation::default();
    let route = Route::new(None).unwrap();
    let token = CancellationToken::new();
    let visitor = simulation.visitor(&route, &token).await.unwrap();
    let mut socket = simulation
        .connect(route, &visitor, "222", &token)
        .await
        .unwrap();
    let frame = socket.read_frame(&token).await.unwrap().unwrap();
    let decoded = qingkan_core::protocol::decode_frame(&frame, "222").unwrap();
    assert_eq!(
        decoded.events.iter().map(|e| e.kind).collect::<Vec<_>>(),
        vec!["chat", "gift"]
    );
}
