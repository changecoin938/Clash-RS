use std::collections::HashMap;

use crate::{
    Error,
    config::internal::proxy::OutboundShadowsocks,
    proxy::{
        HandlerCommonOptions,
        shadowsocks::outbound::{Handler, HandlerOptions},
        transport::{
            GrpcClient, HttpUpgradeClient, KcpClient, Shadowtls, SimpleOBFSMode,
            SimpleOBFSOption, SimpleObfsHttp, SimpleObfsTLS, V2RayOBFSOption,
            V2rayWsClient, XhttpClient,
        },
    },
};

impl TryFrom<OutboundShadowsocks> for Handler {
    type Error = crate::Error;

    fn try_from(value: OutboundShadowsocks) -> Result<Self, Self::Error> {
        (&value).try_into()
    }
}

impl TryFrom<&OutboundShadowsocks> for Handler {
    type Error = crate::Error;

    fn try_from(s: &OutboundShadowsocks) -> Result<Self, Self::Error> {
        // Prefer explicit plugin if provided; otherwise map high-level network → plugin
        let plugin = match &s.plugin {
            Some(_) => None, // handled below
            None => match s.network.as_deref() {
                Some("ws") => {
                    // Build V2rayWsClient from ws_opts
                    let opt: V2RayOBFSOption = {
                        let mut headers = HashMap::new();
                        if let Some(h) = s.ws_opts.as_ref()
                            .and_then(|o| o.headers.as_ref())
                        {
                            headers = h.clone();
                        }
                        V2RayOBFSOption {
                            mode: "websocket".to_owned(),
                            host: s.common_opts.server.clone(),
                            port: s.common_opts.port,
                            path: s.ws_opts.as_ref().and_then(|o| o.path.clone()).unwrap_or("/".to_owned()),
                            tls: s.tls.unwrap_or(false),
                            headers,
                            skip_cert_verify: s.skip_cert_verify.unwrap_or(false),
                            mux: false,
                        }
                    };
                    let plugin = V2rayWsClient::try_from(opt)
                        .map_err(|e| Error::InvalidConfig(format!("invalid ws opts: {e}")))?;
                    Some(Box::new(plugin) as _)
                }
                Some("http") => {
                    // Map to simple-obfs http
                    let host = s
                        .http_opts
                        .as_ref()
                        .and_then(|o| o.headers.as_ref())
                        .and_then(|h| h.get("Host").cloned())
                        .unwrap_or_else(|| s.common_opts.server.clone());
                    Some(Box::new(SimpleObfsHttp::new(host, s.common_opts.port)) as _)
                }
                Some("tls") => {
                    // Map to simple-obfs tls
                    let host = s
                        .http_opts
                        .as_ref()
                        .and_then(|o| o.headers.as_ref())
                        .and_then(|h| h.get("Host").cloned())
                        .unwrap_or_else(|| s.sni.clone().unwrap_or(s.common_opts.server.clone()));
                    Some(Box::new(SimpleObfsTLS::new(host)) as _)
                }
                Some("httpupgrade") => {
                    let client: HttpUpgradeClient = s
                        .http_opts
                        .as_ref()
                        .ok_or(Error::InvalidConfig(
                            "http_opts is required for httpupgrade".to_owned(),
                        ))
                        .and_then(|x| (x, &s.common_opts).try_into().map_err(|e| Error::InvalidConfig(format!("invalid httpupgrade options: {e}"))))?;
                    Some(Box::new(client) as _)
                }
                Some("xhttp") => {
                    let client: XhttpClient = s
                        .http_opts
                        .as_ref()
                        .ok_or(Error::InvalidConfig(
                            "http_opts is required for xhttp".to_owned(),
                        ))
                        .and_then(|x| (x, &s.common_opts).try_into().map_err(|e| Error::InvalidConfig(format!("invalid xhttp options: {e}"))))?;
                    Some(Box::new(client) as _)
                }
                Some("grpc") => {
                    let client: GrpcClient = (s.sni.clone(), s
                        .grpc_opts
                        .as_ref()
                        .ok_or(Error::InvalidConfig(
                            "grpc_opts is required for grpc".to_owned(),
                        ))?, &s.common_opts)
                        .try_into()
                        .map_err(|e| Error::InvalidConfig(format!("invalid grpc options: {e}")))?;
                    Some(Box::new(client) as _)
                }
                Some("mkcp") => {
                    let client: KcpClient = s
                        .kcp_opts
                        .as_ref()
                        .ok_or(Error::InvalidConfig(
                            "kcp_opts is required for mkcp".to_owned(),
                        ))
                        .and_then(|x| (x, &s.common_opts).try_into().map_err(|e| Error::InvalidConfig(format!("invalid mkcp options: {e}"))))?;
                    Some(Box::new(client) as _)
                }
                _ => None,
            },
        };

        let h = Handler::new(HandlerOptions {
            name: s.common_opts.name.to_owned(),
            common_opts: HandlerCommonOptions {
                connector: s.common_opts.connect_via.clone(),
                ..Default::default()
            },
            server: s.common_opts.server.to_owned(),
            port: s.common_opts.port,
            password: s.password.to_owned(),
            cipher: s.cipher.to_owned(),
            plugin: match &s.plugin {
                Some(plugin) => match plugin.as_str() {
                    "obfs" => {
                        tracing::warn!(
                            "simple-obfs is deprecated, please use v2ray-plugin \
                             instead"
                        );
                        let opt: SimpleOBFSOption = s
                            .plugin_opts
                            .clone()
                            .ok_or(Error::InvalidConfig(
                                "plugin_opts is required for plugin obfs".to_owned(),
                            ))?
                            .try_into()?;
                        let plugin = match opt.mode {
                            SimpleOBFSMode::Http => Box::new(SimpleObfsHttp::new(
                                opt.host,
                                s.common_opts.port,
                            ))
                                as _,
                            SimpleOBFSMode::Tls => {
                                Box::new(SimpleObfsTLS::new(opt.host)) as _
                            }
                        };
                        Some(plugin)
                    }
                    "v2ray-plugin" => {
                        let opt: V2RayOBFSOption = s
                            .plugin_opts
                            .clone()
                            .ok_or(Error::InvalidConfig(
                                "plugin_opts is required for plugin obfs".to_owned(),
                            ))?
                            .try_into()?;
                        // TODO: support more transport options, replace it with
                        // `V2rayClient`
                        let plugin = V2rayWsClient::try_from(opt)?;
                        Some(Box::new(plugin) as _)
                    }
                    "shadow-tls" => {
                        let plugin: Shadowtls = s
                            .plugin_opts
                            .clone()
                            .ok_or(Error::InvalidConfig(
                                "plugin_opts is required for plugin obfs".to_owned(),
                            ))?
                            .try_into()?;
                        Some(Box::new(plugin) as _)
                    }
                    _ => {
                        return Err(Error::InvalidConfig(format!(
                            "unsupported plugin: {plugin}"
                        )));
                    }
                },
                None => plugin,
            },
            udp: s.udp,
        });
        Ok(h)
    }
}

