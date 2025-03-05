use crate::config::proxy::{CommonConfigOptions, GrpcOpt, H2Opt, WsOpt};

#[derive(serde::Serialize, serde::Deserialize, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub struct OutboundVlessCustom {
    #[serde(flatten)]
    pub common_opts: CommonConfigOptions,
    pub uuid: String,
    pub alpn: Option<Vec<String>>,
    #[serde(alias = "servername")]
    pub server_name: Option<String>,
    pub tls: Option<bool>,
    pub skip_cert_verify: Option<bool>,
    pub udp: Option<bool>,
    pub network: Option<String>,
    pub ws_opts: Option<WsOpt>,
    pub h2_opts: Option<H2Opt>,
    pub grpc_opts: Option<GrpcOpt>,
}