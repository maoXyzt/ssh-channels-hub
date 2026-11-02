use bytes::Bytes;
use http_body_util::{BodyExt, Full, combinators::UnsyncBoxBody};
use hyper::body::Incoming;
use hyper::header::{
  CONNECTION, HOST, HeaderMap, HeaderName, HeaderValue, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION,
  TE, TRAILER, TRANSFER_ENCODING, UPGRADE,
};
use hyper::http::uri::{Authority, PathAndQuery, Scheme};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode, Uri, Version};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::client::legacy::{Client, connect::HttpConnector};
use hyper_util::rt::{TokioExecutor, TokioIo};
use std::convert::Infallible;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tracing::warn;

const UPSTREAM_HEADER_TIMEOUT: Duration = Duration::from_secs(300);

type BoxError = Box<dyn std::error::Error + Send + Sync>;
type GatewayBody = UnsyncBoxBody<Bytes, BoxError>;
type GatewayClient = Client<HttpsConnector<HttpConnector>, GatewayBody>;

#[derive(Clone)]
pub struct HttpGateway {
  scheme: Scheme,
  authority: Authority,
  host_header: HeaderValue,
  upstream_label: String,
  client: GatewayClient,
}

impl HttpGateway {
  pub fn new(host: &str, port: u16) -> Result<Self, String> {
    let connector = HttpsConnectorBuilder::new()
      .with_native_roots()
      .map_err(|error| format!("Failed to load native TLS roots: {error}"))?
      .https_only()
      .enable_http1()
      .build();
    Self::with_connector(host, port, Scheme::HTTPS, connector)
  }

  #[cfg(test)]
  fn new_http_for_test(host: &str, port: u16) -> Self {
    let connector = HttpsConnectorBuilder::new()
      .with_native_roots()
      .unwrap()
      .https_or_http()
      .enable_http1()
      .build();
    Self::with_connector(host, port, Scheme::HTTP, connector).unwrap()
  }

  fn with_connector(
    host: &str,
    port: u16,
    scheme: Scheme,
    connector: HttpsConnector<HttpConnector>,
  ) -> Result<Self, String> {
    let authority_text = if host.contains(':') {
      format!("[{host}]:{port}")
    } else {
      format!("{host}:{port}")
    };
    let authority = authority_text
      .parse::<Authority>()
      .map_err(|error| format!("Invalid HTTP gateway authority '{authority_text}': {error}"))?;
    let host_text = if host.contains(':') {
      format!("[{host}]")
    } else {
      host.to_string()
    };
    let is_default_port = matches!((scheme.as_str(), port), ("https", 443) | ("http", 80));
    let host_value = if is_default_port {
      host_text
    } else {
      authority_text.clone()
    };
    let host_header = HeaderValue::from_str(&host_value)
      .map_err(|error| format!("Invalid HTTP gateway Host header: {error}"))?;
    let client = Client::builder(TokioExecutor::new()).build(connector);
    Ok(Self {
      scheme,
      authority,
      host_header,
      upstream_label: authority_text,
      client,
    })
  }

  pub async fn serve_connection<S>(
    &self,
    stream: S,
    channel_name: String,
  ) -> Result<(), hyper::Error>
  where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
  {
    let gateway = self.clone();
    hyper::server::conn::http1::Builder::new()
      .serve_connection(
        TokioIo::new(stream),
        service_fn(move |request| {
          let gateway = gateway.clone();
          let channel_name = channel_name.clone();
          async move { gateway.handle(request, &channel_name).await }
        }),
      )
      .with_upgrades()
      .await
  }

