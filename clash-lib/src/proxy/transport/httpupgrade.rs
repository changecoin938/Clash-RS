use async_trait::async_trait;
use bytes::BytesMut;
use http::Request;
use std::collections::HashMap;
use tokio::io::{AsyncRead, AsyncWrite};

use super::Transport;
use crate::{common::errors::map_io_error, proxy::AnyStream};

pub struct Client {
    pub host: String,
    pub path: http::uri::PathAndQuery,
    pub headers: HashMap<String, String>,
}

impl Client {
    pub fn new(host: String, path: http::uri::PathAndQuery, headers: HashMap<String, String>) -> Self {
        Self { host, path, headers }
    }

    fn req(&self) -> std::io::Result<Request<()>> {
        let uri = http::Uri::builder()
            .scheme("https")
            .authority(self.host.as_str())
            .path_and_query(self.path.clone())
            .build()
            .map_err(map_io_error)?;
        let mut request = Request::builder()
            .method("GET")
            .uri(uri)
            .version(http::Version::HTTP_11)
            .header("Connection", "Upgrade");
        // If caller didn't provide Upgrade header, default to "websocket"
        if !self.headers.keys().any(|k| k.eq_ignore_ascii_case("Upgrade")) {
            request = request.header("Upgrade", "websocket");
        }
        for (k, v) in self.headers.iter() {
            request = request.header(k.as_str(), v.as_str());
        }
        Ok(request.body(()).unwrap())
    }
}

#[async_trait]
impl Transport for Client {
    async fn proxy_stream(&self, stream: AnyStream) -> std::io::Result<AnyStream> {
        Ok(HttpUpgradeStream::new(stream, self.host.clone(), self.path.clone(), self.headers.clone()).into())
    }
}

pub struct HttpUpgradeStream {
    inner: AnyStream,
    host: String,
    path: http::uri::PathAndQuery,
    headers: HashMap<String, String>,

    first_request: bool,
    first_response: bool,
    read_buf: BytesMut,
}

impl HttpUpgradeStream {
    pub fn new(inner: AnyStream, host: String, path: http::uri::PathAndQuery, headers: HashMap<String, String>) -> Self {
        Self { inner, host, path, headers, first_request: true, first_response: true, read_buf: BytesMut::new() }
    }

    fn build_request(&self) -> std::io::Result<Vec<u8>> {
        let client = Client { host: self.host.clone(), path: self.path.clone(), headers: self.headers.clone() };
        let req = client.req()?;
        let mut buf = Vec::new();
        let method = req.method().as_str();
        let path = req.uri().path_and_query().map(|p| p.as_str()).unwrap_or("/");
        buf.extend_from_slice(method.as_bytes());
        buf.extend_from_slice(b" ");
        buf.extend_from_slice(path.as_bytes());
        buf.extend_from_slice(b" HTTP/1.1\r\n");
        for (k, v) in req.headers().iter() {
            buf.extend_from_slice(k.as_str().as_bytes());
            buf.extend_from_slice(b": ");
            buf.extend_from_slice(v.as_bytes());
            buf.extend_from_slice(b"\r\n");
        }
        // Ensure Host header exists
        if !req.headers().contains_key(http::header::HOST) {
            buf.extend_from_slice(b"Host: ");
            buf.extend_from_slice(self.host.as_bytes());
            buf.extend_from_slice(b"\r\n");
        }
        buf.extend_from_slice(b"\r\n");
        Ok(buf)
    }
}

impl AsyncWrite for HttpUpgradeStream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        let pin = self.get_mut();
        if pin.first_request {
            let req = pin.build_request()?;
            // No content-length; upgrade should be immediate; no body in handshake
            // We'll write user payload after handshake is acknowledged by first read
            pin.first_request = false;
            match tokio::io::AsyncWrite::poll_write(std::pin::Pin::new(&mut pin.inner), cx, &req) {
                std::task::Poll::Ready(Ok(_)) => return std::task::Poll::Ready(Ok(0)), // we didn't consume user buf yet
                std::task::Poll::Ready(Err(e)) => return std::task::Poll::Ready(Err(e)),
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }
        tokio::io::AsyncWrite::poll_write(std::pin::Pin::new(&mut pin.inner), cx, buf)
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        let pin = self.get_mut();
        tokio::io::AsyncWrite::poll_flush(std::pin::Pin::new(&mut pin.inner), cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        let pin = self.get_mut();
        tokio::io::AsyncWrite::poll_shutdown(std::pin::Pin::new(&mut pin.inner), cx)
    }
}

impl AsyncRead for HttpUpgradeStream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let pin = self.get_mut();
        if !pin.read_buf.is_empty() {
            let to_read = std::cmp::min(buf.remaining(), pin.read_buf.len());
            if to_read == 0 { return std::task::Poll::Pending; }
            let data = pin.read_buf.split_to(to_read);
            buf.put_slice(&data[..]);
            return std::task::Poll::Ready(Ok(()));
        }

        if pin.first_response {
            let mut b = [0; 20 * 1024];
            let mut b = tokio::io::ReadBuf::new(&mut b);
            match tokio::io::AsyncRead::poll_read(std::pin::Pin::new(&mut pin.inner), cx, &mut b) {
                std::task::Poll::Ready(rv) => match rv {
                    Ok(_) => {
                        let bytes = b.filled();
                        let needle = b"\r\n\r\n";
                        if let Some(idx) = bytes.windows(needle.len()).position(|w| w == needle) {
                            // Check 101 Switching Protocols (optional)
                            if !(bytes.starts_with(b"HTTP/1.1 101") || bytes.starts_with(b"HTTP/1.0 101") || bytes.starts_with(b"HTTP/1.1 200") ) {
                                return std::task::Poll::Ready(Err(std::io::Error::other("http upgrade failed")));
                            }
                            pin.first_response = false;
                            let body = &bytes[idx + 4..];
                            if !body.is_empty() {
                                let to_copy = std::cmp::min(body.len(), buf.remaining());
                                buf.put_slice(&body[..to_copy]);
                                if body.len() > to_copy { pin.read_buf.extend_from_slice(&body[to_copy..]); }
                            }
                            return std::task::Poll::Ready(Ok(()));
                        }
                        std::task::Poll::Ready(Err(std::io::Error::other("EOF")))
                    }
                    Err(e) => std::task::Poll::Ready(Err(e)),
                },
                std::task::Poll::Pending => std::task::Poll::Pending,
            }
        } else {
            tokio::io::AsyncRead::poll_read(std::pin::Pin::new(&mut pin.inner), cx, buf)
        }
    }
}

impl From<HttpUpgradeStream> for AnyStream { fn from(s: HttpUpgradeStream) -> Self { Box::new(s) } }


