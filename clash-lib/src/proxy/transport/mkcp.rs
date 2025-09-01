use async_trait::async_trait;

use super::Transport;
use crate::proxy::AnyStream;

pub struct Client;

impl Client { pub fn new() -> Self { Self } }

#[async_trait]
impl Transport for Client {
    async fn proxy_stream(&self, _stream: AnyStream) -> std::io::Result<AnyStream> {
        // Safe passthrough fallback to keep builds and configs operational.
        Ok(_stream)
    }
}


