use crate::{
    Error, Hash, Instruction, Pubkey, Result, Signature,
    accounts::{CapacityBook, Decode, Market, Model, RawAccount, TermsBook},
    ensure,
    hosted::{HostedClient, RPC_PATH, http_client, response_bytes},
    instruction::{ModelKeys, Protocol},
    transaction::{PendingTransaction, PreparedTransaction},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json, value::RawValue};
use std::{
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Commitment {
    #[default]
    Processed,
    Confirmed,
    Finalized,
}
#[derive(Clone)]
enum Transport {
    Own {
        http: reqwest::Client,
        url: reqwest::Url,
    },
    Hosted(HostedClient),
}
type CachedBlockhash = tokio::sync::Mutex<Option<(tokio::time::Instant, LatestBlockhash)>>;
#[derive(Clone)]
pub struct RpcClient {
    transport: Transport,
    commitment: Commitment,
    ids: Arc<AtomicU64>,
    blockhash: Arc<[CachedBlockhash; 3]>,
}
impl std::fmt::Debug for RpcClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RpcClient")
            .field("commitment", &self.commitment)
            .finish_non_exhaustive()
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Context {
    pub slot: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContextValue<T> {
    pub context: Context,
    pub value: T,
}
#[derive(Clone, Debug, Deserialize)]
pub struct LatestBlockhash {
    pub blockhash: Hash,
    pub last_valid_block_height: u64,
    pub min_context_slot: u64,
}
#[derive(Clone, Copy, Debug)]
pub struct SendOptions {
    pub skip_preflight: bool,
    pub preflight_commitment: Commitment,
    pub max_retries: usize,
}
impl Default for SendOptions {
    fn default() -> Self {
        Self {
            skip_preflight: false,
            preflight_commitment: Commitment::default(),
            max_retries: 3,
        }
    }
}
#[derive(Deserialize)]
struct RpcError {
    code: i64,
    message: String,
    data: Option<Value>,
}
#[derive(Deserialize)]
struct Response<'a> {
    jsonrpc: &'a str,
    id: u64,
    #[serde(default, borrow, deserialize_with = "present_result")]
    result: Option<&'a RawValue>,
    error: Option<RpcError>,
}
fn present_result<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<&'de RawValue>, D::Error> {
    <&RawValue>::deserialize(d).map(Some)
}
#[derive(Deserialize)]
struct WireAccount {
    #[serde(with = "crate::serde_pubkey")]
    owner: Pubkey,
    lamports: u64,
    executable: bool,
    data: (String, String),
}
impl WireAccount {
    fn decode(self) -> Result<RawAccount> {
        ensure(self.data.1 == "base64", "RPC account encoding")?;
        Ok(RawAccount {
            owner: self.owner,
            lamports: self.lamports,
            executable: self.executable,
            data: STANDARD
                .decode(self.data.0)
                .map_err(|_| Error::Invalid("RPC account base64"))?,
        })
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Simulation {
    pub err: Option<Value>,
    #[serde(default)]
    pub logs: Option<Vec<String>>,
    pub units_consumed: Option<u64>,
    pub return_data: Option<Value>,
}
impl Simulation {
    pub fn check(&self) -> Result<()> {
        if let Some(err) = &self.err {
            Err(Error::Transaction(err.clone()))
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignatureStatus {
    pub slot: u64,
    pub confirmations: Option<u64>,
    pub err: Option<Value>,
    pub confirmation_status: Option<Commitment>,
}
impl SignatureStatus {
    pub fn reached(&self, target: Commitment) -> bool {
        match self.confirmation_status {
            Some(Commitment::Finalized) => true,
            Some(Commitment::Confirmed) => target != Commitment::Finalized,
            Some(Commitment::Processed) => target == Commitment::Processed,
            None => self.confirmations.is_none() || target == Commitment::Processed,
        }
    }
}
impl RpcClient {
    pub fn new(url: &str) -> Result<Self> {
        Self::with_http(url, http_client()?)
    }
    pub fn with_http(url: &str, http: reqwest::Client) -> Result<Self> {
        let url = reqwest::Url::parse(url).map_err(|_| Error::Invalid("RPC URL"))?;
        ensure(
            matches!(url.scheme(), "http" | "https") && url.fragment().is_none(),
            "RPC HTTP URL",
        )?;
        Ok(Self {
            transport: Transport::Own { http, url },
            commitment: Commitment::default(),
            ids: Arc::new(AtomicU64::new(1)),
            blockhash: Arc::new(std::array::from_fn(|_| tokio::sync::Mutex::new(None))),
        })
    }
    pub fn hosted(client: HostedClient) -> Self {
        Self {
            transport: Transport::Hosted(client),
            commitment: Commitment::default(),
            ids: Arc::new(AtomicU64::new(1)),
            blockhash: Arc::new(std::array::from_fn(|_| tokio::sync::Mutex::new(None))),
        }
    }
    pub fn with_commitment(mut self, commitment: Commitment) -> Self {
        self.commitment = commitment;
        self
    }
    pub fn commitment(&self) -> Commitment {
        self.commitment
    }
    pub async fn call<T: DeserializeOwned>(&self, method: &str, params: Value) -> Result<T> {
        let id = self.ids.fetch_add(1, Ordering::Relaxed);
        let body =
            serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        let bytes = match &self.transport {
            Transport::Own { http, url } => {
                response_bytes(
                    http.post(url.clone())
                        .header("content-type", "application/json")
                        .body(body)
                        .send()
                        .await?,
                    4 * 1024 * 1024,
                )
                .await?
            }
            Transport::Hosted(client) => {
                client
                    .post(RPC_PATH, "application/json", &body, 4 * 1024 * 1024)
                    .await?
            }
        };
        let response: Response<'_> = serde_json::from_slice(&bytes)?;
        ensure(
            response.jsonrpc == "2.0" && response.id == id,
            "JSON-RPC response identity",
        )?;
        if let Some(error) = response.error {
            return Err(Error::Rpc {
                code: error.code,
                message: error.message,
                data: error.data,
            });
        }
        Ok(serde_json::from_str(
            response
                .result
                .ok_or(Error::Invalid("JSON-RPC result"))?
                .get(),
        )?)
    }
    pub async fn raw_account(&self, address: Pubkey) -> Result<Option<RawAccount>> {
        Ok(self.raw_account_at(address, 0).await?.value)
    }
    pub async fn raw_account_at(
        &self,
        address: Pubkey,
        min_context_slot: u64,
    ) -> Result<ContextValue<Option<RawAccount>>> {
        let result: ContextValue<Option<WireAccount>> = self
            .call(
                "getAccountInfo",
                json!([address.to_string(), self.read_options(min_context_slot)]),
            )
            .await?;
        ensure(
            result.context.slot >= min_context_slot,
            "RPC account context slot",
        )?;
        Ok(ContextValue {
            context: result.context,
            value: result.value.map(WireAccount::decode).transpose()?,
        })
    }
    pub async fn account<T: Decode>(&self, address: Pubkey, program: Pubkey) -> Result<Option<T>> {
        self.raw_account(address)
            .await?
            .map(|a| T::decode(&address, &a, &program))
            .transpose()
    }
    pub async fn require_account<T: Decode>(&self, address: Pubkey, program: Pubkey) -> Result<T> {
        self.account(address, program)
            .await?
            .ok_or(Error::MissingAccount(address))
    }
    pub async fn multiple_accounts(
        &self,
        addresses: &[Pubkey],
    ) -> Result<ContextValue<Vec<Option<RawAccount>>>> {
        self.multiple_accounts_at(addresses, 0).await
    }
    pub async fn multiple_accounts_at(
        &self,
        addresses: &[Pubkey],
        min_context_slot: u64,
    ) -> Result<ContextValue<Vec<Option<RawAccount>>>> {
        ensure(
            !addresses.is_empty() && addresses.len() <= 100,
            "RPC account batch size",
        )?;
        let result: ContextValue<Vec<Option<WireAccount>>> = self
            .call(
                "getMultipleAccounts",
                json!([
                    addresses
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>(),
                    self.read_options(min_context_slot)
                ]),
            )
            .await?;
        ensure(
            result.context.slot >= min_context_slot,
            "RPC account context slot",
        )?;
        ensure(
            result.value.len() == addresses.len(),
            "RPC account batch response",
        )?;
        Ok(ContextValue {
            context: result.context,
            value: result
                .value
                .into_iter()
                .map(|a| a.map(WireAccount::decode).transpose())
                .collect::<Result<_>>()?,
        })
    }
    fn read_options(&self, min_context_slot: u64) -> Value {
        let mut options = json!({"encoding":"base64","commitment":self.commitment});
        if min_context_slot > 0 {
            options["minContextSlot"] = json!(min_context_slot);
        }
        options
    }
    pub async fn program_snapshot(
        &self,
        program: Pubkey,
        filters: Vec<Value>,
    ) -> Result<ContextValue<Vec<(Pubkey, RawAccount)>>> {
        self.program_snapshot_at(program, filters, 0).await
    }
    pub async fn program_snapshot_at(
        &self,
        program: Pubkey,
        filters: Vec<Value>,
        min_context_slot: u64,
    ) -> Result<ContextValue<Vec<(Pubkey, RawAccount)>>> {
        #[derive(Deserialize)]
        struct Entry {
            #[serde(with = "crate::serde_pubkey")]
            pubkey: Pubkey,
            account: WireAccount,
        }
        let mut options = self.read_options(min_context_slot);
        options["withContext"] = json!(true);
        options["filters"] = json!(filters);
        let result: ContextValue<Vec<Entry>> = self
            .call("getProgramAccounts", json!([program.to_string(), options]))
            .await?;
        ensure(
            result.context.slot >= min_context_slot,
            "RPC snapshot context slot",
        )?;
        Ok(ContextValue {
            context: result.context,
            value: result
                .value
                .into_iter()
                .map(|e| Ok((e.pubkey, e.account.decode()?)))
                .collect::<Result<_>>()?,
        })
    }
    pub async fn program_accounts<T: Decode>(
        &self,
        program: Pubkey,
        mut filters: Vec<Value>,
    ) -> Result<Vec<(Pubkey, T)>> {
        filters.push(json!({"memcmp":{"offset":0,"bytes":bs58::encode(T::MAGIC).into_string()}}));
        #[derive(Deserialize)]
        struct Entry {
            #[serde(with = "crate::serde_pubkey")]
            pubkey: Pubkey,
            account: WireAccount,
        }
        let values:Vec<Entry>=self.call("getProgramAccounts",json!([program.to_string(),{"encoding":"base64","commitment":self.commitment,"filters":filters}])).await?;
        values
            .into_iter()
            .map(|e| {
                Ok((
                    e.pubkey,
                    T::decode(&e.pubkey, &e.account.decode()?, &program)?,
                ))
            })
            .collect()
    }
    pub async fn validate_market(&self, protocol: &Protocol) -> Result<Market> {
        let market: Market = self
            .require_account(protocol.market, protocol.program)
            .await?;
        ensure(market.mint == protocol.mint, "market payment mint")?;
        Ok(market)
    }
    pub async fn model_books(
        &self,
        protocol: &Protocol,
        model: Pubkey,
    ) -> Result<(Model, TermsBook, CapacityBook)> {
        let state: Model = self.require_account(model, protocol.program).await?;
        ensure(state.market == protocol.market, "model market")?;
        let result = self
            .multiple_accounts(&[state.terms, state.capacity])
            .await?;
        let terms = TermsBook::decode(
            &state.terms,
            result.value[0]
                .as_ref()
                .ok_or(Error::MissingAccount(state.terms))?,
            &protocol.program,
        )?;
        let capacity = CapacityBook::decode(
            &state.capacity,
            result.value[1]
                .as_ref()
                .ok_or(Error::MissingAccount(state.capacity))?,
            &protocol.program,
        )?;
        terms.validate_pair(&capacity, &model, &state)?;
        Ok((state, terms, capacity))
    }
    pub async fn balance(&self, wallet: Pubkey) -> Result<u64> {
        Ok(self
            .call::<ContextValue<u64>>(
                "getBalance",
                json!([wallet.to_string(),{"commitment":self.commitment}]),
            )
            .await?
            .value)
    }
    pub async fn token_balance(&self, account: Pubkey) -> Result<u64> {
        let value: ContextValue<Value> = self
            .call(
                "getTokenAccountBalance",
                json!([account.to_string(),{"commitment":self.commitment}]),
            )
            .await?;
        crate::amount::parse_atomic(
            value.value["amount"]
                .as_str()
                .ok_or(Error::Invalid("token balance amount"))?,
        )
    }
    pub async fn rent(&self, space: usize) -> Result<u64> {
        self.call(
            "getMinimumBalanceForRentExemption",
            json!([space,{"commitment":self.commitment}]),
        )
        .await
    }
    pub async fn block_height(&self) -> Result<u64> {
        self.call("getBlockHeight", json!([{"commitment":self.commitment}]))
            .await
    }
    pub async fn prepare(
        &self,
        instructions: &[Instruction],
        payer: Pubkey,
    ) -> Result<PreparedTransaction> {
        let latest = self.latest_blockhash().await?;
        PreparedTransaction::new(
            instructions,
            payer,
            latest.blockhash,
            latest.last_valid_block_height,
            latest.min_context_slot,
        )
    }
    pub async fn latest_blockhash(&self) -> Result<LatestBlockhash> {
        self.latest_blockhash_at(0).await
    }
    async fn latest_blockhash_at(&self, min_context_slot: u64) -> Result<LatestBlockhash> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Blockhash {
            blockhash: String,
            last_valid_block_height: u64,
        }
        let result: ContextValue<Blockhash> = self
            .call(
                "getLatestBlockhash",
                json!([{"commitment":self.commitment,"minContextSlot":min_context_slot}]),
            )
            .await?;
        Ok(LatestBlockhash {
            blockhash: Hash::from_str(&result.value.blockhash)
                .map_err(|_| Error::Invalid("RPC blockhash"))?,
            last_valid_block_height: result.value.last_valid_block_height,
            min_context_slot: result.context.slot,
        })
    }
    // opt-in short reuse; clones share refreshes, with separate caches for each commitment.
    pub async fn prepare_cached(
        &self,
        instructions: &[Instruction],
        payer: Pubkey,
        min_context_slot: u64,
    ) -> Result<PreparedTransaction> {
        let mut cached = self.blockhash[self.commitment as usize].lock().await;
        let fresh = cached.as_ref().is_some_and(|(at, latest)| {
            at.elapsed() < Duration::from_secs(5) && latest.min_context_slot >= min_context_slot
        });
        if !fresh {
            let started = tokio::time::Instant::now();
            let latest = self.latest_blockhash_at(min_context_slot).await?;
            ensure(
                latest.min_context_slot >= min_context_slot,
                "RPC blockhash context slot",
            )?;
            *cached = Some((started, latest));
        }
        let latest = cached.as_ref().unwrap().1.clone();
        drop(cached);
        PreparedTransaction::new(
            instructions,
            payer,
            latest.blockhash,
            latest.last_valid_block_height,
            latest.min_context_slot,
        )
    }
    pub async fn invalidate_blockhash(&self) {
        *self.blockhash[self.commitment as usize].lock().await = None;
    }
    pub async fn simulate(&self, pending: &PendingTransaction) -> Result<Simulation> {
        pending.validate()?;
        let response:ContextValue<Simulation>=self.call("simulateTransaction",json!([pending.wire_base64,{"encoding":"base64","commitment":self.commitment,"sigVerify":true,"minContextSlot":pending.min_context_slot}])).await?;
        Ok(response.value)
    }
    pub async fn signature_status(&self, signature: &Signature) -> Result<Option<SignatureStatus>> {
        Ok(self
            .signature_statuses(std::slice::from_ref(signature), true)
            .await?
            .remove(0))
    }
    pub async fn signature_statuses(
        &self,
        signatures: &[Signature],
        search_history: bool,
    ) -> Result<Vec<Option<SignatureStatus>>> {
        ensure(
            !signatures.is_empty() && signatures.len() <= 256,
            "signature batch size",
        )?;
        let response: ContextValue<Vec<Option<Value>>> = self
            .call(
                "getSignatureStatuses",
                json!([signatures.iter().map(ToString::to_string).collect::<Vec<_>>(),{"searchTransactionHistory":search_history}]),
            )
            .await?;
        ensure(
            response.value.len() == signatures.len(),
            "signature status count",
        )?;
        response
            .value
            .into_iter()
            .map(|status| {
                status
                    .map(|status| {
                        let error_valid = match status.get("err") {
                            Some(Value::Null) => true,
                            Some(Value::String(error)) => !error.is_empty(),
                            Some(Value::Object(error)) => !error.is_empty(),
                            _ => false,
                        };
                        ensure(
                            error_valid
                                && (status
                                    .get("confirmationStatus")
                                    .is_some_and(|label| !label.is_null())
                                    || status.get("confirmations").is_some()),
                            "signature status evidence",
                        )?;
                        Ok(serde_json::from_value(status)?)
                    })
                    .transpose()
            })
            .collect()
    }
    pub async fn submit(&self, pending: &PendingTransaction) -> Result<Signature> {
        let signature = pending.validate()?;
        if let Some(status) = self.pending_status(pending, &signature).await? {
            if let Some(error) = &status.err
                && status.reached(Commitment::Confirmed)
            {
                return Err(Error::Transaction(error.clone()));
            }
            return Ok(signature);
        }
        self.broadcast(pending).await
    }
    pub async fn broadcast(&self, pending: &PendingTransaction) -> Result<Signature> {
        self.broadcast_with_options(
            pending,
            SendOptions {
                preflight_commitment: self.commitment,
                ..SendOptions::default()
            },
        )
        .await
    }
    pub async fn broadcast_with_options(
        &self,
        pending: &PendingTransaction,
        options: SendOptions,
    ) -> Result<Signature> {
        let signature = pending.validate()?;
        let result:String=self.call("sendTransaction",json!([pending.wire_base64,{"encoding":"base64","skipPreflight":options.skip_preflight,"preflightCommitment":options.preflight_commitment,"maxRetries":options.max_retries,"minContextSlot":pending.min_context_slot}])).await?;
        ensure(
            result == signature.to_string(),
            "submitted transaction signature",
        )?;
        Ok(signature)
    }
    pub async fn confirm(
        &self,
        pending: &PendingTransaction,
        timeout: Duration,
    ) -> Result<SignatureStatus> {
        let signature = pending.validate()?;
        let deadline = tokio::time::Instant::now() + timeout;
        let mut next_height_check = tokio::time::Instant::now();
        loop {
            let poll = async {
                if tokio::time::Instant::now() >= next_height_check {
                    next_height_check = tokio::time::Instant::now() + Duration::from_secs(5);
                    self.pending_status(pending, &signature).await
                } else {
                    Ok(self
                        .signature_statuses(std::slice::from_ref(&signature), false)
                        .await?
                        .remove(0))
                }
            };
            // zero requests one reconciliation pass; positive timeouts bound all awaited rpc work.
            let status = if timeout.is_zero() {
                poll.await?
            } else {
                tokio::time::timeout_at(deadline, poll)
                    .await
                    .map_err(|_| Error::Timeout)??
            };
            if let Some(status) = status {
                if let Some(err) = &status.err
                    && status.reached(Commitment::Confirmed)
                {
                    return Err(Error::Transaction(err.clone()));
                }
                if status.err.is_none() && status.reached(self.commitment) {
                    return Ok(status);
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            tokio::time::sleep(
                Duration::from_millis(250)
                    .min(deadline.saturating_duration_since(tokio::time::Instant::now())),
            )
            .await;
        }
    }
    async fn pending_status(
        &self,
        pending: &PendingTransaction,
        signature: &Signature,
    ) -> Result<Option<SignatureStatus>> {
        let status = self.signature_status(signature).await?;
        if status.is_none()
            && self
                .call::<u64>("getBlockHeight", json!([{"commitment":"finalized"}]))
                .await?
                > pending.last_valid_block_height
        {
            self.invalidate_blockhash().await;
            return self
                .signature_status(signature)
                .await?
                .map(Some)
                .ok_or(Error::Expired);
        }
        Ok(status)
    }
    pub async fn transaction(&self, signature: &Signature) -> Result<Option<Value>> {
        let commitment = if self.commitment == Commitment::Processed {
            Commitment::Confirmed
        } else {
            self.commitment
        };
        self.call("getTransaction",json!([signature.to_string(),{"encoding":"json","commitment":commitment,"maxSupportedTransactionVersion":0}])).await
    }
    pub async fn create_model_books(
        &self,
        protocol: &Protocol,
        payer: Pubkey,
        model: ModelKeys,
        limit: u32,
    ) -> Result<Vec<Instruction>> {
        let [terms, capacity] = crate::instruction::book_sizes(limit)?;
        let (terms_rent, capacity_rent) = tokio::join!(self.rent(terms), self.rent(capacity));
        Ok(vec![
            protocol.create_account(payer, model.terms, terms_rent?, terms)?,
            protocol.create_account(payer, model.capacity, capacity_rent?, capacity)?,
        ])
    }
}
