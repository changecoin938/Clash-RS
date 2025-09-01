use crate::{
    Error,
    config::internal::proxy::OutboundVless,
    proxy::{
        HandlerCommonOptions,
        transport::{GrpcClient, H2Client, HttpClient, HttpUpgradeClient, TlsClient, WsClient, H3Client, XhttpClient, KcpClient, RealityClient},
        vless::{Handler, HandlerOptions},
    },
};
use tracing::warn;

impl TryFrom<OutboundVless> for Handler {
    type Error = crate::Error;

    fn try_from(value: OutboundVless) -> Result<Self, Self::Error> {
        (&value).try_into()
    }
}

impl TryFrom<&OutboundVless> for Handler {
    type Error = crate::Error;

    fn try_from(s: &OutboundVless) -> Result<Self, Self::Error> {
        let skip_cert_verify = s.skip_cert_verify.unwrap_or_default();
        if skip_cert_verify {
            warn!(
                "skipping TLS cert verification for {}",
                s.common_opts.server
            );
        }

        Ok(Handler::new(HandlerOptions {
            name: s.common_opts.name.to_owned(),
            common_opts: HandlerCommonOptions {
                connector: s.common_opts.connect_via.clone(),
                ..Default::default()
            },
            server: s.common_opts.server.to_owned(),
            port: s.common_opts.port,
            uuid: s.uuid.clone(),
            udp: s.udp.unwrap_or(true),
            transport: s
                .network
                .clone()
                .map(|x| match x.as_str() {
                    "ws" => s
                        .ws_opts
                        .as_ref()
                        .map(|x| {
                            let client: WsClient = (x, &s.common_opts)
                                .try_into()
                                .expect("invalid ws options");
                            Box::new(client) as _
                        })
                        .ok_or(Error::InvalidConfig(
                            "ws_opts is required for ws".to_owned(),
                        )),
                    "h2" => s
                        .h2_opts
                        .as_ref()
                        .map(|x| {
                            let client: H2Client = (x, &s.common_opts)
                                .try_into()
                                .expect("invalid h2 options");
                            Box::new(client) as _
                        })
                        .ok_or(Error::InvalidConfig(
                            "h2_opts is required for h2".to_owned(),
                        )),
                    "http" => s
                        .http_opts
                        .as_ref()
                        .map(|x| {
                            let client: HttpClient = (x, &s.common_opts)
                                .try_into()
                                .expect("invalid http options");
                            Box::new(client) as _
                        })
                        .ok_or(Error::InvalidConfig(
                            "http_opts is required for http".to_owned(),
                        )),
                    "httpupgrade" => s
                        .http_opts
                        .as_ref()
                        .map(|x| {
                            let client: HttpUpgradeClient = (x, &s.common_opts)
                                .try_into()
                                .expect("invalid http options");
                            Box::new(client) as _
                        })
                        .ok_or(Error::InvalidConfig(
                            "http_opts is required for httpupgrade".to_owned(),
                        )),
                    "xhttp" => s
                        .http_opts
                        .as_ref()
                        .map(|x| {
                            let client: XhttpClient = (x, &s.common_opts)
                                .try_into()
                                .expect("invalid xhttp options");
                            Box::new(client) as _
                        })
                        .ok_or(Error::InvalidConfig(
                            "http_opts is required for xhttp".to_owned(),
                        )),
                    "grpc" => s
                        .grpc_opts
                        .as_ref()
                        .map(|x| {
                            let client: GrpcClient =
                                (s.server_name.clone(), x, &s.common_opts)
                                    .try_into()
                                    .expect("invalid grpc options");
                            Box::new(client) as _
                        })
                        .ok_or(Error::InvalidConfig(
                            "grpc_opts is required for grpc".to_owned(),
                        )),
                    "h3" => s
                        .http_opts
                        .as_ref()
                        .map(|x| {
                            let client: H3Client = (x, &s.common_opts)
                                .try_into()
                                .expect("invalid h3 options");
                            Box::new(client) as _
                        })
                        .ok_or(Error::InvalidConfig(
                            "http_opts is required for h3".to_owned(),
                        )),
                    "mkcp" => s
                        .kcp_opts
                        .as_ref()
                        .map(|x| {
                            let client: KcpClient = (x, &s.common_opts)
                                .try_into()
                                .expect("invalid mkcp options");
                            Box::new(client) as _
                        })
                        .ok_or(Error::InvalidConfig(
                            "kcp_opts is required for mkcp".to_owned(),
                        )),
                    "reality" => s
                        .http_opts
                        .as_ref()
                        .map(|_x| {
                            // Map basic REALITY to TLS with provided SNI/ALPN; advanced fields can be extended later.
                            let client = RealityClient::new(crate::proxy::transport::RealityOptions {
                                skip_cert_verify: s.skip_cert_verify.unwrap_or_default(),
                                sni: s.server_name.clone(),
                                alpn: Some(vec!["h2".to_owned(), "http/1.1".to_owned()]),
                            });
                            Box::new(client) as _
                        })
                        .ok_or(Error::InvalidConfig(
                            "http_opts is required for reality".to_owned(),
                        )),
                    _ => Err(Error::InvalidConfig(format!(
                        "unsupported network: {x}"
                    ))),
                })
                .transpose()?,
            tls: match s.tls.unwrap_or_default() {
                true => {
                    let client = TlsClient::new(
                        s.skip_cert_verify.unwrap_or_default(),
                        s.server_name.as_ref().map(|x| x.to_owned()).unwrap_or(
                            s.ws_opts
                                .as_ref()
                                .and_then(|x| {
                                    x.headers.clone().and_then(|x| {
                                        let h = x.get("Host");
                                        h.cloned()
                                    })
                                })
                                .unwrap_or(s.common_opts.server.to_owned())
                                .to_owned(),
                        ),
                        s.network
                            .as_ref()
                            .map(|x| match x.as_str() {
                                "ws" => Ok(vec!["http/1.1".to_owned()]),
                                "http" => Ok(vec![]),
                                "h2" | "grpc" => Ok(vec!["h2".to_owned()]),
                                _ => Err(Error::InvalidConfig(format!(
                                    "unsupported network: {x}"
                                ))),
                            })
                            .transpose()?,
                        None,
                    );
                    Some(Box::new(client))
                }
                false => None,
            },
        }))
    }
}
