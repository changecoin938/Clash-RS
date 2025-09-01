use tracing::warn;

static DEFAULT_ALPN: [&str; 2] = ["h2", "http/1.1"];

use crate::{
    Error,
    config::internal::proxy::OutboundTrojan,
    proxy::{
        HandlerCommonOptions,
        transport::{GrpcClient, HttpClient, HttpUpgradeClient, TlsClient, WsClient, XhttpClient, RealityClient},
        trojan::{Handler, HandlerOptions},
    },
};

impl TryFrom<OutboundTrojan> for Handler {
    type Error = crate::Error;

    fn try_from(value: OutboundTrojan) -> Result<Self, Self::Error> {
        (&value).try_into()
    }
}

impl TryFrom<&OutboundTrojan> for Handler {
    type Error = crate::Error;

    fn try_from(s: &OutboundTrojan) -> Result<Self, Self::Error> {
        let skip_cert_verify = s.skip_cert_verify.unwrap_or_default();
        if skip_cert_verify {
            warn!(
                "skipping TLS cert verification for {}",
                s.common_opts.server
            );
        }

        let h = Handler::new(HandlerOptions {
            name: s.common_opts.name.to_owned(),
            common_opts: HandlerCommonOptions {
                connector: s.common_opts.connect_via.clone(),
                ..Default::default()
            },
            server: s.common_opts.server.to_owned(),
            port: s.common_opts.port,
            password: s.password.clone(),
            udp: s.udp.unwrap_or_default(),
            tls: {
                let client = TlsClient::new(
                    skip_cert_verify,
                    s.sni
                        .as_ref()
                        .map(|x| x.to_owned())
                        .unwrap_or(s.common_opts.server.to_owned()),
                    s.alpn.clone().or(Some(
                        DEFAULT_ALPN
                            .iter()
                            .copied()
                            .map(|x| x.to_owned())
                            .collect::<Vec<String>>(),
                    )),
                    None,
                );
                Some(Box::new(client))
            },
            transport: s
                .network
                .as_ref()
                .map(|x| match x.as_str() {
                    "ws" => s
                        .ws_opts
                        .as_ref()
                        .map(|x| {
                            let client: WsClient = (x, &s.common_opts)
                                .try_into()
                                .expect("invalid ws_opts");
                            Box::new(client) as _
                        })
                        .ok_or(Error::InvalidConfig(
                            "ws_opts is required for ws".to_owned(),
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
                    "grpc" => s
                        .grpc_opts
                        .as_ref()
                        .map(|x| {
                            let client: GrpcClient =
                                (s.sni.clone(), x, &s.common_opts)
                                    .try_into()
                                    .expect("invalid grpc_opts");
                            Box::new(client) as _
                        })
                        .ok_or(Error::InvalidConfig(
                            "grpc_opts is required for grpc".to_owned(),
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
                    "reality" => s
                        .http_opts
                        .as_ref()
                        .map(|_x| {
                            let client = RealityClient::new(crate::proxy::transport::RealityOptions {
                                skip_cert_verify: s.skip_cert_verify.unwrap_or_default(),
                                sni: s.sni.clone().or(Some(s.common_opts.server.clone())),
                                alpn: s.alpn.clone().or(Some(vec!["h2".to_owned(), "http/1.1".to_owned()])),
                            });
                            Box::new(client) as _
                        })
                        .ok_or(Error::InvalidConfig(
                            "http_opts is required for reality".to_owned(),
                        )),
                    _ => Err(Error::InvalidConfig(format!(
                        "unsupported trojan network: {x}"
                    ))),
                })
                .transpose()?,
        });
        Ok(h)
    }
}
