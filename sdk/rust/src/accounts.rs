use crate::{
    Pubkey, Result, address,
    amount::{Rates, Usage},
    ensure,
};
use serde::{Deserialize, Serialize};

pub const MARKET_LEN: usize = 72;
pub const MODEL_LEN: usize = 144;
pub const PROVIDER_LEN: usize = 364;
pub const MODEL_METADATA_LEN: usize = 396;
pub const USER_LEN: usize = 120;
pub const BOOKING_LEN: usize = 368;
pub const UNBOND_LEN: usize = 160;
pub const HEADER_LEN: usize = 48;
pub const TERMS_LEN: usize = 120;
pub const CAPACITY_LEN: usize = 32;
pub const MAX_ENTRIES: u32 = 4096;
pub const RECEIPT_BUCKET_LEN: usize = 736;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RawAccount {
    #[serde(with = "crate::serde_pubkey")]
    pub owner: Pubkey,
    pub lamports: u64,
    pub executable: bool,
    pub data: Vec<u8>,
}

pub trait Decode: Sized {
    const MAGIC: &'static [u8; 8];
    fn decode(address: &Pubkey, account: &RawAccount, program: &Pubkey) -> Result<Self>;
}

fn state<'a>(
    account: &'a RawAccount,
    program: &Pubkey,
    magic: &[u8; 8],
    len: usize,
) -> Result<&'a [u8]> {
    ensure(
        account.owner == *program
            && !account.executable
            && account.data.len() == len
            && account.data.get(..8) == Some(magic),
        "account owner, size or discriminator",
    )?;
    Ok(&account.data)
}
pub(crate) fn key(d: &[u8], at: usize) -> Pubkey {
    Pubkey::new_from_array(d[at..at + 32].try_into().unwrap())
}
pub(crate) fn n16(d: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(d[at..at + 2].try_into().unwrap())
}
pub(crate) fn n32(d: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(d[at..at + 4].try_into().unwrap())
}
pub(crate) fn n64(d: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(d[at..at + 8].try_into().unwrap())
}
pub(crate) fn i64le(d: &[u8], at: usize) -> i64 {
    i64::from_le_bytes(d[at..at + 8].try_into().unwrap())
}
fn flag(d: &[u8], at: usize) -> Result<bool> {
    ensure(d[at] <= 1, "boolean flag")?;
    Ok(d[at] == 1)
}
fn rates(d: &[u8], at: usize) -> Rates {
    Rates {
        input: n32(d, at),
        cached: n32(d, at + 4),
        output: n32(d, at + 8),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentRef {
    pub sha256: [u8; 32],
    pub uri: String,
}
impl DocumentRef {
    pub fn new(sha256: [u8; 32], uri: impl Into<String>) -> Result<Self> {
        let uri = uri.into();
        ensure(
            !uri.is_empty() && uri.len() <= 256 && !uri.bytes().any(|b| b <= 32 || b == 127),
            "metadata URI",
        )?;
        Ok(Self { sha256, uri })
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        Self::new(self.sha256, self.uri.clone())?;
        Ok([self.sha256.as_slice(), self.uri.as_bytes()].concat())
    }
    fn decode(d: &[u8], hash: usize, uri: usize) -> Result<Self> {
        let len = usize::from(n16(d, hash + 32));
        ensure((1..=256).contains(&len), "metadata URI length")?;
        ensure(
            d[uri + len..].iter().all(|&b| b == 0),
            "metadata URI padding",
        )?;
        let uri = std::str::from_utf8(&d[uri..uri + len])
            .map_err(|_| crate::Error::Invalid("metadata URI UTF-8"))?;
        Self::new(d[hash..hash + 32].try_into().unwrap(), uri)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Market {
    #[serde(with = "crate::serde_pubkey")]
    pub admin: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub mint: Pubkey,
}
impl Decode for Market {
    const MAGIC: &'static [u8; 8] = b"FOSSAV20";
    fn decode(_: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let d = state(a, p, Self::MAGIC, MARKET_LEN)?;
        Ok(Self {
            admin: key(d, 8),
            mint: key(d, 40),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Model {
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    pub id: [u8; 32],
    #[serde(with = "crate::serde_pubkey")]
    pub terms: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub capacity: Pubkey,
    pub limit: u32,
    pub bump: u8,
}
impl Decode for Model {
    const MAGIC: &'static [u8; 8] = b"MODELV20";
    fn decode(at: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let d = state(a, p, Self::MAGIC, MODEL_LEN)?;
        let value = Self {
            market: key(d, 8),
            id: d[40..72].try_into().unwrap(),
            terms: key(d, 72),
            capacity: key(d, 104),
            limit: n32(d, 136),
            bump: d[140],
        };
        ensure(
            (1..=MAX_ENTRIES).contains(&value.limit) && value.terms != value.capacity,
            "model book bindings",
        )?;
        ensure(
            address::model(p, &value.market, &value.id) == (*at, value.bump),
            "model PDA",
        )?;
        Ok(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Provider {
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub authority: Pubkey,
    pub document: DocumentRef,
    pub approved: bool,
    pub bump: u8,
}
impl Decode for Provider {
    const MAGIC: &'static [u8; 8] = b"PROV0V20";
    fn decode(at: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let d = state(a, p, Self::MAGIC, PROVIDER_LEN)?;
        let value = Self {
            market: key(d, 8),
            authority: key(d, 40),
            document: DocumentRef::decode(d, 72, 108)?,
            approved: flag(d, 106)?,
            bump: d[107],
        };
        ensure(
            address::provider(p, &value.market, &value.authority) == (*at, value.bump),
            "provider PDA",
        )?;
        Ok(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelMetadata {
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub model: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub authority: Pubkey,
    pub document: DocumentRef,
    pub bump: u8,
}
impl Decode for ModelMetadata {
    const MAGIC: &'static [u8; 8] = b"MMETAV20";
    fn decode(at: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let d = state(a, p, Self::MAGIC, MODEL_METADATA_LEN)?;
        let value = Self {
            market: key(d, 8),
            model: key(d, 40),
            authority: key(d, 72),
            document: DocumentRef::decode(d, 104, 140)?,
            bump: d[138],
        };
        ensure(
            address::model_metadata(p, &value.model) == (*at, value.bump),
            "model metadata PDA",
        )?;
        Ok(value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct User {
    #[serde(with = "crate::serde_pubkey")]
    pub owner: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    pub free: u64,
    #[serde(with = "crate::serde_pubkey")]
    pub vault: Pubkey,
    pub bump: u8,
    pub generation: u64,
}
impl Decode for User {
    const MAGIC: &'static [u8; 8] = b"USER0V20";
    fn decode(at: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let d = state(a, p, Self::MAGIC, USER_LEN)?;
        let mut generation = [0; 8];
        generation[..7].copy_from_slice(&d[113..120]);
        let value = Self {
            owner: key(d, 8),
            market: key(d, 40),
            free: n64(d, 72),
            vault: key(d, 80),
            bump: d[112],
            generation: u64::from_le_bytes(generation),
        };
        ensure(address::vault(p, at).1 == value.bump, "user vault PDA bump")?;
        Ok(value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfferTerms {
    pub rates: Rates,
    pub speed_tps: u16,
    pub private: bool,
    pub active: bool,
    pub capacity: u32,
}
impl OfferTerms {
    pub fn validate(self) -> Result<()> {
        self.rates.validate()?;
        ensure(
            self.speed_tps > 0 && self.capacity > 0,
            "offer speed or capacity",
        )
    }
    pub fn encode(self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut out = Vec::with_capacity(20);
        self.rates.encode(&mut out);
        out.extend(self.speed_tps.to_le_bytes());
        out.extend([u8::from(self.private), u8::from(self.active)]);
        out.extend(self.capacity.to_le_bytes());
        Ok(out)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Offer {
    #[serde(with = "crate::serde_pubkey")]
    pub provider: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub payout: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub bond_vault: Pubkey,
    pub terms: OfferTerms,
    pub approved: bool,
    pub bond_bump: u8,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Capacity {
    pub funded_bond: u64,
    pub exposure: u64,
    pub active: u32,
    pub obligations: u32,
    pub pending_unbond: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TermsBook {
    #[serde(with = "crate::serde_pubkey")]
    pub model: Pubkey,
    pub limit: u32,
    pub entries: Vec<Offer>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CapacityBook {
    #[serde(with = "crate::serde_pubkey")]
    pub model: Pubkey,
    pub limit: u32,
    pub entries: Vec<Capacity>,
}
fn book<'a>(
    a: &'a RawAccount,
    p: &Pubkey,
    magic: &[u8; 8],
    size: usize,
) -> Result<(&'a [u8], Pubkey, u32, usize)> {
    ensure(a.data.len() >= HEADER_LEN, "book header")?;
    let limit = n32(&a.data, 40);
    let count = n32(&a.data, 44);
    ensure(
        (1..=MAX_ENTRIES).contains(&limit) && count <= limit,
        "book limit or count",
    )?;
    let d = state(a, p, magic, HEADER_LEN + limit as usize * size)?;
    Ok((d, key(d, 8), limit, count as usize))
}
impl Decode for TermsBook {
    const MAGIC: &'static [u8; 8] = b"TERMSV20";
    fn decode(_: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let (d, model, limit, count) = book(a, p, Self::MAGIC, TERMS_LEN)?;
        let mut entries = Vec::with_capacity(count);
        for (slot, t) in d[48..48 + count * TERMS_LEN]
            .chunks_exact(TERMS_LEN)
            .enumerate()
        {
            let terms = OfferTerms {
                rates: rates(t, 96),
                speed_tps: n16(t, 108),
                private: flag(t, 110)?,
                active: flag(t, 111)?,
                capacity: n32(t, 116),
            };
            terms.validate()?;
            ensure(
                address::bond(p, &model, slot as u32).1 == t[113],
                "bond PDA bump",
            )?;
            entries.push(Offer {
                provider: key(t, 0),
                payout: key(t, 32),
                bond_vault: key(t, 64),
                terms,
                approved: flag(t, 112)?,
                bond_bump: t[113],
            });
        }
        Ok(Self {
            model,
            limit,
            entries,
        })
    }
}
impl Decode for CapacityBook {
    const MAGIC: &'static [u8; 8] = b"CAPACV20";
    fn decode(_: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let (d, model, limit, count) = book(a, p, Self::MAGIC, CAPACITY_LEN)?;
        let entries = d[48..48 + count * CAPACITY_LEN]
            .chunks_exact(CAPACITY_LEN)
            .map(|c| Capacity {
                funded_bond: n64(c, 0),
                exposure: n64(c, 8),
                active: n32(c, 16),
                obligations: n32(c, 20),
                pending_unbond: n64(c, 24),
            })
            .collect();
        Ok(Self {
            model,
            limit,
            entries,
        })
    }
}
impl TermsBook {
    pub fn validate_pair(&self, capacity: &CapacityBook, at: &Pubkey, model: &Model) -> Result<()> {
        ensure(
            self.model == *at
                && capacity.model == *at
                && self.limit == model.limit
                && capacity.limit == model.limit
                && self.entries.len() == capacity.entries.len(),
            "model offer books",
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[repr(u8)]
pub enum BookingStatus {
    Reserved = 0,
    Claimed = 1,
    Disputed = 2,
    Released = 3,
    Finalized = 4,
    Available = 5,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Booking {
    #[serde(with = "crate::serde_pubkey")]
    pub owner: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub provider: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    pub reserve: u64,
    pub rates: Rates,
    pub max_input: u32,
    pub max_output: u32,
    pub claim_end: i64,
    pub dispute_end: i64,
    pub status: BookingStatus,
    pub generation: u64,
    pub charge: u64,
    pub usage: Usage,
    pub min_speed: u16,
    #[serde(with = "crate::serde_pubkey")]
    pub user: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub model: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub rent_payer: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub payout: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub bond_vault: Pubkey,
    pub slot: u32,
    pub bond_bump: u8,
    pub requested_privacy: bool,
    pub cache_allowed: bool,
    pub is_receipt_case: bool,
    pub estimated_score: u64,
}
impl Decode for Booking {
    const MAGIC: &'static [u8; 8] = b"BOOK0V20";
    fn decode(_: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let d = state(a, p, Self::MAGIC, BOOKING_LEN)?;
        let status = match d[148] {
            0 => BookingStatus::Reserved,
            1 => BookingStatus::Claimed,
            2 => BookingStatus::Disputed,
            3 => BookingStatus::Released,
            4 => BookingStatus::Finalized,
            5 => BookingStatus::Available,
            _ => return Err(crate::Error::Invalid("booking status")),
        };
        ensure(d[359] == 0 || d[359] == 0xa5, "booking case marker")?;
        Ok(Self {
            owner: key(d, 8),
            provider: key(d, 40),
            market: key(d, 72),
            reserve: n64(d, 104),
            rates: rates(d, 112),
            max_input: n32(d, 124),
            max_output: n32(d, 128),
            claim_end: i64le(d, 132),
            dispute_end: i64le(d, 140),
            status,
            generation: n64(d, 152),
            charge: n64(d, 160),
            usage: Usage {
                uncached: n32(d, 168),
                cached: n32(d, 172),
                output: n32(d, 176),
            },
            min_speed: n16(d, 180),
            user: key(d, 192),
            model: key(d, 224),
            rent_payer: key(d, 256),
            payout: key(d, 288),
            bond_vault: key(d, 320),
            slot: n32(d, 352),
            bond_bump: d[356],
            requested_privacy: flag(d, 357)?,
            cache_allowed: flag(d, 358)?,
            is_receipt_case: d[359] == 0xa5,
            estimated_score: n64(d, 360),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Unbond {
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub provider: Pubkey,
    pub amount: u64,
    pub ready: i64,
    pub executed: bool,
    pub slot: u32,
    #[serde(with = "crate::serde_pubkey")]
    pub model: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub rent_payer: Pubkey,
}
impl Decode for Unbond {
    const MAGIC: &'static [u8; 8] = b"UNBNDV20";
    fn decode(_: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let d = state(a, p, Self::MAGIC, UNBOND_LEN)?;
        Ok(Self {
            market: key(d, 8),
            provider: key(d, 40),
            amount: n64(d, 72),
            ready: i64le(d, 80),
            executed: flag(d, 88)?,
            slot: n32(d, 92),
            model: key(d, 96),
            rent_payer: key(d, 128),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ReceiptBucket {
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub model: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub provider: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub bond_vault: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub rent_payer: Pubkey,
    pub slot: u32,
    pub bond_bump: u8,
    pub finalized: bool,
    pub epoch: i64,
    pub epoch_end: i64,
    pub dispute_end: i64,
    pub count: u64,
    pub charge: u64,
    pub outstanding: u64,
    pub frontier: [[u8; 32]; 16],
}
impl Decode for ReceiptBucket {
    const MAGIC: &'static [u8; 8] = b"RECEIPT1";
    fn decode(_: &Pubkey, a: &RawAccount, p: &Pubkey) -> Result<Self> {
        let d = state(a, p, Self::MAGIC, RECEIPT_BUCKET_LEN)?;
        let mut frontier = [[0; 32]; 16];
        for (entry, bytes) in frontier.iter_mut().zip(d[224..].chunks_exact(32)) {
            entry.copy_from_slice(bytes);
        }
        let v = Self {
            market: key(d, 8),
            model: key(d, 40),
            provider: key(d, 72),
            bond_vault: key(d, 104),
            rent_payer: key(d, 136),
            slot: n32(d, 168),
            bond_bump: d[172],
            finalized: flag(d, 173)?,
            epoch: i64le(d, 176),
            epoch_end: i64le(d, 184),
            dispute_end: i64le(d, 192),
            count: n64(d, 200),
            charge: n64(d, 208),
            outstanding: n64(d, 216),
            frontier,
        };
        ensure(
            v.count < 65536 && v.outstanding <= v.count && v.epoch_end > v.epoch,
            "receipt bucket counters",
        )?;
        ensure(
            !v.finalized || (v.charge == 0 && v.outstanding == 0),
            "finalized receipt liabilities",
        )?;
        ensure(
            address::bond(p, &v.model, v.slot).1 == v.bond_bump,
            "receipt bond bump",
        )?;
        Ok(v)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TokenAccount {
    #[serde(with = "crate::serde_pubkey")]
    pub mint: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub owner: Pubkey,
    pub amount: u64,
    pub frozen: bool,
}
impl TokenAccount {
    pub fn decode(a: &RawAccount) -> Result<Self> {
        ensure(
            a.owner == address::TOKEN_PROGRAM && !a.executable && a.data.len() == 165,
            "SPL token account",
        )?;
        let d = &a.data;
        ensure(d[108] == 1 || d[108] == 2, "initialized token account")?;
        Ok(Self {
            mint: key(d, 0),
            owner: key(d, 32),
            amount: n64(d, 64),
            frozen: d[108] == 2,
        })
    }
    pub fn validate(&self, mint: &Pubkey, owner: &Pubkey) -> Result<()> {
        ensure(
            self.mint == *mint && self.owner == *owner && !self.frozen,
            "token mint, authority or frozen state",
        )
    }
}
