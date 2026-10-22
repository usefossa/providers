use crate::{Error, Result, accounts::DocumentRef, ensure};
pub use fossa_metadata::{Kind, MAX_BYTES, SCHEMA};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug)]
pub struct Document {
    kind: Kind,
    bytes: Vec<u8>,
    value: serde_json::Value,
    sha256: [u8; 32],
}
impl Document {
    pub fn parse(kind: Kind, bytes: impl Into<Vec<u8>>) -> Result<Self> {
        let bytes = bytes.into();
        let value = fossa_metadata::parse(kind, &bytes).map_err(Error::Metadata)?;
        let sha256 = Sha256::digest(&bytes).into();
        Ok(Self {
            kind,
            bytes,
            value,
            sha256,
        })
    }
    pub fn model(id: &str, bytes: impl Into<Vec<u8>>) -> Result<Self> {
        let doc = Self::parse(Kind::Model, bytes)?;
        ensure(doc.value["id"].as_str() == Some(id), "model metadata id")?;
        Ok(doc)
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn value(&self) -> &serde_json::Value {
        &self.value
    }
    pub fn kind(&self) -> Kind {
        self.kind
    }
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
    pub fn reference(&self, uri: impl Into<String>) -> Result<DocumentRef> {
        DocumentRef::new(self.sha256, uri)
    }
    pub fn verify(&self, reference: &DocumentRef) -> Result<()> {
        ensure(self.sha256 == reference.sha256, "metadata SHA-256")
    }
    pub async fn fetch(kind: Kind, reference: &DocumentRef) -> Result<Self> {
        let url = validate_uri(&reference.uri)?;
        let host = url
            .host_str()
            .ok_or(Error::Invalid("metadata host"))?
            .trim_matches(['[', ']']);
        let addresses = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            tokio::net::lookup_host((host, url.port_or_known_default().unwrap_or(443))),
        )
        .await
        .map_err(|_| Error::Invalid("metadata DNS timeout"))??
        .collect::<Vec<_>>();
        ensure(
            !addresses.is_empty() && addresses.iter().all(|a| public_ip(a.ip())),
            "public metadata host",
        )?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .resolve_to_addrs(host, &addresses)
            .build()?;
        let body = crate::hosted::response_bytes(http.get(url).send().await?, MAX_BYTES).await?;
        let doc = Self::parse(kind, body)?;
        doc.verify(reference)?;
        Ok(doc)
    }
}
pub fn validate_uri(uri: &str) -> Result<reqwest::Url> {
    DocumentRef::new([0; 32], uri)?;
    let url = reqwest::Url::parse(uri).map_err(|_| Error::Invalid("metadata URL"))?;
    let host = url.host_str().ok_or(Error::Invalid("metadata host"))?;
    ensure(
        url.scheme() == "https"
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && host.parse::<std::net::IpAddr>().is_err()
            && !host.starts_with('[')
            && host != "localhost"
            && !host.ends_with(".local"),
        "public metadata HTTPS URL on port 443",
    )?;
    Ok(url)
}
fn public_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            !ip.is_private()
                && !ip.is_loopback()
                && !ip.is_link_local()
                && !ip.is_documentation()
                && !ip.is_broadcast()
                && a != 0
                && a < 224
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 192 && b == 0)
                && !(a == 198 && (b == 18 || b == 19))
        }
        std::net::IpAddr::V6(ip) => {
            let s = ip.segments();
            (s[0] & 0xe000) == 0x2000
                && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
                && s[0] != 0x2002
        }
    }
}
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    for &b in bytes {
        s.push(DIGITS[usize::from(b >> 4)] as char);
        s.push(DIGITS[usize::from(b & 15)] as char);
    }
    s
}
