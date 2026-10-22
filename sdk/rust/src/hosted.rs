use crate::{
    Error, Pubkey, Result, Signer,
    accounts::DocumentRef,
    ensure,
    instruction::Protocol,
    metadata::{self, Document},
};
use serde::{Deserialize, Serialize};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub type WalletSigner = Arc<dyn Signer + Send + Sync>;
pub const METADATA_PROVIDER_PATH: &str = "/v1/providers/metadata/provider";
pub const METADATA_MODEL_PATH: &str = "/v1/providers/metadata/model";
pub const IMAGE_PATH: &str = "/v1/providers/images";
pub const RPC_PATH: &str = "/v1/providers/rpc";

pub fn http_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .pool_idle_timeout(Duration::from_secs(90))
        .tcp_nodelay(true)
        .tcp_keepalive(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}
pub(crate) async fn response_bytes(mut response: reqwest::Response, max: usize) -> Result<Vec<u8>> {
    if !response.status().is_success() {
        return Err(Error::HttpStatus(response.status().as_u16()));
    }
    ensure(
        response.content_length().is_none_or(|n| n <= max as u64),
        "HTTP response size",
    )?;
    let mut bytes =
        Vec::with_capacity(response.content_length().unwrap_or(0).min(max as u64) as usize);
    while let Some(chunk) = response.chunk().await? {
        ensure(bytes.len() + chunk.len() <= max, "HTTP response size")?;
        bytes.extend(chunk);
    }
    Ok(bytes)
}
pub fn validate_api_origin(api: &str) -> Result<String> {
    let url = reqwest::Url::parse(api).map_err(|_| Error::Invalid("provider API URL"))?;
    ensure(
        url.scheme() == "https"
            || (url.scheme() == "http"
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))),
        "provider API HTTPS URL",
    )?;
    ensure(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/",
        "provider API origin",
    )?;
    Ok(api.trim_end_matches('/').to_owned())
}
pub fn signing_message(
    api: &str,
    path: &str,
    wallet: &Pubkey,
    expires: u64,
    nonce: &str,
    body: &[u8],
) -> Result<String> {
    let api = validate_api_origin(api)?;
    ensure(
        matches!(
            path,
            METADATA_PROVIDER_PATH | METADATA_MODEL_PATH | IMAGE_PATH | RPC_PATH
        ),
        "provider request path",
    )?;
    ensure(
        nonce.len() == 32
            && nonce
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)),
        "provider request nonce",
    )?;
    Ok(format!(
        "fossa-provider-v1\n{api}\nPOST\n{path}\n{wallet}\n{expires}\n{nonce}\n{}",
        metadata::sha256_hex(body)
    ))
}
#[derive(Clone)]
pub struct HostedClient {
    http: reqwest::Client,
    api: String,
    signer: WalletSigner,
}
impl std::fmt::Debug for HostedClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostedClient")
            .field("api", &self.api)
            .finish_non_exhaustive()
    }
}
impl HostedClient {
    pub fn new(api: &str, signer: WalletSigner) -> Result<Self> {
        Self::with_http(api, signer, http_client()?)
    }
    pub fn with_http(api: &str, signer: WalletSigner, http: reqwest::Client) -> Result<Self> {
        Ok(Self {
            http,
            api: validate_api_origin(api)?,
            signer,
        })
    }
    pub fn api_url(&self) -> &str {
        &self.api
    }
    pub fn wallet(&self) -> Result<Pubkey> {
        self.signer
            .try_pubkey()
            .map_err(|e| Error::Signing(e.to_string()))
    }
    pub async fn post(
        &self,
        path: &str,
        content_type: &str,
        body: &[u8],
        max_response: usize,
    ) -> Result<Vec<u8>> {
        let expires = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::Invalid("system time"))?
            .as_secs()
            + 120;
        let nonce = metadata::hex(&rand::random::<[u8; 16]>());
        let wallet = self.wallet()?;
        let message = signing_message(&self.api, path, &wallet, expires, &nonce, body)?;
        let signature = self
            .signer
            .try_sign_message(message.as_bytes())
            .map_err(|e| Error::Signing(e.to_string()))?;
        ensure(
            signature.verify(wallet.as_ref(), message.as_bytes()),
            "wallet signature",
        )?;
        let response = self
            .http
            .post(format!("{}{path}", self.api))
            .header("content-type", content_type)
            .header("x-fossa-wallet", wallet.to_string())
            .header("x-fossa-expires", expires.to_string())
            .header("x-fossa-nonce", nonce)
            .header("x-fossa-signature", signature.to_string())
            .body(body.to_vec())
            .send()
            .await?;
        response_bytes(response, max_response).await
    }
    pub async fn upload_metadata(&self, document: &Document) -> Result<Upload> {
        let path = match document.kind() {
            metadata::Kind::Provider => METADATA_PROVIDER_PATH,
            metadata::Kind::Model => METADATA_MODEL_PATH,
        };
        self.upload(path, "application/json", document.bytes())
            .await
    }
    pub async fn upload_image(&self, content_type: &str, bytes: &[u8]) -> Result<Upload> {
        ensure(
            !bytes.is_empty() && bytes.len() <= 2 * 1024 * 1024,
            "image size",
        )?;
        let valid = match content_type {
            "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
            "image/webp" => bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
            _ => false,
        };
        ensure(valid, "PNG, JPEG or WebP image")?;
        self.upload(IMAGE_PATH, content_type, bytes).await
    }
    async fn upload(&self, path: &str, content_type: &str, body: &[u8]) -> Result<Upload> {
        let upload: Upload =
            serde_json::from_slice(&self.post(path, content_type, body, 4096).await?)?;
        ensure(
            upload.sha256 == metadata::sha256_hex(body) && upload.bytes == body.len(),
            "uploaded content hash or size",
        )?;
        let prefix = if path == IMAGE_PATH {
            "/v1/providers/images/"
        } else {
            "/v1/market/metadata/"
        };
        ensure(
            upload.uri == format!("{}{prefix}{}", self.api, upload.sha256),
            "uploaded content URI",
        )?;
        Ok(upload)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Upload {
    pub sha256: String,
    pub uri: String,
    pub bytes: usize,
}
impl Upload {
    pub fn reference(&self) -> Result<DocumentRef> {
        ensure(
            self.sha256.len() == 64
                && self
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "uploaded hash",
        )?;
        let mut hash = [0; 32];
        for (n, byte) in hash.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&self.sha256[n * 2..n * 2 + 2], 16)
                .map_err(|_| Error::Invalid("uploaded hash"))?;
        }
        DocumentRef::new(hash, self.uri.clone())
    }
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicConfig {
    #[serde(with = "crate::serde_pubkey")]
    pub program_id: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub usdc_mint: Pubkey,
    pub provider_api: Option<String>,
    pub provider_rpc: Option<String>,
}
impl PublicConfig {
    pub fn protocol(&self) -> Protocol {
        Protocol {
            program: self.program_id,
            market: self.market,
            mint: self.usdc_mint,
        }
    }
    pub async fn fetch(api: &str) -> Result<Self> {
        let base = validate_api_origin(api)?;
        let bytes = response_bytes(
            http_client()?
                .get(format!("{base}/v1/config"))
                .send()
                .await?,
            64 * 1024,
        )
        .await?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}
