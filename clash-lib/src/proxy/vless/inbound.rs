use crate::{
    Dispatcher,
    common::{auth::ThreadSafeAuthenticator, errors::new_io_error},
    proxy::{inbound::InboundHandlerTrait, utils::{ToCanonical, apply_tcp_options, try_create_dualstack_tcplistener}},
    session::{Network, Session, SocksAddr, Type},
};
use async_trait::async_trait;
use std::{io, net::SocketAddr, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use tracing::warn;

#[derive(Clone)]
pub struct VlessInbound {
    addr: SocketAddr,
    allow_lan: bool,
    uuid: String,
    dispatcher: Arc<Dispatcher>,
    authenticator: ThreadSafeAuthenticator,
    fw_mark: Option<u32>,
}

impl VlessInbound {
    pub fn new(
        addr: SocketAddr,
        allow_lan: bool,
        uuid: String,
        dispatcher: Arc<Dispatcher>,
        authenticator: ThreadSafeAuthenticator,
        fw_mark: Option<u32>,
    ) -> Self {
        Self { addr, allow_lan, uuid, dispatcher, authenticator, fw_mark }
    }
}

impl Drop for VlessInbound {
    fn drop(&mut self) { warn!("VLESS inbound listener on {} stopped", self.addr); }
}

#[async_trait]
impl InboundHandlerTrait for VlessInbound {
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
            let fw_mark = self.fw_mark;
            tokio::spawn(async move {
                if let Err(e) = handle(socket, dispatcher, fw_mark).await {
                    let _ = e;
                }
            });
        }
    }

    async fn listen_udp(&self) -> std::io::Result<()> { Err(new_io_error("VLESS inbound UDP unsupported")) }
}

async fn handle(mut s: TcpStream, dispatcher: Arc<Dispatcher>, fw_mark: Option<u32>) -> io::Result<()> {
    // VLESS inbound handshake (version 0)
    // Request: [ver:1][uuid:16][optlen:1][opts:optlen][cmd:1][port:2][addr_type:1][addr][extra]
    // Response: [ver:1][add_len:1][add:0]

    // version
    let version = s.read_u8().await?;
    if version != 0 {
        return Err(io::Error::other("unsupported VLESS version"));
    }

    // uuid
    let mut uuid_bytes = [0u8; 16];
    s.read_exact(&mut uuid_bytes).await?;

    // options
    let opt_len = s.read_u8().await? as usize;
    if opt_len > 0 {
        let mut opts = vec![0u8; opt_len];
        s.read_exact(&mut opts).await?;
        // For now we ignore optional fields
    }

    // command
    let _cmd = s.read_u8().await?; // 1: TCP, 2: UDP

    // destination
    let port = s.read_u16().await?;
    let atyp = s.read_u8().await?;

    let dst = match atyp {
        0x01 => {
            // IPv4 (4 bytes)
            let mut ip = [0u8; 4];
            s.read_exact(&mut ip).await?;
            SocksAddr::from((std::net::Ipv4Addr::from(ip), port))
        }
        0x03 => {
            // IPv6 (16 bytes) - note: VLESS uses 3 for IPv6
            let mut ip6 = [0u8; 16];
            s.read_exact(&mut ip6).await?;
            SocksAddr::from((std::net::Ipv6Addr::from(ip6), port))
        }
        0x02 => {
            // Domain
            let dlen = s.read_u8().await? as usize;
            let mut buf = vec![0u8; dlen];
            s.read_exact(&mut buf).await?;
            let domain = String::from_utf8(buf)
                .map_err(|_| io::Error::other("invalid domain in VLESS"))?;
            SocksAddr::try_from((domain, port))?
        }
        _ => return Err(io::Error::other("invalid VLESS address type")),
    };

    // Send minimal response: version + add_len(0)
    s.write_all(&[0u8, 0u8]).await?;
    s.flush().await?;

    let sess = Session {
        network: Network::Tcp,
        typ: Type::Ignore,
        destination: dst,
        so_mark: fw_mark,
        ..Default::default()
    };
    dispatcher.dispatch_stream(sess.to_owned(), Box::new(s)).await;
    Ok(())
}



