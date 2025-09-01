use async_trait::async_trait;
use base64::Engine;
use std::{fmt::Debug, io, sync::Arc};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::debug;

use crate::{
    app::{
        dispatcher::{BoxedChainedDatagram, BoxedChainedStream, ChainedStreamWrapper},
        dns::ThreadSafeDNSResolver,
    },
    app::dispatcher::ChainedStream,
    impl_default_connector,
    proxy::{
        AnyStream, ConnectorType, DialWithConnector, HandlerCommonOptions, OutboundHandler,
        OutboundType,
        transport::{TlsClient, Transport},
        utils::{GLOBAL_DIRECT_CONNECTOR, RemoteConnector},
    },
    session::Session,
};

#[derive(Default)]
pub struct HandlerOptions {
    pub name: String,
    pub common_opts: HandlerCommonOptions,
    pub server: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub tls: bool,
    pub sni: Option<String>,
    pub skip_cert_verify: bool,
}

pub struct Handler {
    opts: HandlerOptions,
    connector: tokio::sync::RwLock<Option<Arc<dyn RemoteConnector>>>,
}

impl Default for Handler {
    fn default() -> Self {
        Self::new(Default::default())
    }
}

impl Debug for Handler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpOutbound").field("name", &self.opts.name).finish()
    }
}

impl_default_connector!(Handler);

impl Handler {
    pub fn new(opts: HandlerOptions) -> Self {
        Self {
            opts,
            connector: Default::default(),
        }
    }

    async fn build_stream(&self, stream: AnyStream) -> io::Result<AnyStream> {
        if self.opts.tls {
            let tls = TlsClient::new(
                self.opts.skip_cert_verify,
                self.opts
                    .sni
                    .clone()
                    .unwrap_or_else(|| self.opts.server.clone()),
                None,
                None,
            );
            return tls.proxy_stream(stream).await;
        }
        Ok(stream)
    }

    fn basic_auth_header(&self) -> Option<String> {
        match (&self.opts.username, &self.opts.password) {
            (Some(u), Some(p)) => {
                let raw = format!("{}:{}", u, p);
                Some(format!(
                    "Basic {}",
                    base64::engine::general_purpose::STANDARD.encode(raw)
                ))
            }
            _ => None,
        }
    }

    async fn http_connect(&self, mut s: AnyStream, host: &str, port: u16) -> io::Result<AnyStream> {
        let mut req = format!("CONNECT {}:{} HTTP/1.1\r\nHost: {}:{}\r\n", host, port, host, port);
        if let Some(auth) = self.basic_auth_header() {
            req.push_str(&format!("Proxy-Authorization: {}\r\n", auth));
        }
        req.push_str("Proxy-Connection: keep-alive\r\n\r\n");
        s.write_all(req.as_bytes()).await?;

        // Read a small response header
        let mut buf = vec![0u8; 1024];
        let n = s.read(&mut buf).await?;
        let head = &buf[..n];
        let ok = head.starts_with(b"HTTP/1.1 200") || head.starts_with(b"HTTP/1.0 200");
        if !ok {
            return Err(io::Error::other("http proxy connect failed"));
        }
        Ok(s)
    }
}

#[async_trait]
impl OutboundHandler for Handler {
    fn name(&self) -> &str { &self.opts.name }

    fn proto(&self) -> OutboundType { OutboundType::Direct }

    async fn support_udp(&self) -> bool { false }

    async fn connect_stream(&self, sess: &Session, resolver: ThreadSafeDNSResolver) -> io::Result<BoxedChainedStream> {
        let dialer = self.connector.read().await;
        if let Some(d) = dialer.as_ref() { debug!("{:?} is connecting via {:?}", self, d); }
        self.connect_stream_with_connector(sess, resolver, dialer.as_ref().unwrap_or(&GLOBAL_DIRECT_CONNECTOR.clone()).as_ref()).await
    }

    async fn connect_datagram(&self, _sess: &Session, _resolver: ThreadSafeDNSResolver) -> io::Result<BoxedChainedDatagram> {
        Err(io::Error::other("http outbound does not support UDP"))
    }

    async fn support_connector(&self) -> ConnectorType { ConnectorType::Tcp }

    async fn connect_stream_with_connector(
        &self,
        sess: &Session,
        resolver: ThreadSafeDNSResolver,
        connector: &dyn RemoteConnector,
    ) -> io::Result<BoxedChainedStream> {
        let raw = connector
            .connect_stream(
                resolver,
                self.opts.server.as_str(),
                self.opts.port,
                sess.iface.as_ref(),
                #[cfg(target_os = "linux")]
                sess.so_mark,
            )
            .await?;
        let proxied = self.build_stream(raw).await?;
        let after_connect = self.http_connect(proxied, &sess.destination.host(), sess.destination.port()).await?;
        let chained = ChainedStreamWrapper::new(after_connect);
        chained.append_to_chain(self.name()).await;
        Ok(Box::new(chained))
    }
}


