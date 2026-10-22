pub mod accounts;
pub mod address;
pub mod amount;
pub mod hosted;
pub mod inference;
pub mod instruction;
pub mod metadata;
pub mod onboarding;
pub mod receipt;
pub mod rpc;
pub mod transaction;

pub use solana_address::Address as Pubkey;
pub use solana_hash::Hash;
pub use solana_instruction::{AccountMeta, Instruction};
pub use solana_keypair::Keypair;
pub use solana_signature::Signature;
pub use solana_signer::Signer;
pub use solana_transaction::Transaction;
pub use {instruction::Protocol, rpc::RpcClient, transaction::PendingTransaction};

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) mod serde_pubkey {
    use crate::Pubkey;
    use serde::{Deserialize, Deserializer, Serializer, de::Error};
    pub fn serialize<S: Serializer>(key: &Pubkey, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&key.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Pubkey, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid {0}")]
    Invalid(&'static str),
    #[error("arithmetic overflow or insufficient balance")]
    Arithmetic,
    #[error("metadata: {0}")]
    Metadata(String),
    #[error("RPC {code}: {message}")]
    Rpc {
        code: i64,
        message: String,
        data: Option<serde_json::Value>,
    },
    #[error("HTTP request failed: {0}")]
    Http(reqwest::Error),
    #[error("HTTP status {0}")]
    HttpStatus(u16),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("signing: {0}")]
    Signing(String),
    #[error("transaction failed: {0}")]
    Transaction(serde_json::Value),
    #[error("transaction expired; inspect its signature before preparing a replacement")]
    Expired,
    #[error("confirmation timed out; transaction remains pending")]
    Timeout,
    #[error("account missing: {0}")]
    MissingAccount(Pubkey),
}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        Self::Http(error.without_url())
    }
}

pub(crate) fn ensure(value: bool, reason: &'static str) -> Result<()> {
    if value {
        Ok(())
    } else {
        Err(Error::Invalid(reason))
    }
}