  /// Forwards HTTP bodies unchanged, switching to a byte relay only after a
  /// supported upstream WebSocket handshake. TLS and the target stay fixed.
  async fn handle(
    &self,
    mut request: Request<Incoming>,
    channel_name: &str,
  ) -> Result<Response<GatewayBody>, Infallible> {
    // Hyper provides the downstream upgrade handoff only for HTTP/1.1.
    let is_websocket = request.version() == Version::HTTP_11
      && has_header_token(request.headers(), CONNECTION, "upgrade")
      && has_header_token(request.headers(), UPGRADE, "websocket");
    if request.method() == Method::CONNECT
      || (request.headers().contains_key(UPGRADE) && !is_websocket)
    {
      return Ok(text_response(
        StatusCode::BAD_REQUEST,
        "HTTP gateway supports HTTP/1.1 and WebSocket requests\n",
      ));
    }

    let path_and_query = request
      .uri()
      .path_and_query()
      .cloned()
      .unwrap_or_else(|| PathAndQuery::from_static("/"));
    let uri = match Uri::builder()
      .scheme(self.scheme.clone())
      .authority(self.authority.clone())
      .path_and_query(path_and_query)
      .build()
    {
      Ok(uri) => uri,
      Err(error) => {
        warn!(channel = %channel_name, error = %error, "Invalid HTTP gateway request target");
        return Ok(text_response(
          StatusCode::BAD_REQUEST,
          "Invalid HTTP gateway request target\n",
        ));
      }
    };

    let downstream_upgrade = is_websocket.then(|| hyper::upgrade::on(&mut request));
    *request.uri_mut() = uri;
    strip_hop_by_hop_headers(request.headers_mut());
    if is_websocket {
      request
        .headers_mut()
        .insert(CONNECTION, HeaderValue::from_static("upgrade"));
      request
        .headers_mut()
        .insert(UPGRADE, HeaderValue::from_static("websocket"));
    }
    request.headers_mut().insert(HOST, self.host_header.clone());
    let (parts, body) = request.into_parts();
    let request = Request::from_parts(parts, incoming_body(body));

    match tokio::time::timeout(UPSTREAM_HEADER_TIMEOUT, self.client.request(request)).await {
      Ok(Ok(mut response)) => {
        if response.status() == StatusCode::SWITCHING_PROTOCOLS {
          if !is_websocket
            || !has_header_token(response.headers(), CONNECTION, "upgrade")
            || !is_websocket_response(response.headers())
          {
            return Ok(text_response(
              StatusCode::BAD_GATEWAY,
              "Invalid upstream WebSocket upgrade\n",
            ));
          }
          let upstream_upgrade = hyper::upgrade::on(&mut response);
          let downstream_upgrade = downstream_upgrade.expect("WebSocket request has an upgrade");
          let channel_name = channel_name.to_string();
          tokio::spawn(async move {
            let result = async {
              let (downstream, upstream) = tokio::try_join!(downstream_upgrade, upstream_upgrade)?;
              tokio::io::copy_bidirectional(
                &mut TokioIo::new(downstream),
                &mut TokioIo::new(upstream),
              )
              .await?;
              Ok::<_, BoxError>(())
            }
            .await;
            if let Err(error) = result {
              warn!(channel = %channel_name, error = %error, "WebSocket gateway relay failed");
            }
          });
          let (mut parts, body) = response.into_parts();
          strip_hop_by_hop_headers(&mut parts.headers);
          parts
            .headers
            .insert(CONNECTION, HeaderValue::from_static("upgrade"));
          parts
            .headers
            .insert(UPGRADE, HeaderValue::from_static("websocket"));
          return Ok(Response::from_parts(parts, incoming_body(body)));
        }
        let (mut parts, body) = response.into_parts();
        strip_hop_by_hop_headers(&mut parts.headers);
        Ok(Response::from_parts(parts, incoming_body(body)))
      }
      Ok(Err(error)) => {
        warn!(
          channel = %channel_name,
          upstream = %self.upstream_label,
          error = %error,
          "HTTP gateway upstream request failed"
        );
        Ok(text_response(
          StatusCode::BAD_GATEWAY,
          "HTTP gateway upstream request failed\n",
        ))
      }
      Err(_) => {
        warn!(
          channel = %channel_name,
          upstream = %self.upstream_label,
          "HTTP gateway upstream response headers timed out"
        );
        Ok(text_response(
          StatusCode::GATEWAY_TIMEOUT,
          "HTTP gateway upstream response headers timed out\n",
        ))
      }
    }
  }
}

fn incoming_body(body: Incoming) -> GatewayBody {
  body
    .map_err(|error| Box::new(error) as BoxError)
    .boxed_unsync()
}

