use crate::{
    Error, Pubkey, Result, Signature, Signer,
    accounts::{Booking, BookingStatus},
    ensure,
    instruction::Protocol,
    metadata::sha256_hex,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authorization {
    pub booking: Pubkey,
    pub owner: Pubkey,
    pub generation: u64,
    pub body_sha256: String,
    pub signature: Signature,
}

pub fn statement(
    protocol: &Protocol,
    booking: Pubkey,
    provider: Pubkey,
    generation: u64,
    body: &[u8],
) -> Result<String> {
    ensure(generation < (1u64 << 56), "inference generation")?;
    let hash = sha256_hex(body);
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
        let message = statement(protocol, booking, state.provider, state.generation, body)?;
        let value = Self {
            booking,
            owner,
            generation: state.generation,
            body_sha256: sha256_hex(body),
            signature: signer
                .try_sign_message(message.as_bytes())
                .map_err(|e| Error::Signing(e.to_string()))?,
        };
        value.verify(protocol, booking, state, body, now)?;
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
        ensure(
            self.booking == booking
                && state.status == BookingStatus::Reserved
                && !state.is_receipt_case
                && state.market == protocol.market
                && state.owner == self.owner
                && state.generation == self.generation
                && now <= state.claim_end,
            "inference booking binding or deadline",
        )?;
        ensure(self.body_sha256 == sha256_hex(body), "inference body hash")?;
        let message = statement(
            protocol,
            self.booking,
            state.provider,
            self.generation,
            body,
        )?;
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
        let signature = STANDARD
            .decode(get("x-fossa-signature")?)
            .map_err(|_| Error::Invalid("inference signature base64"))?;
        Ok(Self {
            booking: get("x-fossa-booking")?
                .parse()
                .map_err(|_| Error::Invalid("inference booking key"))?,
            owner: get("x-fossa-user")?
                .parse()
                .map_err(|_| Error::Invalid("inference owner key"))?,
            generation: crate::amount::parse_atomic(generation)?,
            body_sha256: get("x-fossa-body-sha256")?.into(),
            signature: Signature::try_from(signature.as_slice())
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
