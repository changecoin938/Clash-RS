use crate::{
    config::internal::proxy::OutboundHttp,
    proxy::http::outbound::{Handler, HandlerOptions},
    proxy::HandlerCommonOptions,
};

impl TryFrom<OutboundHttp> for Handler {
    type Error = crate::Error;

    fn try_from(value: OutboundHttp) -> Result<Self, Self::Error> {
        (&value).try_into()
    }
}

impl TryFrom<&OutboundHttp> for Handler {
    type Error = crate::Error;

    fn try_from(s: &OutboundHttp) -> Result<Self, Self::Error> {
        Ok(Handler::new(HandlerOptions {
            name: s.common_opts.name.clone(),
            common_opts: HandlerCommonOptions {
                connector: s.common_opts.connect_via.clone(),
                ..Default::default()
            },
            server: s.common_opts.server.clone(),
            port: s.common_opts.port,
            username: s.username.clone(),
            password: s.password.clone(),
            tls: s.tls.unwrap_or_default(),
            sni: s.sni.clone(),
            skip_cert_verify: s.skip_cert_verify.unwrap_or_default(),
        }))
    }
}


