use http::uri::InvalidUri;

use crate::{
    config::proxy::{CommonConfigOptions, GrpcOpt, H2Opt, HttpOpt, KcpOpt, WsOpt},
    proxy::transport::{self, GrpcClient, H2Client, HttpClient, HttpUpgradeClient, WsClient, H3Client, XhttpClient},
};

impl TryFrom<(&WsOpt, &CommonConfigOptions)> for WsClient {
    type Error = std::io::Error;

    fn try_from(pair: (&WsOpt, &CommonConfigOptions)) -> Result<Self, Self::Error> {
        let (x, common) = pair;
        let path = x.path.as_ref().map(|x| x.to_owned()).unwrap_or_default();
        let headers = x.headers.as_ref().map(|x| x.to_owned()).unwrap_or_default();
        let max_early_data = x.max_early_data.unwrap_or_default() as usize;
        let early_data_header_name = x
            .early_data_header_name
            .as_ref()
            .map(|x| x.to_owned())
            .unwrap_or_default();

        let client = transport::WsClient::new(
            common.server.to_owned(),
            common.port,
            path,
            headers,
            None,
            max_early_data,
            early_data_header_name,
        );
        Ok(client)
    }
}

impl TryFrom<(Option<String>, &GrpcOpt, &CommonConfigOptions)> for GrpcClient {
    type Error = InvalidUri;

    fn try_from(
        opt: (Option<String>, &GrpcOpt, &CommonConfigOptions),
    ) -> Result<Self, Self::Error> {
        let (sni, x, common) = opt;
        let client = transport::GrpcClient::new(
            sni.as_ref().unwrap_or(&common.server).to_owned(),
            x.grpc_service_name
                .as_ref()
                .map(|x| x.to_owned())
                .unwrap_or_default()
                .try_into()?,
        );
        Ok(client)
    }
}

impl TryFrom<(&H2Opt, &CommonConfigOptions)> for H2Client {
    type Error = InvalidUri;

    fn try_from(pair: (&H2Opt, &CommonConfigOptions)) -> Result<Self, Self::Error> {
        let (x, common) = pair;
        let host = x
            .host
            .as_ref()
            .map(|x| x.to_owned())
            .unwrap_or(vec![common.server.to_owned()]);
        let path = x.path.as_ref().map(|x| x.to_owned()).unwrap_or_default();

        Ok(H2Client::new(
            host,
            std::collections::HashMap::new(),
            http::Method::GET,
            path.try_into()?,
        ))
    }
}

impl TryFrom<(&HttpOpt, &CommonConfigOptions)> for HttpClient {
    type Error = http::uri::InvalidUri;

    fn try_from(pair: (&HttpOpt, &CommonConfigOptions)) -> Result<Self, Self::Error> {
        let (x, common) = pair;
        let path = x.path.as_ref().cloned().unwrap_or_else(|| "/".to_owned());
        let method = x
            .method
            .as_ref()
            .map(|m| if m.eq_ignore_ascii_case("get") { http::Method::GET } else { http::Method::POST })
            .unwrap_or(http::Method::POST);
        let headers = x.headers.as_ref().cloned().unwrap_or_default();
        Ok(HttpClient::new(
            common.server.to_owned(),
            path.try_into()?,
            method,
            headers,
        ))
    }
}

impl TryFrom<(&HttpOpt, &CommonConfigOptions)> for HttpUpgradeClient {
    type Error = http::uri::InvalidUri;

    fn try_from(pair: (&HttpOpt, &CommonConfigOptions)) -> Result<Self, Self::Error> {
        let (x, common) = pair;
        let path = x.path.as_ref().cloned().unwrap_or_else(|| "/".to_owned());
        let headers = x.headers.as_ref().cloned().unwrap_or_default();
        Ok(HttpUpgradeClient::new(common.server.to_owned(), path.try_into()?, headers))
    }
}

impl TryFrom<(&HttpOpt, &CommonConfigOptions)> for H3Client {
    type Error = http::uri::InvalidUri;

    fn try_from(pair: (&HttpOpt, &CommonConfigOptions)) -> Result<Self, Self::Error> {
        let (_x, common) = pair;
        Ok(transport::H3Client::new(common.server.to_owned(), common.port, Default::default()))
    }
}

impl TryFrom<(&HttpOpt, &CommonConfigOptions)> for XhttpClient {
    type Error = http::uri::InvalidUri;

    fn try_from(pair: (&HttpOpt, &CommonConfigOptions)) -> Result<Self, Self::Error> {
        let (_x, common) = pair;
        Ok(transport::XhttpClient::new(common.server.to_owned(), common.port, Default::default()))
    }
}

// Placeholder mapping for KCP options (transport not yet implemented)
impl TryFrom<(&KcpOpt, &CommonConfigOptions)> for crate::proxy::transport::KcpClient {
    type Error = std::io::Error;
    fn try_from(_pair: (&KcpOpt, &CommonConfigOptions)) -> Result<Self, Self::Error> {
        Ok(crate::proxy::transport::KcpClient::new())
    }
}
