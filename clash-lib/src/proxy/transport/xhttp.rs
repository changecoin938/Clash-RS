use async_trait::async_trait;
use bytes::BufMut;
use std::pin::Pin;
use tokio::io::{AsyncRead, AsyncWrite};

use super::Transport;
use crate::proxy::AnyStream;

#[derive(Clone, Default)]
pub struct Options {
    pub host: Option<String>,
    pub path: Option<String>,
    pub method: Option<String>,
    pub headers: Option<std::collections::HashMap<String, String>>,
}

pub struct Client {
    server: String,
    port: u16,
    opts: Options,
}

impl Client {
    pub fn new(server: String, port: u16, opts: Options) -> Self {
        Self { server, port, opts }
    }
}

#[async_trait]
impl Transport for Client {
    async fn proxy_stream(&self, stream: AnyStream) -> std::io::Result<AnyStream> {
        let host = self
            .opts
            .host
            .clone()
            .unwrap_or_else(|| self.server.clone());
        let path = self
            .opts
            .path
            .clone()
            .unwrap_or_else(|| "/".to_owned());
        let method = self
            .opts
            .method
            .clone()
            .unwrap_or_else(|| "GET".to_owned());
        let headers = self
            .opts
            .headers
            .clone()
            .unwrap_or_default();

        Ok(XHttpObfs::new(stream, host, self.port, path, method, headers).into())
    }
}

struct XHttpObfs {
    inner: AnyStream,
    host: String,
    port: u16,
    path: String,
    method: String,
    headers: std::collections::HashMap<String, String>,

    first_request: bool,
    first_response: bool,
    read_buf: bytes::BytesMut,
}

impl XHttpObfs {
    fn new(
        inner: AnyStream,
        host: String,
        port: u16,
        path: String,
        method: String,
        headers: std::collections::HashMap<String, String>,
    ) -> Self {
        Self {
            inner,
            host,
            port,
            path,
            method,
            headers,
            first_request: true,
            first_response: true,
            read_buf: bytes::BytesMut::new(),
        }
    }
}

impl AsyncWrite for XHttpObfs {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        let pin = self.get_mut();
        if pin.first_request {
            let mut header = Vec::new();
            header.put_slice(format!("{} {} HTTP/1.1\r\n", pin.method, pin.path).as_bytes());
            header.put_slice(
                format!(
                    "Host: {}\r\n",
                    if pin.port != 80 { format!("{}:{}", pin.host, pin.port) } else { pin.host.clone() }
                )
                .as_bytes(),
            );
            // Xray/XHTTP tends to include Connection/Pragma style headers; allow custom headers from config
            for (k, v) in pin.headers.iter() {
                if k.eq_ignore_ascii_case("content-length") {
                    continue;
                }
                header.put_slice(format!("{}: {}\r\n", k, v).as_bytes());
            }
            header.put_slice(format!("Content-Length: {}\r\n\r\n", buf.len()).as_bytes());
            header.put_slice(buf);

            pin.first_request = false;
            Pin::new(&mut pin.inner).poll_write(cx, &header)
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

impl AsyncRead for XHttpObfs {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let pin = self.get_mut();
        if !pin.read_buf.is_empty() {
            let to_read = std::cmp::min(buf.remaining(), pin.read_buf.len());
            if to_read == 0 {
                return std::task::Poll::Pending;
            }

            let data = pin.read_buf.split_to(to_read);
            buf.put_slice(&data[..]);
            return std::task::Poll::Ready(Ok(()));
        }

        if pin.first_response {
            let mut tmp = [0; 20 * 1024];
            let mut tmp = tokio::io::ReadBuf::new(&mut tmp);
            match Pin::new(&mut pin.inner).poll_read(cx, &mut tmp) {
                std::task::Poll::Ready(rv) => match rv {
                    Ok(_) => {
                        let needle = b"\r\n\r\n";
                        let filled = tmp.filled();
                        if let Some(idx) = filled.windows(needle.len()).position(|w| w == needle) {
                            pin.first_response = false;
                            let body = &filled[idx + 4..filled.len()];
                            if body.len() <= buf.remaining() {
                                buf.put_slice(body);
                                std::task::Poll::Ready(Ok(()))
                            } else {
                                let to_read = &body[..buf.remaining()];
                                let rest = &body[buf.remaining()..];
                                buf.put_slice(to_read);
                                pin.read_buf.put_slice(rest);
                                std::task::Poll::Ready(Ok(()))
                            }
                        } else {
                            std::task::Poll::Ready(Err(std::io::Error::other("invalid http response")))
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

impl From<XHttpObfs> for AnyStream {
    fn from(obfs: XHttpObfs) -> Self { Box::new(obfs) }
}


