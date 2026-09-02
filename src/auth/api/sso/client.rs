use quick_xml::de::from_str;
use reqwest::header::{HeaderMap, HeaderValue, HOST, USER_AGENT};
use reqwest::Client;
use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;
use url::Url;

use quick_xml::de::DeError;

#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum CasError {
    #[error("HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Failed to parse CAS XML: {0}")]
    Xml(#[from] DeError), // ← use DeError instead of quick_xml::Error
    #[error("Ticket invalid or authentication failed")]
    InvalidTicket,
    #[error("Unexpected response from CAS server")]
    UnexpectedResponse,
}

#[derive(Debug, Deserialize)]
#[serde(rename = "serviceResponse")] // root element
pub struct CasServiceResponse {
    #[serde(rename = "authenticationSuccess")]
    pub success: Option<CasAuthenticationSuccess>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CasAuthenticationSuccess {
    #[serde(rename = "user")]
    pub username: String,

    #[serde(rename = "attributes")]
    pub attributes: Option<CasAttributes>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CasAttributes {
    #[serde(rename = "ldap_cn")]
    pub ldap_cn: Option<String>,

    #[serde(rename = "kd_org")]
    pub kd_org: Option<String>,

    #[serde(rename = "peran_user")]
    pub peran_user: Option<String>,

    #[serde(rename = "nama")]
    pub nama: Option<String>,

    #[serde(rename = "npm")]
    pub npm: Option<String>,
}

/// CAS client v2
pub struct CasClient {
    pub service_url: String,
    pub server_url: String,
    client: Client,
    pub proxy_callback: Option<String>,
}

impl CasClient {
    /// Create a new CAS client
    pub fn new(
        service_url: impl Into<String>,
        server_url: impl Into<String>,
        proxy_callback: Option<String>,
    ) -> Self {
        Self {
            service_url: service_url.into(),
            server_url: server_url.into(),
            client: Client::new(),
            proxy_callback,
        }
    }

    /// Generate CAS login URL
    #[allow(dead_code)]
    pub fn login_url(&self, renew: bool) -> String {
        let mut url = Url::parse(&self.server_url)
            .expect("Invalid CAS server URL")
            .join("login")
            .unwrap();
        url.query_pairs_mut()
            .append_pair("service", &self.service_url);
        if renew {
            url.query_pairs_mut().append_pair("renew", "true");
        }
        url.to_string()
    }

    /// Generate CAS logout URL
    #[allow(dead_code)]
    pub fn logout_url(&self, redirect: Option<&str>) -> String {
        let mut url = Url::parse(&self.server_url)
            .expect("Invalid CAS server URL")
            .join("logout")
            .unwrap();
        if let Some(redirect_url) = redirect {
            url.query_pairs_mut().append_pair("service", redirect_url);
        }
        url.to_string()
    }

    /// Verify a CAS ticket
    pub async fn verify_ticket(&self, ticket: &str) -> Result<CasAuthenticationSuccess, CasError> {
        let mut url = Url::parse(&self.server_url)
            .expect("Invalid CAS server URL")
            .join("serviceValidate")
            .unwrap();
        url.query_pairs_mut()
            .append_pair("service", &self.service_url)
            .append_pair("ticket", ticket);
        if let Some(callback) = &self.proxy_callback {
            url.query_pairs_mut().append_pair("pgtUrl", callback);
        }

        // Build headers
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static("RustCASClient/0.1"));
        headers.insert(HOST, HeaderValue::from_static("sso.ui.ac.id"));

        let resp_text = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await?
            .text()
            .await?;

        let parsed: CasServiceResponse = from_str(&resp_text)?;
        if let Some(success) = parsed.success {
            Ok(success)
        } else {
            Err(CasError::InvalidTicket)
        }
    }
}

/// Proxy ticket response
#[derive(Debug, Deserialize)]
struct ProxyResponse {
    #[serde(rename = "proxyTicket")]
    pub proxy_ticket: Option<String>,
}
