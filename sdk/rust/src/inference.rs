use crate::{
    Error, Pubkey, Result, Signature, Signer,
    accounts::{Booking, BookingStatus},
    ensure,
    instruction::Protocol,
    metadata::sha256_hex,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authorization {
    pub booking: Pubkey,
    pub owner: Pubkey,
    pub generation: u64,
    pub body_sha256: String,
    pub signature: Signature,
}

#[derive(Clone, Debug)]
pub struct VerifiedAuthorization {
    authorization: Authorization,
    market: Pubkey,
    provider: Pubkey,
}
#[derive(Clone, Debug)]
pub struct RequestVerifier {
    market: Pubkey,
    provider: Pubkey,
    provider_text: String,
    prefix: String,
    keys: Arc<RwLock<HashMap<Pubkey, ed25519_dalek::VerifyingKey>>>,
}
impl RequestVerifier {
    pub fn new(protocol: &Protocol, provider: Pubkey) -> Self {
        Self {
            market: protocol.market,
            provider,
            provider_text: provider.to_string(),
            prefix: format!("fossa-infer-v2:{}:{}:", protocol.program, protocol.market),
            keys: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    pub fn verify(&self, headers: &HeaderMap, body: &[u8]) -> Result<VerifiedAuthorization> {
        let authorization = Authorization::from_headers(headers)?;
        ensure(
            authorization.body_sha256 == sha256_hex(body),
            "inference body hash",
        )?;
        let booking = authorization.booking;
        let generation = authorization.generation;
        let hash = &authorization.body_sha256;
        let provider = &self.provider_text;
        let message = if generation == 0 {
            format!("fossa-infer-v1:{booking}:{provider}:{hash}")
        } else {
            format!("{}{booking}:{generation}:{provider}:{hash}", self.prefix)
        };
        let cached = self.keys.read().unwrap().get(&authorization.owner).copied();
        let key = match cached {
            Some(key) => key,
            None => ed25519_dalek::VerifyingKey::from_bytes(&authorization.owner.to_bytes())
                .map_err(|_| Error::Invalid("inference owner key"))?,
        };
        key.verify_strict(
            message.as_bytes(),
            &ed25519_dalek::Signature::from_bytes(authorization.signature.as_array()),
        )
        .map_err(|_| Error::Invalid("inference signature"))?;
        if cached.is_none() {
            let mut keys = self.keys.write().unwrap();
            if keys.len() >= 1024 {
                let evicted = *keys.keys().next().unwrap();
                keys.remove(&evicted);
            }
            keys.insert(authorization.owner, key);
        }
        Ok(VerifiedAuthorization {
            authorization,
            market: self.market,
            provider: self.provider,
        })
    }
}
impl VerifiedAuthorization {
    pub fn authorization(&self) -> &Authorization {
        &self.authorization
    }
    pub fn verify_booking(&self, booking: Pubkey, state: &Booking, now: i64) -> Result<()> {
        self.authorization
            .verify_binding(self.market, self.provider, booking, state, now)
    }
}

pub fn statement(
    protocol: &Protocol,
    booking: Pubkey,
    provider: Pubkey,
    generation: u64,
    body: &[u8],
) -> Result<String> {
    statement_hash(protocol, booking, provider, generation, &sha256_hex(body))
}
fn statement_hash(
    protocol: &Protocol,
    booking: Pubkey,
    provider: Pubkey,
    generation: u64,
    hash: &str,
) -> Result<String> {
    ensure(generation < (1u64 << 56), "inference generation")?;
    Ok(if generation == 0 {
        format!("fossa-infer-v1:{booking}:{provider}:{hash}")
    } else {
        format!(
            "fossa-infer-v2:{}:{}:{booking}:{generation}:{provider}:{hash}",
            protocol.program, protocol.market
        )
    })
}
impl Authorization {
    pub fn sign(
        protocol: &Protocol,
        booking: Pubkey,
        state: &Booking,
        body: &[u8],
        signer: &dyn Signer,
        now: i64,
    ) -> Result<Self> {
        let owner = signer
            .try_pubkey()
            .map_err(|e| Error::Signing(e.to_string()))?;
        ensure(owner == state.owner, "booking owner signer")?;
        let body_sha256 = sha256_hex(body);
        let message = statement_hash(
            protocol,
            booking,
            state.provider,
            state.generation,
            &body_sha256,
        )?;
        let value = Self {
            booking,
            owner,
            generation: state.generation,
            body_sha256,
            signature: signer
                .try_sign_message(message.as_bytes())
                .map_err(|e| Error::Signing(e.to_string()))?,
        };
        value.verify_binding(protocol.market, state.provider, booking, state, now)?;
        value.verify_message(&message)?;
        Ok(value)
    }
    pub fn verify(
        &self,
        protocol: &Protocol,
        booking: Pubkey,
        state: &Booking,
        body: &[u8],
        now: i64,
    ) -> Result<()> {
        self.verify_binding(protocol.market, state.provider, booking, state, now)?;
        self.verify_signature(protocol, state.provider, body)
            .map(|_| ())
    }
    fn verify_binding(
        &self,
        market: Pubkey,
        provider: Pubkey,
        booking: Pubkey,
        state: &Booking,
        now: i64,
    ) -> Result<()> {
        ensure(
            self.booking == booking
                && state.status == BookingStatus::Reserved
                && !state.is_receipt_case
                && state.market == market
                && state.provider == provider
                && state.owner == self.owner
                && state.generation == self.generation
                && now < state.claim_end,
            "inference booking binding or deadline",
        )
    }
    // authenticate before fetching accounts; the verified value binds the later booking check.
    pub fn verify_signature(
        &self,
        protocol: &Protocol,
        provider: Pubkey,
        body: &[u8],
    ) -> Result<VerifiedAuthorization> {
        ensure(self.body_sha256 == sha256_hex(body), "inference body hash")?;
        let message = statement_hash(
            protocol,
            self.booking,
            provider,
            self.generation,
            &self.body_sha256,
        )?;
        self.verify_message(&message)?;
        Ok(VerifiedAuthorization {
            authorization: self.clone(),
            market: protocol.market,
            provider,
        })
    }
    fn verify_message(&self, message: &str) -> Result<()> {
        ensure(
            self.signature
                .verify(self.owner.as_ref(), message.as_bytes()),
            "inference signature",
        )
    }
    pub fn headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        for (name, value) in [
            ("x-fossa-booking", self.booking.to_string()),
            ("x-fossa-user", self.owner.to_string()),
            ("x-fossa-generation", self.generation.to_string()),
            ("x-fossa-body-sha256", self.body_sha256.clone()),
            (
                "x-fossa-signature",
                STANDARD.encode(self.signature.as_ref()),
            ),
        ] {
            headers.insert(
                name,
                HeaderValue::from_str(&value).map_err(|_| Error::Invalid("inference header"))?,
            );
        }
        Ok(headers)
    }
    pub fn from_headers(headers: &HeaderMap) -> Result<Self> {
        let get = |name: &str| {
            headers
                .get(name)
                .and_then(|v| v.to_str().ok())
                .ok_or(Error::Invalid("inference authorization header"))
        };
        let generation = headers
            .get("x-fossa-generation")
            .map(|v| {
                v.to_str()
                    .map_err(|_| Error::Invalid("inference generation"))
            })
            .transpose()?
            .unwrap_or("0");
        ensure(generation.len() <= 17, "inference generation length")?;
        let signature = get("x-fossa-signature")?;
        ensure(signature.len() == 88, "inference signature length")?;
        let mut bytes = [0; 66];
        let length = STANDARD
            .decode_slice(signature, &mut bytes)
            .map_err(|_| Error::Invalid("inference signature base64"))?;
        let body_sha256 = get("x-fossa-body-sha256")?;
        ensure(
            body_sha256.len() == 64
                && body_sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "inference body hash",
        )?;
        let generation = crate::amount::parse_atomic(generation)?;
        ensure(generation < (1u64 << 56), "inference generation")?;
        Ok(Self {
            booking: get("x-fossa-booking")?
                .parse()
                .map_err(|_| Error::Invalid("inference booking key"))?,
            owner: get("x-fossa-user")?
                .parse()
                .map_err(|_| Error::Invalid("inference owner key"))?,
            generation,
            body_sha256: body_sha256.into(),
            signature: Signature::try_from(&bytes[..length])
                .map_err(|_| Error::Invalid("inference signature bytes"))?,
        })
    }
    pub fn request(&self, endpoint: &str, body: Vec<u8>) -> Result<reqwest::Request> {
        ensure(self.body_sha256 == sha256_hex(&body), "inference body hash")?;
        let url = reqwest::Url::parse(endpoint).map_err(|_| Error::Invalid("provider endpoint"))?;
        ensure(
            url.scheme() == "https"
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none(),
            "provider HTTPS endpoint",
        )?;
        let mut request = reqwest::Request::new(reqwest::Method::POST, url);
        *request.headers_mut() = self.headers()?;
        *request.body_mut() = Some(body.into());
        Ok(request)
    }
}
