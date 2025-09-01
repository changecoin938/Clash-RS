use async_trait::async_trait;
use bytes::{BufMut, BytesMut};
use http::{Method, Uri};
use std::{collections::HashMap, pin::Pin};
use tokio::io::{AsyncRead, AsyncWrite};

use super::Transport;
use crate::{common::errors::map_io_error, proxy::AnyStream};

pub struct Client {
    pub host: String,
    pub path: http::uri::PathAndQuery,
    pub method: Method,
    pub headers: HashMap<String, String>,
}

impl Client {
    pub fn new(
        host: String,
        path: http::uri::PathAndQuery,
        method: Method,
        headers: HashMap<String, String>,
    ) -> Self {
        Self {
            host,
            path,
            method,
            headers,
        }
    }

    fn build_request_prefix(&self, initial_body_len: usize) -> std::io::Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(512);

        // Build request line
        let path = self
            .path
            .as_str()
            .parse::<Uri>()
            .map_err(map_io_error)?;
        let path = path.path_and_query().map(|p| p.as_str()).unwrap_or("/");
        buf.extend_from_slice(self.method.as_str().as_bytes());
        buf.extend_from_slice(b" ");
        buf.extend_from_slice(path.as_bytes());
        buf.extend_from_slice(b" HTTP/1.1\r\n");

        // Host header
        if let Some(host_hdr) = self.headers.get("Host") {
            buf.extend_from_slice(b"Host: ");
            buf.extend_from_slice(host_hdr.as_bytes());
            buf.extend_from_slice(b"\r\n");
        } else {
            buf.extend_from_slice(b"Host: ");
            buf.extend_from_slice(self.host.as_bytes());
            buf.extend_from_slice(b"\r\n");
        }

        // User headers
        for (k, v) in self.headers.iter() {
            if k.eq_ignore_ascii_case("Content-Length") || k.eq_ignore_ascii_case("Transfer-Encoding") {
                // handled below
                continue;
            }
            // Avoid duplicating Host as we added it above
            if k.eq_ignore_ascii_case("Host") {
                continue;
            }
            buf.extend_from_slice(k.as_bytes());
            buf.extend_from_slice(b": ");
            buf.extend_from_slice(v.as_bytes());
            buf.extend_from_slice(b"\r\n");
        }

        // Basic connection defaults
        buf.extend_from_slice(b"Connection: keep-alive\r\n");

        // For the initial write, we declare Content-Length, matching behavior in simple HTTP obfs
        buf.extend_from_slice(b"Content-Type: application/octet-stream\r\n");
        buf.extend_from_slice(b"Content-Length: ");
        buf.extend_from_slice(initial_body_len.to_string().as_bytes());
        buf.extend_from_slice(b"\r\n\r\n");

        Ok(buf)
    }
}

#[async_trait]
impl Transport for Client {
    async fn proxy_stream(&self, stream: AnyStream) -> std::io::Result<AnyStream> {
        Ok(Http1Stream::new(stream, self.host.clone(), self.path.clone(), self.method.clone(), self.headers.clone()).into())
    }
}

pub struct Http1Stream {
    inner: AnyStream,
    host: String,
    path: http::uri::PathAndQuery,
    method: Method,
    headers: HashMap<String, String>,

    first_request: bool,
    first_response: bool,
    read_buf: BytesMut,
}

impl Http1Stream {
    pub fn new(
        inner: AnyStream,
        host: String,
        path: http::uri::PathAndQuery,
        method: Method,
        headers: HashMap<String, String>,
    ) -> Self {
        Self {
            inner,
            host,
            path,
            method,
            headers,

            first_request: true,
            first_response: true,
            read_buf: BytesMut::new(),
        }
    }
}

impl AsyncWrite for Http1Stream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        let pin = self.get_mut();
        if pin.first_request {
            let client = Client::new(
                pin.host.clone(),
                pin.path.clone(),
                pin.method.clone(),
                pin.headers.clone(),
            );
            let mut req_prefix = client.build_request_prefix(buf.len())?;
            req_prefix.extend_from_slice(buf);
            pin.first_request = false;
            Pin::new(&mut pin.inner).poll_write(cx, &req_prefix)
        } else {
            Pin::new(&mut pin.inner).poll_write(cx, buf)
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        let pin = self.get_mut();
        Pin::new(&mut pin.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        let pin = self.get_mut();
        Pin::new(&mut pin.inner).poll_shutdown(cx)
    }
}

impl AsyncRead for Http1Stream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let pin = self.get_mut();
        if !pin.read_buf.is_empty() {
            let to_read = std::cmp::min(buf.remaining(), pin.read_buf.len());
            if to_read == 0 {
                assert!(buf.remaining() > 0);
                return std::task::Poll::Pending;
            }

            let data = pin.read_buf.split_to(to_read);
            buf.put_slice(&data[..]);
            return std::task::Poll::Ready(Ok(()));
        }

        if pin.first_response {
            let mut b = [0; 20 * 1024];
            let mut b = tokio::io::ReadBuf::new(&mut b);
            match Pin::new(&mut pin.inner).poll_read(cx, &mut b) {
                std::task::Poll::Ready(rv) => match rv {
                    Ok(_) => {
                        let needle = b"\r\n\r\n";
                        let idx = b
                            .filled()
                            .windows(needle.len())
                            .position(|window| window == needle);

                        if let Some(idx) = idx {
                            pin.first_response = false;
                            let body = &b.filled()[idx + 4..b.filled().len()];
                            if body.len() < buf.remaining() {
                                buf.put_slice(body);
                                std::task::Poll::Ready(Ok(()))
                            } else {
                                let to_read = &body[..buf.remaining()];
                                let to_buf = &body[buf.remaining()..];
                                buf.put_slice(to_read);
                                pin.read_buf.put_slice(to_buf);
                                std::task::Poll::Ready(Ok(()))
                            }
                        } else {
                            std::task::Poll::Ready(Err(std::io::Error::other("EOF")))
                        }
                    }
                    Err(e) => std::task::Poll::Ready(Err(e)),
                },
                std::task::Poll::Pending => std::task::Poll::Pending,
            }
        } else {
            Pin::new(&mut pin.inner).poll_read(cx, buf)
        }
    }
}

impl From<Http1Stream> for AnyStream {
    fn from(s: Http1Stream) -> Self {
        Box::new(s)
    }
}


