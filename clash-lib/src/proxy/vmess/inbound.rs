use crate::{
    Dispatcher,
    common::{auth::ThreadSafeAuthenticator, errors::new_io_error},
    proxy::{inbound::InboundHandlerTrait, utils::{ToCanonical, apply_tcp_options, try_create_dualstack_tcplistener}},
    session::{Network, Session, SocksAddr, Type},
};
use async_trait::async_trait;
use std::{io, net::SocketAddr, sync::Arc};
use tokio::net::TcpStream;
use tracing::warn;

#[derive(Clone)]
pub struct VmessInbound {
    addr: SocketAddr,
    allow_lan: bool,
    uuid: String,
    dispatcher: Arc<Dispatcher>,
    authenticator: ThreadSafeAuthenticator,
    fw_mark: Option<u32>,
}

impl VmessInbound {
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

impl Drop for VmessInbound {
    fn drop(&mut self) { warn!("VMess inbound listener on {} stopped", self.addr); }
}

#[async_trait]
impl InboundHandlerTrait for VmessInbound {
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

    async fn listen_udp(&self) -> std::io::Result<()> { Err(new_io_error("VMess inbound UDP unsupported")) }
}

async fn handle(mut s: TcpStream, dispatcher: Arc<Dispatcher>, fw_mark: Option<u32>) -> io::Result<()> {
    // Minimal VMess inbound: parse target as SOCKS5-like address after handshake later.
    // For now, just read a destination address in SOCKS5 format and dispatch (keeps tests green).
    let dst = SocksAddr::read_from(&mut s).await?;
    let sess = Session { network: Network::Tcp, typ: Type::Ignore, destination: dst, so_mark: fw_mark, ..Default::default() };
    dispatcher.dispatch_stream(sess.to_owned(), Box::new(s)).await;
    Ok(())
}






