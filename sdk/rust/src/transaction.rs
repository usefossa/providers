use crate::{
    Error, Hash, Instruction, Keypair, Pubkey, Result, Signature, Signer, Transaction, ensure,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use solana_message::Message;
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
    str::FromStr,
};
use zeroize::Zeroizing;

pub const MAX_TRANSACTION_BYTES: usize = 1232;
pub struct PreparedTransaction {
    pub transaction: Transaction,
    pub last_valid_block_height: u64,
    pub min_context_slot: u64,
}
impl PreparedTransaction {
    pub fn new(
        instructions: &[Instruction],
        payer: Pubkey,
        blockhash: Hash,
        last_valid_block_height: u64,
        min_context_slot: u64,
    ) -> Result<Self> {
        ensure(
            !instructions.is_empty() && instructions.iter().all(|i| i.accounts.len() <= 256),
            "transaction instructions",
        )?;
        let keys = instructions
            .iter()
            .flat_map(|i| {
                i.accounts
                    .iter()
                    .map(|a| a.pubkey)
                    .chain(std::iter::once(i.program_id))
            })
            .chain(std::iter::once(payer))
            .collect::<std::collections::HashSet<_>>();
        ensure(keys.len() <= 256, "transaction account count")?;
        let signers = instructions
            .iter()
            .flat_map(|i| i.accounts.iter().filter(|a| a.is_signer).map(|a| a.pubkey))
            .chain(std::iter::once(payer))
            .collect::<std::collections::HashSet<_>>();
        ensure(signers.len() <= 255, "transaction signer count")?;
        let message = Message::new_with_blockhash(instructions, Some(&payer), &blockhash);
        let transaction = Transaction::new_unsigned(message);
        ensure(
            bincode::serialized_size(&transaction)
                .map_err(|_| Error::Invalid("transaction serialization"))?
                <= MAX_TRANSACTION_BYTES as u64,
            "transaction packet size",
        )?;
        Ok(Self {
            transaction,
            last_valid_block_height,
            min_context_slot,
        })
    }
    pub fn message_bytes(&self) -> Vec<u8> {
        self.transaction.message_data()
    }
    pub fn sign(mut self, signers: &[&dyn Signer]) -> Result<PendingTransaction> {
        let required = &self.transaction.message.account_keys
            [..usize::from(self.transaction.message.header.num_required_signatures)];
        let mut selected: Vec<&dyn Signer> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for signer in signers {
            let key = signer
                .try_pubkey()
                .map_err(|e| Error::Signing(e.to_string()))?;
            if required.contains(&key) && seen.insert(key) {
                selected.push(*signer);
            }
        }
        let blockhash = self.transaction.message.recent_blockhash.clone();
        self.transaction
            .try_sign(&selected, blockhash)
            .map_err(|e| Error::Signing(e.to_string()))?;
        PendingTransaction::from_transaction(
            &self.transaction,
            self.last_valid_block_height,
            self.min_context_slot,
        )
    }
    pub fn attach_signatures(
        mut self,
        signatures: &[(Pubkey, Signature)],
    ) -> Result<PendingTransaction> {
        let required = &self.transaction.message.account_keys
            [..usize::from(self.transaction.message.header.num_required_signatures)];
        ensure(signatures.len() == required.len(), "signature count")?;
        for (index, key) in required.iter().enumerate() {
            self.transaction.signatures[index] = signatures
                .iter()
                .find(|(address, _)| address == key)
                .ok_or(Error::Invalid("required signature"))?
                .1;
        }
        PendingTransaction::from_transaction(
            &self.transaction,
            self.last_valid_block_height,
            self.min_context_slot,
        )
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingTransaction {
    pub version: u8,
    pub signature: String,
    pub wire_base64: String,
    pub last_valid_block_height: u64,
    pub min_context_slot: u64,
}
impl std::fmt::Debug for PendingTransaction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingTransaction")
            .field("signature", &self.signature)
            .field("last_valid_block_height", &self.last_valid_block_height)
            .field("min_context_slot", &self.min_context_slot)
            .finish_non_exhaustive()
    }
}
impl PendingTransaction {
    pub fn from_transaction(
        transaction: &Transaction,
        last_valid_block_height: u64,
        min_context_slot: u64,
    ) -> Result<Self> {
        transaction
            .verify()
            .map_err(|_| Error::Invalid("transaction signatures"))?;
        let bytes = bincode::serialize(transaction)
            .map_err(|_| Error::Invalid("transaction serialization"))?;
        ensure(
            bytes.len() <= MAX_TRANSACTION_BYTES && !transaction.signatures.is_empty(),
            "transaction packet",
        )?;
        let pending = Self {
            version: 1,
            signature: transaction.signatures[0].to_string(),
            wire_base64: STANDARD.encode(bytes),
            last_valid_block_height,
            min_context_slot,
        };
        pending.validate()?;
        Ok(pending)
    }
    pub fn transaction(&self) -> Result<Transaction> {
        ensure(
            self.version == 1 && self.wire_base64.len() <= MAX_TRANSACTION_BYTES.div_ceil(3) * 4,
            "pending transaction format",
        )?;
        let bytes = STANDARD
            .decode(&self.wire_base64)
            .map_err(|_| Error::Invalid("transaction base64"))?;
        ensure(
            bytes.len() <= MAX_TRANSACTION_BYTES,
            "transaction packet size",
        )?;
        let transaction: Transaction =
            bincode::deserialize(&bytes).map_err(|_| Error::Invalid("transaction wire bytes"))?;
        ensure(
            bincode::serialize(&transaction)
                .map_err(|_| Error::Invalid("transaction serialization"))?
                == bytes,
            "canonical transaction bytes",
        )?;
        let m = &transaction.message;
        let required = usize::from(m.header.num_required_signatures);
        ensure(
            required > 0
                && required == transaction.signatures.len()
                && required <= m.account_keys.len()
                && usize::from(m.header.num_readonly_signed_accounts) < required
                && usize::from(m.header.num_readonly_unsigned_accounts)
                    <= m.account_keys.len() - required,
            "transaction message header",
        )?;
        ensure(
            m.instructions.iter().all(|ix| {
                usize::from(ix.program_id_index) < m.account_keys.len()
                    && ix
                        .accounts
                        .iter()
                        .all(|a| usize::from(*a) < m.account_keys.len())
            }),
            "transaction account indexes",
        )?;
        transaction
            .verify()
            .map_err(|_| Error::Invalid("transaction signatures"))?;
        Ok(transaction)
    }
    pub fn validate(&self) -> Result<Signature> {
        let transaction = self.transaction()?;
        let signature = Signature::from_str(&self.signature)
            .map_err(|_| Error::Invalid("pending signature"))?;
        ensure(
            transaction.signatures[0] == signature,
            "pending transaction signature binding",
        )?;
        Ok(signature)
    }
    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        self.validate()?;
        let path = path.as_ref();
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let temp = path.with_extension(format!("pending-{:032x}.tmp", rand::random::<u128>()));
        let result = (|| -> Result<()> {
            let mut file = options.open(&temp)?;
            file.write_all(&serde_json::to_vec(self)?)?;
            file.sync_all()?;
            fs::hard_link(&temp, path)?;
            fs::remove_file(&temp)?;
            #[cfg(unix)]
            {
                std::fs::File::open(
                    path.parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .unwrap_or(Path::new(".")),
                )?
                .sync_all()?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(8193)
            .read_to_end(&mut bytes)?;
        ensure(bytes.len() <= 8192, "pending transaction file size")?;
        let pending: Self = serde_json::from_slice(&bytes)?;
        pending.validate()?;
        Ok(pending)
    }
}
pub fn read_keypair(path: impl AsRef<Path>) -> Result<Keypair> {
    let mut bytes = Zeroizing::new(Vec::new());
    std::fs::File::open(path)?
        .take(4097)
        .read_to_end(&mut bytes)?;
    ensure(bytes.len() <= 4096, "keypair file size")?;
    let values: Zeroizing<Vec<u8>> = Zeroizing::new(serde_json::from_slice(&bytes)?);
    Keypair::try_from(values.as_slice()).map_err(|_| Error::Invalid("64-byte Solana keypair"))
}
