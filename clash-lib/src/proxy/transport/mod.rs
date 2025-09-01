mod grpc;
mod h2;
mod http1;
mod httpupgrade;
mod shadow_tls;
mod simple_obfs;
mod sip003;
mod tls;
mod v2ray;
mod ws;
pub mod mkcp;
mod h3;
mod xhttp;
mod reality;

pub use grpc::Client as GrpcClient;
pub use h2::Client as H2Client;
pub use http1::Client as HttpClient;
pub use httpupgrade::Client as HttpUpgradeClient;
pub use shadow_tls::Client as Shadowtls;
pub use simple_obfs::*;
pub use sip003::Plugin as Sip003Plugin;
pub use tls::Client as TlsClient;
pub use v2ray::{V2RayOBFSOption, V2rayWsClient};
pub use ws::Client as WsClient;
pub use h3::Client as H3Client;
pub use h3::Options as H3Options;
pub use mkcp::Client as KcpClient;
pub use xhttp::Client as XhttpClient;
pub use xhttp::Options as XhttpOptions;
pub use reality::Client as RealityClient;
pub use reality::Options as RealityOptions;

use downcast_rs::{impl_downcast, Downcast};

#[async_trait::async_trait]
pub trait Transport: Send + Sync + Downcast {
    async fn proxy_stream(
        &self,
        stream: super::AnyStream,
    ) -> std::io::Result<super::AnyStream>;
}

impl_downcast!(Transport);
