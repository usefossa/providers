use crate::{Error, Result, args::Options, config::Config};
use fossa_sdk::{
    Instruction, Keypair, PendingTransaction, Protocol, Pubkey, RpcClient, Signer,
    hosted::HostedClient, transaction::read_keypair,
};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};

pub fn keypair(config: &Config) -> Result<Arc<Keypair>> {
    let path = config
        .keypair
        .as_ref()
        .ok_or_else(|| Error::Input("set --keypair or operator.keypair for signing".into()))?;
    let signer = Arc::new(read_keypair(path)?);
    if config
        .wallet
        .is_some_and(|wallet| wallet != signer.pubkey())
    {
        return Err(Error::Input(
            "keypair does not match configured wallet".into(),
        ));
    }
    Ok(signer)
}
pub fn wallet(config: &Config) -> Result<Pubkey> {
    config
        .wallet
        .map(Ok)
        .unwrap_or_else(|| keypair(config).map(|s| s.pubkey()))
}
pub fn hosted(config: &Config) -> Result<HostedClient> {
    if config.keypair.is_none() {
        return Err(Error::Input("hosted RPC and uploads require request authentication; set --keypair PATH or use --rpc-url URL for key-free reads".into()));
    }
    Ok(HostedClient::new(&config.api_url, keypair(config)?)?)
}
pub fn rpc(config: &Config) -> Result<RpcClient> {
    if config.hosted_rpc {
        Ok(RpcClient::hosted(hosted(config)?)
            .with_commitment(fossa_sdk::rpc::Commitment::Confirmed))
    } else {
        Ok(RpcClient::new(config.rpc_url.as_deref().ok_or_else(|| {
            Error::Input("RPC disabled; set --rpc-url URL or enable --hosted-rpc".into())
        })?)?
        .with_commitment(fossa_sdk::rpc::Commitment::Confirmed))
    }
}
pub struct Runtime {
    pub config: Config,
    pub options: Options,
    pub rpc: RpcClient,
    pub protocol: Protocol,
}
impl Runtime {
    pub async fn connect(config: Config, options: Options) -> Result<Self> {
        let rpc = rpc(&config)?;
        let protocol = config.protocol();
        rpc.validate_market(&protocol).await?;
        Ok(Self {
            config,
            options,
            rpc,
            protocol,
        })
    }
    pub fn wallet(&self) -> Result<Pubkey> {
        wallet(&self.config)
    }
    pub fn payer(&self) -> Result<Pubkey> {
        self.config
            .payer
            .as_ref()
            .map(|p| read_keypair(p).map(|k| k.pubkey()).map_err(Error::from))
            .unwrap_or_else(|| self.wallet())
    }
    pub fn ensure_no_pending(&self) -> Result<()> {
        if self.config.journal.join("pending.json").exists() {
            return Err(Error::Input(format!(
                "pending transaction exists; inspect/resume with transaction status/resume --file {}",
                self.config.journal.join("pending.json").display()
            )));
        }
        Ok(())
    }
    pub async fn transact(
        &self,
        label: &str,
        instructions: Vec<Instruction>,
        extra: &[Keypair],
        details: Value,
    ) -> Result<Value> {
        let payer = self.payer()?;
        let plan = json!({"mode":"plan","operation":label,"payer":payer.to_string(),"details":details,"instructions":instruction_view(&instructions)});
        if !self.options.execute {
            return Ok(plan);
        }
        self.ensure_no_pending()?;
        let signer = keypair(&self.config)?;
        let payer_key = self.config.payer.as_ref().map(read_keypair).transpose()?;
        let mut signers: Vec<&dyn Signer> = vec![signer.as_ref()];
        if let Some(key) = &payer_key {
            signers.push(key);
        }
        signers.extend(extra.iter().map(|key| key as &dyn Signer));
        let pending = self
            .rpc
            .prepare(&instructions, payer)
            .await?
            .sign(&signers)?;
        self.rpc.simulate(&pending).await?.check()?;
        secure_directory(&self.config.journal)?;
        let path = self.config.journal.join("pending.json");
        pending.save(&path)?;
        self.rpc
            .broadcast(&pending)
            .await
            .map_err(|e| pending_error(&pending, &path, e))?;
        let status = self
            .rpc
            .confirm(
                &pending,
                Duration::from_secs(self.options.confirmation_seconds),
            )
            .await
            .map_err(|e| pending_error(&pending, &path, e))?;
        let archive = archive_pending(&path, &pending)?;
        Ok(
            json!({"mode":"executed","operation":label,"details":details,"signature":pending.signature,"status":status,"journal":archive}),
        )
    }
}
pub fn instruction_view(instructions: &[Instruction]) -> Vec<Value> {
    instructions.iter().map(|ix| json!({"program":ix.program_id.to_string(),"opcode":ix.data.first(),"data_hex":hex(&ix.data),"accounts":ix.accounts.iter().map(|a| json!({"address":a.pubkey.to_string(),"signer":a.is_signer,"writable":a.is_writable})).collect::<Vec<_>>()})).collect()
}
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut text, b| {
            write!(&mut text, "{b:02x}").unwrap();
            text
        })
}
fn secure_directory(path: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)?;
    Ok(())
}
pub fn archive_pending(path: &Path, pending: &PendingTransaction) -> Result<std::path::PathBuf> {
    let archive = path.with_file_name(format!("{}.json", pending.signature));
    if path == archive {
        return Ok(archive);
    }
    if archive.exists() {
        let prior = PendingTransaction::load(&archive)?;
        if prior.wire_base64 != pending.wire_base64 {
            return Err(Error::Input(
                "journal archive collision; pending journal preserved".into(),
            ));
        }
    } else {
        std::fs::hard_link(path, &archive)?;
    }
    #[cfg(unix)]
    {
        std::fs::File::open(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?
        .sync_all()?;
    }
    std::fs::remove_file(path)?;
    #[cfg(unix)]
    {
        std::fs::File::open(
            archive
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?
        .sync_all()?;
    }
    Ok(archive)
}
pub fn pending_error(pending: &PendingTransaction, path: &Path, error: fossa_sdk::Error) -> Error {
    Error::Pending {
        message: crate::sdk_message(&error),
        signature: pending.signature.clone(),
        journal: path.display().to_string(),
    }
}
