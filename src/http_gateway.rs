use bytes::Bytes;
use http_body_util::{BodyExt, Full, combinators::UnsyncBoxBody};
use hyper::body::Incoming;
use hyper::header::{
  CONNECTION, HOST, HeaderMap, HeaderName, HeaderValue, PROXY_AUTHENTICATE, PROXY_AUTHORIZATION,
  TE, TRAILER, TRANSFER_ENCODING, UPGRADE,
};
use hyper::http::uri::{Authority, PathAndQuery, Scheme};
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode, Uri};
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
    let host_header = HeaderValue::from_str(&authority_text)
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
      .await
  }

  async fn handle(
    &self,
    mut request: Request<Incoming>,
    channel_name: &str,
  ) -> Result<Response<GatewayBody>, Infallible> {
    if request.method() == Method::CONNECT || request.headers().contains_key(UPGRADE) {
      return Ok(text_response(
        StatusCode::BAD_REQUEST,
        "HTTP gateway only supports ordinary HTTP/1.1 requests\n",
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

    *request.uri_mut() = uri;
    strip_hop_by_hop_headers(request.headers_mut());
    request.headers_mut().insert(HOST, self.host_header.clone());
    let (parts, body) = request.into_parts();
    let request = Request::from_parts(parts, incoming_body(body));

    match tokio::time::timeout(UPSTREAM_HEADER_TIMEOUT, self.client.request(request)).await {
      Ok(Ok(response)) => {
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
}
