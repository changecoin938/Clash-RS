use async_trait::async_trait;

use super::Transport;
use crate::proxy::AnyStream;

#[derive(Clone, Default)]
pub struct Options {}

pub struct Client {
    #[allow(dead_code)]
    server: String,
    #[allow(dead_code)]
    port: u16,
    #[allow(dead_code)]
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
        Ok(stream)
    }
}


