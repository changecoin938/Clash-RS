use async_trait::async_trait;

use super::Transport;
use crate::proxy::AnyStream;

#[derive(Clone, Default)]
pub struct Options {
    pub skip_cert_verify: bool,
    pub sni: Option<String>,
    pub alpn: Option<Vec<String>>, // e.g., ["h2"], ["http/1.1"]
}

pub struct Client {
    opts: Options,
}

impl Client {
    pub fn new(opts: Options) -> Self { Self { opts } }
}

#[async_trait]
impl Transport for Client {
    async fn proxy_stream(&self, stream: AnyStream) -> std::io::Result<AnyStream> {
        // Use existing TLS transport with provided options. REALITY-specific
        // fingerprinting is not modeled here; this provides a TLS wrapper with custom SNI/ALPN.
        let sni = self.opts.sni.clone().unwrap_or_else(|| "".to_owned());
        let tls = crate::proxy::transport::tls::Client::new(
            self.opts.skip_cert_verify,
            sni,
            self.opts.alpn.clone(),
            None,
        );
        tls.proxy_stream(stream).await
    }
}