fn text_response(status: StatusCode, message: &'static str) -> Response<GatewayBody> {
  let body = Full::new(Bytes::from_static(message.as_bytes()))
    .map_err(|error| -> BoxError { match error {} })
    .boxed_unsync();
  Response::builder()
    .status(status)
    .body(body)
    .expect("static HTTP gateway error response is valid")
}

/// A request may offer several upgrade protocols across comma-separated fields.
fn has_header_token(headers: &HeaderMap, name: HeaderName, token: &str) -> bool {
  headers
    .get_all(name)
    .iter()
    .filter_map(|value| value.to_str().ok())
    .flat_map(|value| value.split(','))
    .any(|value| value.trim().eq_ignore_ascii_case(token))
}

/// A response selects the actual protocol, rather than offering alternatives.
/// Reject extra values or repeated fields: rewriting a protocol stack to just
/// WebSocket would misrepresent what the upstream selected.
fn is_websocket_response(headers: &HeaderMap) -> bool {
  let mut values = headers.get_all(UPGRADE).iter();
  values
    .next()
    .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"websocket"))
    && values.next().is_none()
}

fn strip_hop_by_hop_headers(headers: &mut HeaderMap) {
  let nominated = headers
    .get_all(CONNECTION)
    .iter()
    .filter_map(|value| value.to_str().ok())
    .flat_map(|value| value.split(','))
    .filter_map(|name| HeaderName::from_bytes(name.trim().as_bytes()).ok())
    .collect::<Vec<_>>();
  for name in nominated {
    headers.remove(name);
  }

  for name in [
    CONNECTION,
    PROXY_AUTHENTICATE,
    PROXY_AUTHORIZATION,
    TE,
    TRAILER,
    TRANSFER_ENCODING,
    UPGRADE,
  ] {
    headers.remove(name);
  }
  headers.remove("keep-alive");
  headers.remove("proxy-connection");
}

#[cfg(test)]
mod tests {
  use super::*;
  use hyper::header::AUTHORIZATION;
  use hyper_util::rt::TokioIo;
  use tokio::io::{AsyncReadExt, AsyncWriteExt};
  use tokio::net::TcpListener;
  use tokio::sync::{mpsc, oneshot};
  use tokio::task::JoinHandle;

  trait AsyncReadWrite: AsyncRead + AsyncWrite + Unpin + Send {}
  impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncReadWrite for T {}

  type TestClient = hyper::client::conn::http1::SendRequest<Full<Bytes>>;

  #[derive(Debug)]
  struct RecordedRequest {
    method: Method,
    uri: String,
    host: Option<String>,
    authorization: Option<String>,
    connection: Option<String>,
    nominated_header: Option<String>,
    body: Bytes,
  }

  async fn connect_gateway(port: u16) -> (TestClient, JoinHandle<()>, JoinHandle<()>) {
    let gateway = HttpGateway::new_http_for_test("127.0.0.1", port);
    let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
    let gateway_task = tokio::spawn(async move {
      let _ = gateway
        .serve_connection(server_stream, "test-gateway".to_string())
        .await;
    });
    let (sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(client_stream))
      .await
      .unwrap();
    let connection_task = tokio::spawn(async move {
      let _ = connection.await;
    });
    (sender, gateway_task, connection_task)
  }

