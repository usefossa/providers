use crate::{
    AccountMeta, Instruction, Pubkey, Result,
    accounts::{self, Booking, CapacityBook, DocumentRef, Model, OfferTerms, TermsBook, User},
    address::{self, ASSOCIATED_TOKEN_PROGRAM, SYSTEM_PROGRAM, TOKEN_PROGRAM},
    amount::{Rates, Usage, collateral},
    ensure,
    receipt::{Receipt, ReceiptProof},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Protocol {
    #[serde(with = "crate::serde_pubkey")]
    pub program: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub market: Pubkey,
    #[serde(with = "crate::serde_pubkey")]
    pub mint: Pubkey,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModelKeys {
    pub model: Pubkey,
    pub terms: Pubkey,
    pub capacity: Pubkey,
}
impl ModelKeys {
    pub fn from_account(model: Pubkey, state: &Model) -> Self {
        Self {
            model,
            terms: state.terms,
            capacity: state.capacity,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserKeys {
    pub user: Pubkey,
    pub owner: Pubkey,
    pub vault: Pubkey,
}
impl UserKeys {
    pub fn from_account(user: Pubkey, state: &User) -> Self {
        Self {
            user,
            owner: state.owner,
            vault: state.vault,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OfferKeys {
    pub model: ModelKeys,
    pub provider: Pubkey,
    pub payout: Pubkey,
    pub bond_vault: Pubkey,
    pub slot: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettlementKeys {
    pub offer: OfferKeys,
    pub user: UserKeys,
    pub booking: Pubkey,
}
impl SettlementKeys {
    pub fn from_booking(
        booking: Pubkey,
        state: &Booking,
        model: ModelKeys,
        user: UserKeys,
    ) -> Result<Self> {
        ensure(
            state.model == model.model && state.user == user.user && state.owner == user.owner,
            "settlement account bindings",
        )?;
        Ok(Self {
            offer: OfferKeys {
                model,
                provider: state.provider,
                payout: state.payout,
                bond_vault: state.bond_vault,
                slot: state.slot,
            },
            user,
            booking,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BookingMode {
    OneShot,
    Reusable { generation: u64, fresh: bool },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookRequest {
    pub model_id: [u8; 32],
    pub rate_caps: Rates,
    pub min_speed: u16,
    pub max_input: u32,
    pub max_output: u32,
    pub private: bool,
    pub max_spend: u64,
    pub estimated_input: u32,
    pub estimated_cached: u32,
    pub estimated_output: u32,
    #[serde(with = "crate::serde_pubkey")]
    pub cache_provider: Pubkey,
}
impl BookRequest {
    pub fn validate(&self) -> Result<()> {
        ensure(
            self.max_input > 0
                && self.max_output > 0
                && self.estimated_input <= self.max_input
                && self.estimated_output <= self.max_output
                && self.estimated_cached <= self.estimated_input
                && (self.estimated_cached == 0 || self.cache_provider != Pubkey::default()),
            "booking request",
        )
    }
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut p = Vec::with_capacity(115);
        p.extend(self.model_id);
        self.rate_caps.encode(&mut p);
        p.extend(self.min_speed.to_le_bytes());
        p.extend(self.max_input.to_le_bytes());
        p.extend(self.max_output.to_le_bytes());
        p.push(u8::from(self.private));
        p.extend(self.max_spend.to_le_bytes());
        for n in [
            self.estimated_input,
            self.estimated_cached,
            self.estimated_output,
        ] {
            p.extend(n.to_le_bytes());
        }
        p.extend(self.cache_provider.as_ref());
        Ok(p)
    }
    pub fn select(
        &self,
        terms: &TermsBook,
        capacity: &CapacityBook,
        free: u64,
    ) -> Result<Option<Selection>> {
        self.validate()?;
        ensure(
            terms.model == capacity.model
                && terms.limit == capacity.limit
                && terms.entries.len() == capacity.entries.len(),
            "offer book pair",
        )?;
        let min_speed = u32::from(self.min_speed).max(self.max_output.div_ceil(86_400 - 300));
        let mut best: Option<Selection> = None;
        for (slot, (offer, cap)) in terms.entries.iter().zip(&capacity.entries).enumerate() {
            let t = offer.terms;
            let r = t.rates;
            if !t.active
                || !offer.approved
                || (self.private && !t.private)
                || u32::from(t.speed_tps) < min_speed
                || cap.active >= t.capacity
                || cap.obligations == u32::MAX
                || r.input > self.rate_caps.input
                || r.cached > self.rate_caps.cached
                || r.output > self.rate_caps.output
            {
                continue;
            }
            let cached = if offer.provider == self.cache_provider {
                self.estimated_cached
            } else {
                0
            };
            let score = r.charge(Usage {
                uncached: self.estimated_input - cached,
                cached,
                output: self.estimated_output,
            });
            if best.as_ref().is_some_and(|best| score >= best.score) {
                continue;
            }
            let reserve = r.reserve(self.max_input, self.max_output);
            if reserve > free.min(self.max_spend) {
                continue;
            }
            let Some(exposure) = cap.exposure.checked_add(reserve) else {
                continue;
            };
            if !collateral(exposure).is_ok_and(|needed| cap.funded_bond >= needed) {
                continue;
            }
            best = Some(Selection {
                slot: slot as u32,
                reserve,
                score,
                exposure,
                cache_allowed: offer.provider == self.cache_provider,
            });
            if score == 0 {
                break;
            }
        }
        Ok(best)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub slot: u32,
    pub reserve: u64,
    pub score: u64,
    pub exposure: u64,
    pub cache_allowed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimReceipt {
    pub bucket: Pubkey,
    pub epoch: i64,
    pub index: u64,
    pub receipt: Receipt,
}

fn r(key: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(key, false)
}
fn w(key: Pubkey) -> AccountMeta {
    AccountMeta::new(key, false)
}
fn s(key: Pubkey) -> AccountMeta {
    AccountMeta::new_readonly(key, true)
}
fn ws(key: Pubkey) -> AccountMeta {
    AccountMeta::new(key, true)
}
fn slot_amount(slot: u32, amount: u64) -> Result<Vec<u8>> {
    ensure(amount > 0, "positive amount")?;
    Ok([
        slot.to_le_bytes().as_slice(),
        amount.to_le_bytes().as_slice(),
    ]
    .concat())
}
fn generation_payload(generation: Option<u64>) -> Result<Vec<u8>> {
    if let Some(n) = generation {
        ensure(n > 0 && n < (1u64 << 56), "booking generation")?;
        Ok(n.to_le_bytes().to_vec())
    } else {
        Ok(vec![])
    }
}

impl Protocol {
    fn ix(&self, op: u8, p: impl AsRef<[u8]>, accounts: Vec<AccountMeta>) -> Instruction {
        let p = p.as_ref();
        let mut data = Vec::with_capacity(p.len() + 1);
        data.push(op);
        data.extend(p);
        Instruction {
            program_id: self.program,
            accounts,
            data,
        }
    }
    pub fn initialize_market(&self, admin: Pubkey) -> Instruction {
        self.ix(0, [], vec![ws(self.market), s(admin), r(self.mint)])
    }
    pub fn initialize_user(&self, user: UserKeys) -> Instruction {
        self.ix(
            1,
            [],
            vec![ws(user.user), s(user.owner), r(self.market), r(user.vault)],
        )
    }
    pub fn register_offer(
        &self,
        k: OfferKeys,
        terms: OfferTerms,
        admin: Option<Pubkey>,
    ) -> Result<Instruction> {
        let mut data = k.slot.to_le_bytes().to_vec();
        data.extend(terms.encode()?);
        let mut keys = vec![
            r(self.market),
            r(k.model.model),
            w(k.model.terms),
            w(k.model.capacity),
            r(address::provider(&self.program, &self.market, &k.provider).0),
            s(k.provider),
            r(k.payout),
            r(k.bond_vault),
        ];
        if let Some(admin) = admin {
            keys.push(s(admin));
        }
        Ok(self.ix(2, data, keys))
    }
    pub fn deposit(&self, u: UserKeys, source: Pubkey, amount: u64) -> Result<Instruction> {
        ensure(amount > 0, "positive deposit")?;
        Ok(self.ix(
            3,
            amount.to_le_bytes(),
            vec![
                r(self.market),
                w(u.user),
                s(u.owner),
                w(source),
                w(u.vault),
                r(self.mint),
                r(TOKEN_PROGRAM),
            ],
        ))
    }
    pub fn fund_bond(&self, k: OfferKeys, source: Pubkey, amount: u64) -> Result<Instruction> {
        Ok(self.ix(
            4,
            slot_amount(k.slot, amount)?,
            vec![
                r(self.market),
                r(k.model.model),
                r(k.model.terms),
                w(k.model.capacity),
                s(k.provider),
                w(source),
                w(k.bond_vault),
                r(self.mint),
                r(TOKEN_PROGRAM),
            ],
        ))
    }
    pub fn update_offer(&self, k: OfferKeys, terms: OfferTerms) -> Result<Instruction> {
        let mut p = k.slot.to_le_bytes().to_vec();
        p.extend(terms.encode()?);
        Ok(self.ix(
            5,
            p,
            vec![
                r(self.market),
                r(k.model.model),
                w(k.model.terms),
                s(k.provider),
            ],
        ))
    }
    pub fn book(
        &self,
        m: ModelKeys,
        u: UserKeys,
        booking: Pubkey,
        rent_payer: Pubkey,
        request: &BookRequest,
        mode: BookingMode,
    ) -> Result<Instruction> {
        ensure(
            address::model(&self.program, &self.market, &request.model_id).0 == m.model,
            "booking model id",
        )?;
        let mut p = request.encode()?;
        let (booking_meta, payer_meta) = match mode {
            BookingMode::OneShot => (ws(booking), s(rent_payer)),
            BookingMode::Reusable { generation, fresh } => {
                p.extend(generation_payload(Some(generation))?);
                (AccountMeta::new(booking, fresh), r(rent_payer))
            }
        };
        Ok(self.ix(
            6,
            p,
            vec![
                r(self.market),
                r(m.model),
                r(m.terms),
                w(m.capacity),
                w(u.user),
                s(u.owner),
                booking_meta,
                payer_meta,
            ],
        ))
    }
    pub fn claim(
        &self,
        k: SettlementKeys,
        usage: Usage,
        receipt: Option<&ClaimReceipt>,
    ) -> Result<Instruction> {
        let o = k.offer;
        let u = k.user;
        let mut p = Vec::with_capacity(156);
        usage.encode(&mut p);
        let mut keys = vec![
            r(self.market),
            r(o.model.model),
            w(o.model.capacity),
            w(u.user),
            w(k.booking),
            s(o.provider),
            w(u.vault),
            w(o.payout),
            r(self.mint),
            r(address::vault(&self.program, &u.user).0),
            r(TOKEN_PROGRAM),
        ];
        if let Some(c) = receipt {
            ensure(
                c.receipt.owner == u.owner
                    && c.receipt.user == u.user
                    && c.receipt.booking == k.booking,
                "claim receipt binding",
            )?;
            p.extend(generation_payload(Some(c.receipt.generation))?);
            p.extend(c.epoch.to_le_bytes());
            p.extend(c.index.to_le_bytes());
            p.extend(c.receipt.encode());
            keys.push(w(c.bucket));
        }
        Ok(self.ix(7, p, keys))
    }
    pub fn expire(&self, k: SettlementKeys, generation: Option<u64>) -> Result<Instruction> {
        Ok(self.ix(8, generation_payload(generation)?, self.settlement_keys(k)))
    }
    pub fn finalize(&self, m: ModelKeys, booking: Pubkey) -> Instruction {
        self.ix(
            9,
            [],
            vec![r(self.market), r(m.model), w(m.capacity), w(booking)],
        )
    }
    pub fn withdraw(&self, u: UserKeys, destination: Pubkey, amount: u64) -> Result<Instruction> {
        ensure(amount > 0, "positive withdrawal")?;
        Ok(self.ix(
            10,
            amount.to_le_bytes(),
            vec![
                r(self.market),
                w(u.user),
                s(u.owner),
                w(u.vault),
                w(destination),
                r(self.mint),
                r(address::vault(&self.program, &u.user).0),
                r(TOKEN_PROGRAM),
            ],
        ))
    }
    pub fn dispute(&self, m: ModelKeys, booking: Pubkey, owner: Pubkey) -> Instruction {
        self.ix(
            11,
            [],
            vec![r(self.market), r(m.model), w(booking), s(owner)],
        )
    }
    pub fn resolve(&self, k: SettlementKeys, admin: Pubkey, refund: u64) -> Instruction {
        let o = k.offer;
        self.ix(
            12,
            refund.to_le_bytes(),
            vec![
                r(self.market),
                r(o.model.model),
                w(o.model.capacity),
                w(k.user.user),
                w(k.booking),
                s(admin),
                w(o.bond_vault),
                w(k.user.vault),
                r(self.mint),
                r(address::bond(&self.program, &o.model.model, o.slot).0),
                r(TOKEN_PROGRAM),
            ],
        )
    }
    pub fn request_unbond(
        &self,
        k: OfferKeys,
        request: Pubkey,
        rent_payer: Pubkey,
        amount: u64,
    ) -> Result<Instruction> {
        Ok(self.ix(
            13,
            slot_amount(k.slot, amount)?,
            vec![
                r(self.market),
                r(k.model.model),
                r(k.model.terms),
                w(k.model.capacity),
                s(k.provider),
                ws(request),
                s(rent_payer),
            ],
        ))
    }
    pub fn execute_unbond(&self, k: OfferKeys, request: Pubkey) -> Instruction {
        self.ix(
            14,
            [],
            vec![
                r(self.market),
                r(k.model.model),
                r(k.model.terms),
                w(k.model.capacity),
                w(request),
                s(k.provider),
                w(k.bond_vault),
                w(k.payout),
                r(self.mint),
                r(address::bond(&self.program, &k.model.model, k.slot).0),
                r(TOKEN_PROGRAM),
            ],
        )
    }
    pub fn release(&self, k: SettlementKeys, generation: Option<u64>) -> Result<Instruction> {
        let mut keys = self.settlement_keys(k);
        keys.push(s(k.offer.provider));
        Ok(self.ix(15, generation_payload(generation)?, keys))
    }
    pub fn approve_offer(
        &self,
        m: ModelKeys,
        slot: u32,
        admin: Pubkey,
        approved: bool,
    ) -> Instruction {
        let mut p = slot.to_le_bytes().to_vec();
        p.push(u8::from(approved));
        self.ix(
            16,
            p,
            vec![r(self.market), r(m.model), w(m.terms), s(admin)],
        )
    }
    pub fn close(&self, account: Pubkey, rent_payer: Pubkey, reusable: bool) -> Instruction {
        self.ix(
            17,
            [],
            vec![w(account), AccountMeta::new(rent_payer, reusable)],
        )
    }
    pub fn initialize_model(&self, k: InitializeModel<'_>) -> Result<Instruction> {
        book_sizes(k.limit)?;
        ensure(
            k.model.terms != k.model.capacity
                && address::model(&self.program, &self.market, &k.id).0 == k.model.model,
            "new model bindings",
        )?;
        let mut p = k.id.to_vec();
        p.extend(k.limit.to_le_bytes());
        p.extend(k.document.encode()?);
        let mut keys = vec![
            r(self.market),
            w(k.model.model),
            ws(k.model.terms),
            ws(k.model.capacity),
            s(k.provider),
            ws(k.payer),
            r(SYSTEM_PROGRAM),
            r(address::provider(&self.program, &self.market, &k.provider).0),
            w(address::model_metadata(&self.program, &k.model.model).0),
        ];
        if let Some(admin) = k.admin {
            keys.push(s(admin));
        }
        Ok(self.ix(18, p, keys))
    }
    pub fn initialize_provider(
        &self,
        provider: Pubkey,
        payer: Pubkey,
        document: &DocumentRef,
    ) -> Result<Instruction> {
        Ok(self.ix(
            19,
            document.encode()?,
            vec![
                r(self.market),
                w(address::provider(&self.program, &self.market, &provider).0),
                s(provider),
                ws(payer),
                r(SYSTEM_PROGRAM),
            ],
        ))
    }
    pub fn approve_provider(&self, provider: Pubkey, admin: Pubkey, approved: bool) -> Instruction {
        self.ix(
            20,
            [u8::from(approved)],
            vec![
                r(self.market),
                w(address::provider(&self.program, &self.market, &provider).0),
                s(admin),
            ],
        )
    }
    pub fn update_provider_metadata(
        &self,
        provider: Pubkey,
        document: &DocumentRef,
    ) -> Result<Instruction> {
        Ok(self.ix(
            21,
            document.encode()?,
            vec![
                r(self.market),
                w(address::provider(&self.program, &self.market, &provider).0),
                s(provider),
            ],
        ))
    }
    pub fn update_model_metadata(
        &self,
        model: Pubkey,
        creator: Pubkey,
        document: &DocumentRef,
    ) -> Result<Instruction> {
        Ok(self.ix(
            22,
            document.encode()?,
            vec![
                r(self.market),
                r(model),
                w(address::model_metadata(&self.program, &model).0),
                s(creator),
            ],
        ))
    }
    pub fn initialize_receipt_bucket(
        &self,
        m: ModelKeys,
        params: InitializeReceiptBucket,
    ) -> Result<Instruction> {
        ensure(
            (1..=1800).contains(&params.duration),
            "receipt bucket duration",
        )?;
        let mut p = params.slot.to_le_bytes().to_vec();
        p.extend(params.duration.to_le_bytes());
        if let Some(epoch) = params.epoch {
            p.extend(epoch.to_le_bytes());
        }
        Ok(self.ix(
            23,
            p,
            vec![
                r(self.market),
                r(m.model),
                r(m.terms),
                AccountMeta::new(params.bucket, params.fresh),
                s(params.payer),
            ],
        ))
    }
    pub fn finalize_receipt_bucket(&self, m: ModelKeys, bucket: Pubkey) -> Instruction {
        self.ix(
            24,
            [],
            vec![r(self.market), r(m.model), w(m.capacity), w(bucket)],
        )
    }
    pub fn dispute_receipt(
        &self,
        model: Pubkey,
        bucket: Pubkey,
        payer: Pubkey,
        proof: &ReceiptProof,
    ) -> Result<Instruction> {
        let p = proof.encode()?;
        let case = address::receipt_case(&self.program, &bucket, proof.epoch, proof.index).0;
        Ok(self.ix(
            25,
            p,
            vec![
                r(self.market),
                r(model),
                w(bucket),
                w(case),
                s(proof.receipt.owner),
                ws(payer),
                r(SYSTEM_PROGRAM),
            ],
        ))
    }
    pub fn initialize_reusable_booking(
        &self,
        u: UserKeys,
        booking: Pubkey,
        payer: Pubkey,
    ) -> Instruction {
        self.ix(
            26,
            [],
            vec![r(self.market), r(u.user), s(u.owner), ws(booking), s(payer)],
        )
    }
    pub fn close_receipt_bucket(
        &self,
        bucket: Pubkey,
        rent_payer: Pubkey,
        authority: Pubkey,
    ) -> Instruction {
        self.ix(27, [], vec![w(bucket), w(rent_payer), s(authority)])
    }
    fn settlement_keys(&self, k: SettlementKeys) -> Vec<AccountMeta> {
        vec![
            r(self.market),
            r(k.offer.model.model),
            w(k.offer.model.capacity),
            w(k.user.user),
            w(k.booking),
        ]
    }
    pub fn create_account(
        &self,
        payer: Pubkey,
        account: Pubkey,
        lamports: u64,
        space: usize,
    ) -> Result<Instruction> {
        create_account(payer, account, self.program, lamports, space)
    }
}

pub struct InitializeModel<'a> {
    pub model: ModelKeys,
    pub provider: Pubkey,
    pub payer: Pubkey,
    pub admin: Option<Pubkey>,
    pub id: [u8; 32],
    pub limit: u32,
    pub document: &'a DocumentRef,
}
#[derive(Clone, Copy, Debug)]
pub struct InitializeReceiptBucket {
    pub bucket: Pubkey,
    pub payer: Pubkey,
    pub slot: u32,
    pub duration: u32,
    pub epoch: Option<i64>,
    pub fresh: bool,
}
pub fn book_sizes(limit: u32) -> Result<[usize; 2]> {
    ensure(
        (1..=accounts::MAX_ENTRIES).contains(&limit),
        "model book limit",
    )?;
    Ok([
        accounts::HEADER_LEN + limit as usize * accounts::TERMS_LEN,
        accounts::HEADER_LEN + limit as usize * accounts::CAPACITY_LEN,
    ])
}
pub fn create_account(
    payer: Pubkey,
    account: Pubkey,
    owner: Pubkey,
    lamports: u64,
    space: usize,
) -> Result<Instruction> {
    let size = u64::try_from(space).map_err(|_| crate::Error::Arithmetic)?;
    let mut data = 0u32.to_le_bytes().to_vec();
    data.extend(lamports.to_le_bytes());
    data.extend(size.to_le_bytes());
    data.extend(owner.as_ref());
    Ok(Instruction {
        program_id: SYSTEM_PROGRAM,
        accounts: vec![ws(payer), ws(account)],
        data,
    })
}
pub fn create_associated_token(payer: Pubkey, owner: Pubkey, mint: Pubkey) -> Instruction {
    Instruction {
        program_id: ASSOCIATED_TOKEN_PROGRAM,
        accounts: vec![
            ws(payer),
            w(address::associated_token(&mint, &owner)),
            r(owner),
            r(mint),
            r(SYSTEM_PROGRAM),
            r(TOKEN_PROGRAM),
        ],
        data: vec![1],
    }
}
