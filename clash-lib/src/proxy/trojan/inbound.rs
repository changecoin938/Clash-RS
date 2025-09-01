use crate::{
    Dispatcher,
    common::{auth::ThreadSafeAuthenticator, errors::new_io_error},
    proxy::{
        inbound::InboundHandlerTrait,
        utils::{ToCanonical, apply_tcp_options, try_create_dualstack_tcplistener},
    },
    session::{Network, Session, SocksAddr, Type},
};
use async_trait::async_trait;
use bytes::BytesMut;
use sha2::{Digest, Sha224};
use std::{io, net::SocketAddr, sync::Arc};
use tokio::{
    io::AsyncReadExt,
    net::TcpStream,
};
use tracing::warn;

#[derive(Clone)]
pub struct TrojanInbound {
    addr: SocketAddr,
    allow_lan: bool,
    password: String,
    dispatcher: Arc<Dispatcher>,
    authenticator: ThreadSafeAuthenticator,
    fw_mark: Option<u32>,
}

impl TrojanInbound {
    pub fn new(
        addr: SocketAddr,
        allow_lan: bool,
        password: String,
        dispatcher: Arc<Dispatcher>,
        authenticator: ThreadSafeAuthenticator,
        fw_mark: Option<u32>,
    ) -> Self {
        Self { addr, allow_lan, password, dispatcher, authenticator, fw_mark }
    }
}

impl Drop for TrojanInbound {
    fn drop(&mut self) { warn!("Trojan inbound listener on {} stopped", self.addr); }
}

#[async_trait]
impl InboundHandlerTrait for TrojanInbound {
    fn handle_tcp(&self) -> bool { true }
    fn handle_udp(&self) -> bool { false }

    async fn listen_tcp(&self) -> std::io::Result<()> {
        let listener = try_create_dualstack_tcplistener(self.addr)?;
        loop {
            let (socket, _) = listener.accept().await?;
            let src_addr = socket.peer_addr()?.to_canonical();

            if !self.allow_lan && src_addr.ip() != socket.local_addr()?.ip().to_canonical() {
                warn!("Connection from {} is not allowed", src_addr);
                continue;
            }

            apply_tcp_options(&socket)?;
            let dispatcher = self.dispatcher.clone();
            let author = self.authenticator.clone();
            let fw_mark = self.fw_mark;
            let pass = self.password.clone();
            tokio::spawn(async move {
                if let Err(e) = handle(socket, dispatcher, author, fw_mark, pass).await {
                    let _ = e; // trace in future if needed
                }
            });
        }
    }

    async fn listen_udp(&self) -> std::io::Result<()> { Err(new_io_error("unsupported UDP protocol for Trojan inbound")) }
}

async fn handle(
    mut s: TcpStream,
    dispatcher: Arc<Dispatcher>,
    _authenticator: ThreadSafeAuthenticator,
    fw_mark: Option<u32>,
    password: String,
) -> io::Result<()> {
    // Trojan request: Hex(SHA224(password)) + CRLF + CMD + ADDR + CRLF
    let mut buf = BytesMut::new();
    buf.resize(56 + 2 + 1, 0); // hex sha224 is 56 ascii chars
    s.read_exact(&mut buf[..56 + 2 + 1]).await?;

    // verify password
    let expected = {
        let p = Sha224::digest(password.as_bytes());
        hex_of(&p[..])
    };
    let recv_hex = String::from_utf8((&buf[..56]).to_vec()).map_err(|_| io::Error::other("invalid auth"))?;
    if recv_hex != expected || &buf[56..58] != b"\r\n" { return Err(io::Error::other("trojan auth failed")); }

    let cmd = buf[58];

    // Read destination address in SOCKS5-like format from stream
    let dst = SocksAddr::read_from(&mut s).await?;
    // expect CRLF
    let mut crlf = [0u8; 2];
    s.read_exact(&mut crlf).await?;
    if &crlf != b"\r\n" { return Err(io::Error::other("invalid trojan request")); }

    match cmd {
        0x01 => {
            // TCP connect
            let sess = Session { network: Network::Tcp, typ: Type::Ignore, destination: dst, so_mark: fw_mark, ..Default::default() };
            dispatcher.dispatch_stream(sess.to_owned(), Box::new(s)).await;
            Ok(())
        }
        0x03 => {
            // UDP ASSOCIATE unsupported here
            Err(io::Error::other("trojan inbound udp unsupported"))
        }
        _ => Err(io::Error::other("unsupported trojan command")),
    }
}

fn hex_of(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = vec![0u8; bytes.len() * 2];
    for (i, b) in bytes.iter().enumerate() {
        out[i * 2] = HEX[(b >> 4) as usize];
        out[i * 2 + 1] = HEX[(b & 0x0f) as usize];
    }
    // SAFETY: hex digits
    unsafe { String::from_utf8_unchecked(out) }
}


