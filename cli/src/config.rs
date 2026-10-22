use crate::{Error, Result, args::Options};
use fossa_sdk::{Protocol, Pubkey};
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const API_URL: &str = "https://api.fossa.ag";
pub const PROGRAM_ID: Pubkey =
    Pubkey::from_str_const("F8q85nw6UScZCBKBJqVAu7ZdYhmT9XRsm6TqZzJqXSNT");
pub const MARKET: Pubkey = Pubkey::from_str_const("8wceBTFDAiC9WiT3xZGRf9Nf4xrz8eBCxExBtDWz1Som");
pub const USDC_MINT: Pubkey =
    Pubkey::from_str_const("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FileConfig {
    pub solana: Solana,
    pub marketplace: Marketplace,
    pub operator: Operator,
    pub hosted: Hosted,
    pub provider: Provider,
    pub journal_dir: Option<PathBuf>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Solana {
    pub rpc_url: Option<String>,
    #[serde(deserialize_with = "optional_pubkey")]
    pub usdc_mint: Option<Pubkey>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Marketplace {
    #[serde(deserialize_with = "optional_pubkey")]
    pub program_id: Option<Pubkey>,
    #[serde(deserialize_with = "optional_pubkey")]
    pub market: Option<Pubkey>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Operator {
    pub keypair: Option<PathBuf>,
    pub payer: Option<PathBuf>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Hosted {
    pub api_url: Option<String>,
    pub rpc: Option<bool>,
}
#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Provider {
    #[serde(deserialize_with = "optional_pubkey")]
    pub wallet: Option<Pubkey>,
    pub metadata_file: Option<PathBuf>,
    pub metadata_uri: Option<String>,
    pub speed_tps: Option<u16>,
    pub capacity: Option<u32>,
    pub private: Option<bool>,
    pub active: Option<bool>,
    pub model_book_limit: Option<u32>,
    pub bond_target_atomic: Option<String>,
    pub models: Vec<Model>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Model {
    pub id: String,
    pub metadata_file: PathBuf,
    pub metadata_uri: Option<String>,
    pub input_rate: u32,
    pub cached_rate: u32,
    pub output_rate: u32,
    pub speed_tps: Option<u16>,
    pub capacity: Option<u32>,
    pub private: Option<bool>,
    pub active: Option<bool>,
    pub bond_target_atomic: Option<String>,
    pub slot: Option<u32>,
}
pub struct Config {
    pub file: FileConfig,
    pub base: PathBuf,
    pub api_url: String,
    pub rpc_url: Option<String>,
    pub hosted_rpc: bool,
    pub keypair: Option<PathBuf>,
    pub payer: Option<PathBuf>,
    pub wallet: Option<Pubkey>,
    pub program_id: Pubkey,
    pub market: Pubkey,
    pub mint: Pubkey,
    pub journal: PathBuf,
}
impl Config {
    pub fn load(options: &Options) -> Result<Self> {
        let cwd = std::env::current_dir()?;
        let (file, base) = if let Some(path) = &options.config {
            let path = cwd.join(path);
            let input = std::fs::read_to_string(&path)?;
            let file = toml::from_str(&input).map_err(|_| {
                Error::Input(
                    "invalid TOML configuration or unknown field; check cli/config.example.toml"
                        .into(),
                )
            })?;
            (file, path.parent().unwrap_or(&cwd).to_path_buf())
        } else {
            (FileConfig::default(), cwd.clone())
        };
        Self::resolve(options, file, base, &cwd)
    }
    pub fn resolve(o: &Options, file: FileConfig, base: PathBuf, cwd: &Path) -> Result<Self> {
        let rpc_url = if o.hosted_rpc == Some(true) {
            None
        } else {
            o.rpc_url.clone().or_else(|| file.solana.rpc_url.clone())
        };
        let hosted_rpc = if o.rpc_url.is_some() {
            false
        } else {
            o.hosted_rpc
                .or(file.hosted.rpc)
                .unwrap_or(rpc_url.is_none())
        };
        if hosted_rpc && rpc_url.is_some() {
            return Err(Error::Input(
                "configure one RPC transport: solana.rpc_url or hosted.rpc = true".into(),
            ));
        }
        let keypair = o
            .keypair
            .as_ref()
            .map(|p| cwd.join(p))
            .or_else(|| file.operator.keypair.as_ref().map(|p| base.join(p)));
        let payer = o
            .payer
            .as_ref()
            .map(|p| cwd.join(p))
            .or_else(|| file.operator.payer.as_ref().map(|p| base.join(p)));
        let journal = o
            .journal_dir
            .as_ref()
            .map(|p| cwd.join(p))
            .or_else(|| file.journal_dir.as_ref().map(|p| base.join(p)))
            .unwrap_or_else(|| base.join("transactions"));
        Ok(Self {
            api_url: fossa_sdk::hosted::validate_api_origin(
                &o.api_url
                    .clone()
                    .or_else(|| file.hosted.api_url.clone())
                    .unwrap_or_else(|| API_URL.into()),
            )?,
            rpc_url,
            hosted_rpc,
            keypair,
            payer,
            journal,
            wallet: o.wallet.or(file.provider.wallet),
            program_id: o
                .program_id
                .or(file.marketplace.program_id)
                .unwrap_or(PROGRAM_ID),
            market: o.market.or(file.marketplace.market).unwrap_or(MARKET),
            mint: o.usdc_mint.or(file.solana.usdc_mint).unwrap_or(USDC_MINT),
            file,
            base,
        })
    }
    pub fn protocol(&self) -> Protocol {
        Protocol {
            program: self.program_id,
            market: self.market,
            mint: self.mint,
        }
    }
}

fn optional_pubkey<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Pubkey>, D::Error> {
    Option::<String>::deserialize(deserializer)?
        .map(|s| {
            s.parse()
                .map_err(|_| serde::de::Error::custom("invalid base58 public key"))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn base58_toml_and_precedence_keep_paths_and_rpc_choices_explicit() {
        let key = Pubkey::new_from_array([7; 32]);
        let file:FileConfig=toml::from_str(&format!("[solana]\nrpc_url='https://rpc.invalid?key=secret'\nusdc_mint='{key}'\n[marketplace]\nprogram_id='{key}'\nmarket='{key}'\n[operator]\nkeypair='wallet.json'\n[provider]\nwallet='{key}'\n")).unwrap();
        let opts = Options {
            hosted_rpc: Some(true),
            keypair: Some("override.json".into()),
            ..Default::default()
        };
        let config = Config::resolve(&opts, file, "/config".into(), Path::new("/cwd")).unwrap();
        assert!(config.hosted_rpc);
        assert!(config.rpc_url.is_none());
        assert_eq!(config.keypair, Some(PathBuf::from("/cwd/override.json")));
        assert_eq!(config.journal, PathBuf::from("/config/transactions"));
        assert_eq!(config.wallet, Some(key));
        assert_eq!(config.program_id, key);
    }
    #[test]
    fn own_rpc_override_disables_configured_hosted_rpc() {
        let file: FileConfig = toml::from_str("[hosted]\nrpc=true\n").unwrap();
        let opts = Options {
            rpc_url: Some("http://localhost:9".into()),
            ..Default::default()
        };
        let config = Config::resolve(&opts, file, "/config".into(), Path::new("/cwd")).unwrap();
        assert!(!config.hosted_rpc);
        assert_eq!(config.rpc_url.as_deref(), Some("http://localhost:9"));
    }
}