impl TryFrom<HashMap<String, serde_yaml::Value>> for SimpleOBFSOption {
    type Error = crate::Error;

    fn try_from(
        value: HashMap<String, serde_yaml::Value>,
    ) -> Result<Self, Self::Error> {
        let host = value
            .get("host")
            .and_then(|x| x.as_str())
            .unwrap_or("bing.com");
        let mode = value
            .get("mode")
            .and_then(|x| x.as_str())
            .ok_or(Error::InvalidConfig("obfs mode is required".to_owned()))?;

        match mode {
            "http" => Ok(SimpleOBFSOption {
                mode: SimpleOBFSMode::Http,
                host: host.to_owned(),
            }),
            "tls" => Ok(SimpleOBFSOption {
                mode: SimpleOBFSMode::Tls,
                host: host.to_owned(),
            }),
            _ => Err(Error::InvalidConfig(format!("invalid obfs mode: {mode}"))),
        }
    }
}

impl TryFrom<HashMap<String, serde_yaml::Value>> for V2RayOBFSOption {
    type Error = crate::Error;

    fn try_from(
        value: HashMap<String, serde_yaml::Value>,
    ) -> Result<Self, Self::Error> {
        let host = value
            .get("host")
            .and_then(|x| x.as_str())
            .unwrap_or("bing.com");
        let mode = value
            .get("mode")
            .and_then(|x| x.as_str())
            .ok_or(Error::InvalidConfig("obfs mode is required".to_owned()))?;
        let port = value
            .get("port")
            .and_then(|x| x.as_u64())
            .ok_or(Error::InvalidConfig("obfs port is required".to_owned()))?
            as u16;

        if mode != "websocket" {
            return Err(Error::InvalidConfig(format!("invalid obfs mode: {mode}")));
        }

        let path = value.get("path").and_then(|x| x.as_str()).unwrap_or("");
        let mux = value.get("mux").and_then(|x| x.as_bool()).unwrap_or(false);
        let tls = value.get("tls").and_then(|x| x.as_bool()).unwrap_or(false);
        let skip_cert_verify = value
            .get("skip-cert-verify")
            .and_then(|x| x.as_bool())
            .unwrap_or(false);

        let mut headers = HashMap::new();
        if let Some(h) = value.get("headers")
            && let Some(h) = h.as_mapping()
        {
            for (k, v) in h {
                if let (Some(k), Some(v)) = (k.as_str(), v.as_str()) {
                    headers.insert(k.to_owned(), v.to_owned());
                }
            }
        }

        Ok(V2RayOBFSOption {
            mode: mode.to_owned(),
            host: host.to_owned(),
            port,
            path: path.to_owned(),
            tls,
            headers,
            skip_cert_verify,
            mux,
        })
    }
}

impl TryFrom<HashMap<String, serde_yaml::Value>> for Shadowtls {
    type Error = crate::Error;

    fn try_from(
        value: HashMap<String, serde_yaml::Value>,
    ) -> Result<Self, Self::Error> {
        let host = value
            .get("host")
            .and_then(|x| x.as_str())
            .unwrap_or("bing.com");
        let password = value
            .get("password")
            .and_then(|x| x.as_str().to_owned())
            .ok_or(Error::InvalidConfig("obfs mode is required".to_owned()))?;
        let strict = value
            .get("strict")
            .and_then(|x| x.as_bool())
            .unwrap_or(true);

        Ok(Shadowtls::new(
            host.to_string(),
            password.to_string(),
            strict,
        ))
    }
}