  async fn spawn_upstream() -> (
    u16,
    mpsc::UnboundedReceiver<RecordedRequest>,
    JoinHandle<()>,
  ) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (record_tx, record_rx) = mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
      loop {
        let (stream, _) = listener.accept().await.unwrap();
        let record_tx = record_tx.clone();
        tokio::spawn(async move {
          let service = service_fn(move |request: Request<Incoming>| {
            let record_tx = record_tx.clone();
            async move {
              let (parts, body) = request.into_parts();
              let body = body.collect().await.unwrap().to_bytes();
              let header = |name: HeaderName| {
                parts
                  .headers
                  .get(name)
                  .and_then(|value| value.to_str().ok())
                  .map(str::to_string)
              };
              record_tx
                .send(RecordedRequest {
                  method: parts.method,
                  uri: parts.uri.to_string(),
                  host: header(HOST),
                  authorization: header(AUTHORIZATION),
                  connection: header(CONNECTION),
                  nominated_header: header(HeaderName::from_static("x-remove-me")),
                  body,
                })
                .unwrap();
              Ok::<_, Infallible>(Response::new(Full::new(Bytes::from_static(b"ok"))))
            }
          });
          let _ = hyper::server::conn::http1::Builder::new()
            .serve_connection(TokioIo::new(stream), service)
            .await;
        });
      }
    });
    (port, record_rx, task)
  }

  #[tokio::test]
  async fn rewrites_gateway_requests_and_reuses_connections() {
    let (port, mut records, upstream_task) = spawn_upstream().await;
    let (mut client, gateway_task, connection_task) = connect_gateway(port).await;

    let request = Request::builder()
      .method(Method::POST)
      .uri("/v1/chat/completions?stream=true")
      .header(HOST, "127.0.0.1:5443")
      .header(AUTHORIZATION, "Bearer test-token")
      .header(CONNECTION, "keep-alive, x-remove-me")
      .header("x-remove-me", "must-not-reach-upstream")
      .body(Full::new(Bytes::from_static(b"{\"input\":true}")))
      .unwrap();
    let response = client.send_request(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
      response.into_body().collect().await.unwrap().to_bytes(),
      Bytes::from_static(b"ok")
    );

    let first = records.recv().await.unwrap();
    assert_eq!(first.method, Method::POST);
    assert_eq!(first.uri, "/v1/chat/completions?stream=true");
    assert_eq!(
      first.host.as_deref(),
      Some(format!("127.0.0.1:{port}").as_str())
    );
    assert_eq!(first.authorization.as_deref(), Some("Bearer test-token"));
    assert!(first.connection.is_none());
    assert!(first.nominated_header.is_none());
    assert_eq!(first.body, Bytes::from_static(b"{\"input\":true}"));

    let request = Request::builder()
      .uri("/v1/models")
      .body(Full::new(Bytes::new()))
      .unwrap();
    let response = client.send_request(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let second = records.recv().await.unwrap();
    assert_eq!(second.uri, "/v1/models");

    upstream_task.abort();
    gateway_task.abort();
    connection_task.abort();
  }

  #[tokio::test]
  async fn streams_upstream_chunks_before_the_response_completes() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (release_tx, release_rx) = oneshot::channel();
    let upstream_task = tokio::spawn(async move {
      let (mut stream, _) = listener.accept().await.unwrap();
      let mut request = [0; 1024];
      let _ = stream.read(&mut request).await.unwrap();
      stream
        .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nfirst\r\n")
        .await
        .unwrap();
      release_rx.await.unwrap();
      stream.write_all(b"6\r\nsecond\r\n0\r\n\r\n").await.unwrap();
    });

    let (mut client, gateway_task, connection_task) = connect_gateway(port).await;
    let request = Request::builder()
      .uri("/v1/stream")
      .body(Full::new(Bytes::new()))
      .unwrap();
    let response = client.send_request(request).await.unwrap();
    let mut body = response.into_body();
    let first = tokio::time::timeout(Duration::from_secs(1), body.frame())
      .await
      .expect("first chunk must arrive before upstream completes")
      .unwrap()
      .unwrap()
      .into_data()
      .unwrap();
    assert_eq!(first, Bytes::from_static(b"first"));

    release_tx.send(()).unwrap();
    let remainder = body.collect().await.unwrap().to_bytes();
    assert_eq!(remainder, Bytes::from_static(b"second"));

    upstream_task.await.unwrap();
    gateway_task.abort();
    connection_task.abort();
  }

  #[tokio::test]
  async fn upstream_connection_failure_returns_bad_gateway() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let (mut client, gateway_task, connection_task) = connect_gateway(port).await;
    let request = Request::builder()
      .uri("/v1/models")
      .body(Full::new(Bytes::new()))
      .unwrap();

    let response = client.send_request(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

    gateway_task.abort();
    connection_task.abort();
  }

  #[tokio::test]
  async fn rejects_connect_and_unsupported_upgrades() {
    let (mut client, gateway_task, connection_task) = connect_gateway(1).await;
    for (method, uri, upgrade) in [
      (Method::CONNECT, "example.com:443", None),
      (Method::GET, "/", Some("h2c")),
      (Method::GET, "/", Some("websocket")),
    ] {
      let mut request = Request::builder().method(method).uri(uri);
      if let Some(upgrade) = upgrade {
        request = request.header(UPGRADE, upgrade);
      }
      let response = client
        .send_request(request.body(Full::new(Bytes::new())).unwrap())
        .await
        .unwrap();
      assert_eq!(response.status(), StatusCode::BAD_REQUEST);
      response.into_body().collect().await.unwrap();
    }
    gateway_task.abort();
    connection_task.abort();
  }

  #[tokio::test]
  async fn websocket_upgrade_relays_frames_and_buffered_bytes() {
    for upgrade in [
      "Upgrade: websocket",
      "Upgrade: h2c, WebSocket",
      "Upgrade:   WeBsOcKeT  ",
      "Upgrade: h2c\r\nUpgrade: WebSocket",
    ] {
      check_websocket_relay(upgrade, false).await;
    }
  }

  #[tokio::test]
  async fn websocket_upgrade_over_tls_checks_host_and_sni() {
    check_websocket_relay("Upgrade: websocket", true).await;
  }

  #[tokio::test]
  async fn rejects_http10_upgrade_before_contacting_upstream() {
    tokio::time::timeout(Duration::from_secs(5), async {
      let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
      let gateway =
        HttpGateway::new_http_for_test("127.0.0.1", listener.local_addr().unwrap().port());
      let (mut client, server) = tokio::io::duplex(4096);
      let task = tokio::spawn(async move {
        gateway
          .serve_connection(server, "http10".into())
          .await
          .unwrap()
      });
      client
        .write_all(b"GET / HTTP/1.0\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n")
        .await
        .unwrap();
      let mut response = Vec::new();
      client.read_to_end(&mut response).await.unwrap();
      assert!(
        String::from_utf8(response)
          .unwrap()
          .starts_with("HTTP/1.0 400")
      );
      task.await.unwrap();
      assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
          .await
          .is_err()
      );
    })
    .await
    .unwrap();
  }

  async fn check_websocket_relay(upgrade: &str, use_tls: bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
      let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
      let port = listener.local_addr().unwrap().port();
      let certificate = tokio_rustls::rustls::pki_types::CertificateDer::from(include_bytes!("../tests/fixtures/tls/localhost.der").to_vec());
      let mut roots = tokio_rustls::rustls::RootCertStore::empty();
      roots.add(certificate.clone()).unwrap();
      let tls_config = tokio_rustls::rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], tokio_rustls::rustls::pki_types::PrivatePkcs8KeyDer::from(include_bytes!("../tests/fixtures/tls/localhost-key.der").to_vec()).into()).unwrap();
      let upstream_task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut stream: Box<dyn AsyncReadWrite> = if use_tls {
          let stream = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(tls_config)).accept(stream).await.unwrap();
          assert_eq!(stream.get_ref().1.server_name(), Some("localhost"));
          Box::new(stream)
        } else { Box::new(stream) };
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
          request.push(stream.read_u8().await.unwrap());
        }
        let request = String::from_utf8(request).unwrap().to_ascii_lowercase();
        assert!(request.starts_with("get /events?token=1 http/1.1"));
        if use_tls { assert!(request.contains(&format!("host: localhost:{port}\r\n"))); }
        assert!(request.contains("upgrade: websocket"));
        assert!(request.contains(&"sec-websocket-key: dGhlIHNhbXBsZSBub25jZQ==".to_ascii_lowercase()));
        assert!(!request.contains("x-remove-me:"));
        // Send a frame with the headers to exercise Hyper's upgrade read buffer.
        stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=\r\n\r\n\x81\x02hi").await.unwrap();
        let mut frame = [0; 8];
        stream.read_exact(&mut frame).await.unwrap();
        assert_eq!(&frame, b"\x81\x82\x01\x02\x03\x04ik");
      });
      let gateway = if use_tls {
        let config = tokio_rustls::rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
        let connector = HttpsConnectorBuilder::new().with_tls_config(config).https_only().enable_http1().build();
        HttpGateway::with_connector("localhost", port, Scheme::HTTPS, connector).unwrap()
      } else { HttpGateway::new_http_for_test("127.0.0.1", port) };
      let (mut client, server) = tokio::io::duplex(4096);
      let gateway_task = tokio::spawn(async move {
        gateway.serve_connection(server, "websocket-test".into()).await.unwrap();
      });
      let request = format!("GET /events?token=1 HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive, Upgrade, x-remove-me\r\n{upgrade}\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nX-Remove-Me: secret\r\n\r\n");
      client.write_all(request.as_bytes()).await.unwrap();
      let mut headers = Vec::new();
      while !headers.ends_with(b"\r\n\r\n") {
        headers.push(client.read_u8().await.unwrap());
      }
      let headers = String::from_utf8(headers).unwrap().to_ascii_lowercase();
      assert!(headers.starts_with("http/1.1 101"));
      assert!(headers.contains(&"sec-websocket-accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=".to_ascii_lowercase()));
      let mut frame = [0; 4];
      client.read_exact(&mut frame).await.unwrap();
      assert_eq!(&frame, b"\x81\x02hi");
      client.write_all(b"\x81\x82\x01\x02\x03\x04ik").await.unwrap();
      upstream_task.await.unwrap();
      drop(client);
      gateway_task.await.unwrap();
    }).await.expect("WebSocket relay must complete");
  }

  #[tokio::test]
  async fn validates_upstream_upgrade_and_preserves_http_rejections() {
    tokio::time::timeout(Duration::from_secs(5), async {
      for (status, headers, expected) in [
        (
          "101 Switching Protocols",
          "Connection: Upgrade\r\nUpgrade: h2c, websocket\r\n",
          StatusCode::BAD_GATEWAY,
        ),
        (
          "101 Switching Protocols",
          "Connection: Upgrade\r\nUpgrade: websocket\r\nUpgrade: h2c\r\n",
          StatusCode::BAD_GATEWAY,
        ),
        (
          "101 Switching Protocols",
          "Connection: Upgrade\r\n",
          StatusCode::BAD_GATEWAY,
        ),
        (
          "101 Switching Protocols",
          "Upgrade: websocket\r\n",
          StatusCode::BAD_GATEWAY,
        ),
        (
          "403 Forbidden",
          "Content-Length: 6\r\n",
          StatusCode::FORBIDDEN,
        ),
      ] {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let upstream_task = tokio::spawn(async move {
          let (mut stream, _) = listener.accept().await.unwrap();
          let mut request = Vec::new();
          while !request.ends_with(b"\r\n\r\n") {
            request.push(stream.read_u8().await.unwrap());
          }
          stream
            .write_all(format!("HTTP/1.1 {status}\r\n{headers}\r\ndenied").as_bytes())
            .await
            .unwrap();
        });
        let (mut client, gateway_task, connection_task) = connect_gateway(port).await;
        let request = Request::builder()
          .uri("/events")
          .header(CONNECTION, "Upgrade")
          .header(UPGRADE, "websocket")
          .body(Full::new(Bytes::new()))
          .unwrap();
        let response = client.send_request(request).await.unwrap();
        assert_eq!(response.status(), expected);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        if expected == StatusCode::FORBIDDEN {
          assert_eq!(&body[..], b"denied");
        }
        upstream_task.await.unwrap();
        gateway_task.abort();
        connection_task.abort();
      }
    })
    .await
    .expect("upstream response checks must complete");
  }

  #[test]
  fn host_header_omits_only_the_scheme_default_port() {
    let default_https = HttpGateway::new("gateway.internal", 443).unwrap();
    assert_eq!(default_https.host_header, "gateway.internal");
    assert_eq!(default_https.upstream_label, "gateway.internal:443");

    let custom_https = HttpGateway::new("gateway.internal", 8443).unwrap();
    assert_eq!(custom_https.host_header, "gateway.internal:8443");

    let default_https_ipv6 = HttpGateway::new("::1", 443).unwrap();
    assert_eq!(default_https_ipv6.host_header, "[::1]");

    let default_http = HttpGateway::new_http_for_test("gateway.internal", 80);
    assert_eq!(default_http.host_header, "gateway.internal");
  }
}
