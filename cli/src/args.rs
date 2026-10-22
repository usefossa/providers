use clap::{Args, Parser, Subcommand, ValueEnum};
use fossa_sdk::{Pubkey, amount};
use std::path::PathBuf;

pub const DOCS: &str = "docs: https://fossa.ag/docs/providers/cli.md\nsdk: https://fossa.ag/docs/providers/sdk.md\nstart: https://fossa.ag/docs/providers/quick-start.md";

#[derive(Parser, Debug)]
#[command(name = "fossa-provider", version, about = "Inspect and manage Fossa providers; mutations plan until --execute", after_help = DOCS)]
pub struct Cli {
    #[command(flatten)]
    pub options: Options,
    #[command(subcommand)]
    pub command: Command,
}
#[derive(Args, Debug, Default)]
pub struct Options {
    /// Explicit TOML config; file paths inside resolve relative to its directory
    #[arg(long, global = true, env = "FOSSA_CONFIG", hide_env_values = true)]
    pub config: Option<PathBuf>,
    /// Own Solana RPC URL; never included in output
    #[arg(long, global = true, env = "FOSSA_RPC_URL", hide_env_values = true)]
    pub rpc_url: Option<String>,
    /// Use Fossa RPC; read requests sign authentication messages, never transactions
    #[arg(
        long,
        global = true,
        env = "FOSSA_HOSTED_RPC",
        hide_env_values = true,
        num_args = 0..=1,
        default_missing_value = "true",
        require_equals = true,
    )]
    pub hosted_rpc: Option<bool>,
    /// Fossa API origin for public config and optional hosting
    #[arg(long, global = true, env = "FOSSA_API_URL", hide_env_values = true)]
    pub api_url: Option<String>,
    /// Standard Solana JSON keypair outside git
    #[arg(long, global = true, env = "FOSSA_KEYPAIR", hide_env_values = true)]
    pub keypair: Option<PathBuf>,
    /// Authority for planning/inspection without loading a signing key
    #[arg(long, global = true, env = "FOSSA_WALLET", hide_env_values = true)]
    pub wallet: Option<Pubkey>,
    /// Optional separate fee payer keypair
    #[arg(long, global = true, env = "FOSSA_PAYER", hide_env_values = true)]
    pub payer: Option<PathBuf>,
    #[arg(long, global = true, env = "FOSSA_PROGRAM_ID", hide_env_values = true)]
    pub program_id: Option<Pubkey>,
    #[arg(long, global = true, env = "FOSSA_MARKET", hide_env_values = true)]
    pub market: Option<Pubkey>,
    #[arg(long, global = true, env = "FOSSA_USDC_MINT", hide_env_values = true)]
    pub usdc_mint: Option<Pubkey>,
    /// Authorize this command's uploads, transaction signing and broadcast
    #[arg(long, global = true)]
    pub execute: bool,
    /// Durable transaction directory; defaults to transactions beside config (or cwd)
    #[arg(long, global = true, env = "FOSSA_JOURNAL_DIR", hide_env_values = true)]
    pub journal_dir: Option<PathBuf>,
    /// Confirmation wait; timeout preserves pending transaction for resume
    #[arg(long, global = true, default_value_t = 30)]
    pub confirmation_seconds: u64,
}
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Inspect effective configuration without exposing RPC credentials
    #[command(subcommand, after_help = DOCS)]
    Config(ConfigCommand),
    /// Reconcile configured provider, model metadata, offers and bond targets
    #[command(after_help = DOCS)]
    Onboard {
        /// Explicit permission for provider approval when market admin signs
        #[arg(long)]
        allow_approval: bool,
        /// Optional market admin signer; required only for admin-owned operations
        #[arg(long)]
        admin: Option<PathBuf>,
    },
    /// Inspect and edit provider profile
    #[command(subcommand, after_help = DOCS)]
    Provider(ProviderCommand),
    /// Inspect model accounts or update metadata owned by your wallet
    #[command(subcommand, after_help = DOCS)]
    Model(ModelCommand),
    /// Inspect offers; edit selected fields while preserving others
    #[command(subcommand, after_help = DOCS)]
    Offer(OfferCommand),
    /// Fund bond or request/execute delayed unbonding
    #[command(subcommand, after_help = DOCS)]
    Bond(BondCommand),
    /// Validate exact document bytes offline or upload immutable metadata
    #[command(subcommand, after_help = DOCS)]
    Metadata(MetadataCommand),
    /// Upload an immutable PNG, JPEG or WebP image
    #[command(subcommand, after_help = DOCS)]
    Image(ImageCommand),
    /// Read and decode an account with SDK ownership/layout validation
    #[command(subcommand, after_help = DOCS)]
    Account(AccountCommand),
    /// Inspect or safely resume a durable signed transaction
    #[command(subcommand, after_help = DOCS)]
    Transaction(TransactionCommand),
    /// User account operations for integration workflows; no inference requests
    #[command(subcommand, after_help = DOCS)]
    User(UserCommand),
    /// Inspect and settle bookings; all mutation commands plan by default
    #[command(subcommand, after_help = DOCS)]
    Booking(BookingCommand),
}
#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Show selected configuration and defaults (RPC URL redacted)
    #[command(after_help = DOCS)]
    Show,
}
#[derive(Subcommand, Debug)]
pub enum ProviderCommand {
    /// Inspect provider PDA and committed metadata reference
    #[command(after_help = DOCS)]
    Show,
    /// Register provider profile; approval remains a separate market-admin decision
    #[command(after_help = DOCS)]
    Register(DocumentArgs),
    /// Edit a verified existing profile, preserving unspecified fields and extensions
    #[command(after_help = DOCS)]
    Update(ProfileArgs),
}
#[derive(Args, Debug)]
pub struct DocumentArgs {
    /// Valid metadata JSON; exact bytes determine SHA-256
    #[arg(long)]
    pub metadata_file: PathBuf,
    /// Custom immutable HTTPS location; omit for Fossa hosting on --execute
    #[arg(long)]
    pub metadata_uri: Option<String>,
}
#[derive(Args, Debug)]
#[group(required = true, multiple = true)]
pub struct ProfileArgs {
    /// Replace display name
    #[arg(long)]
    pub name: Option<String>,
    /// Replace image with an existing HTTPS image URL
    #[arg(long, conflicts_with = "image_file")]
    pub image_url: Option<String>,
    /// Upload local PNG/JPEG/WebP before publishing updated profile
    #[arg(long, conflicts_with = "image_url")]
    pub image_file: Option<PathBuf>,
    /// Replace document with this validated file instead of fetching current metadata
    #[arg(long, conflicts_with_all = ["name", "image_url", "image_file"])]
    pub metadata_file: Option<PathBuf>,
    /// Custom immutable URI for the resulting exact document bytes
    #[arg(long, requires = "metadata_file")]
    pub metadata_uri: Option<String>,
}
#[derive(Subcommand, Debug)]
pub enum ModelCommand {
    /// Inspect model, books and metadata reference
    #[command(after_help = DOCS)]
    Show(ModelArg),
    /// Change model metadata; signer must own model metadata authority
    #[command(after_help = DOCS)]
    UpdateMetadata {
        #[command(flatten)]
        model: ModelArg,
        #[command(flatten)]
        document: DocumentArgs,
    },
}
#[derive(Args, Debug)]
pub struct ModelArg {
    /// Exact model identifier (hashed by SDK)
    #[arg(long)]
    pub model: String,
}
#[derive(Args, Debug)]
pub struct OfferArg {
    #[command(flatten)]
    pub model: ModelArg,
    /// Offer slot; omit only when exactly one belongs to the configured wallet
    #[arg(long)]
    pub slot: Option<u32>,
}
#[derive(Subcommand, Debug)]
pub enum OfferCommand {
    /// Show every offer and capacity entry for a model
    #[command(after_help = DOCS)]
    List(ModelArg),
    /// Patch only named fields; prices are USDC per 1,000,000 tokens
    #[command(after_help = DOCS)]
    Update {
        #[command(flatten)]
        offer: OfferArg,
        #[command(flatten)]
        patch: OfferPatch,
    },
}
#[derive(Args, Debug, Default)]
#[group(required = true, multiple = true)]
pub struct OfferPatch {
    /// USDC per million uncached input tokens, at most 6 decimals
    #[arg(long, value_parser = parse_positive_rate)]
    pub input_price: Option<u32>,
    /// USDC per million cached input tokens, at most 6 decimals
    #[arg(long, value_parser = parse_rate)]
    pub cached_price: Option<u32>,
    /// USDC per million output tokens, at most 6 decimals
    #[arg(long, value_parser = parse_positive_rate)]
    pub output_price: Option<u32>,
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub capacity: Option<u32>,
    #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
    pub speed_tps: Option<u16>,
    #[arg(long, action = clap::ArgAction::Set)]
    pub active: Option<bool>,
    #[arg(long, action = clap::ArgAction::Set)]
    pub private: Option<bool>,
}
#[derive(Subcommand, Debug)]
pub enum BondCommand {
    /// Deposit USDC into selected offer bond vault
    #[command(after_help = DOCS)]
    Fund {
        #[command(flatten)]
        offer: OfferArg,
        #[arg(long, value_parser = parse_positive_usdc)]
        amount: u64,
        /// Source token account; defaults to provider USDC associated account
        #[arg(long)]
        source: Option<Pubkey>,
    },
    /// Create a delayed unbond request; never decreases bond immediately
    #[command(after_help = DOCS)]
    Request {
        #[command(flatten)]
        offer: OfferArg,
        #[arg(long, value_parser = parse_positive_usdc)]
        amount: u64,
    },
    /// Execute an existing ready unbond request
    #[command(after_help = DOCS)]
    Execute {
        #[arg(long)]
        request: Pubkey,
    },
}
#[derive(Clone, Copy, ValueEnum, Debug)]
pub enum MetadataKind {
    Provider,
    Model,
}
impl From<MetadataKind> for fossa_sdk::metadata::Kind {
    fn from(value: MetadataKind) -> Self {
        match value {
            MetadataKind::Provider => Self::Provider,
            MetadataKind::Model => Self::Model,
        }
    }
}
#[derive(Subcommand, Debug)]
pub enum MetadataCommand {
    /// Validate local bytes offline; print hash, size and parsed document
    #[command(after_help = DOCS)]
    Validate { kind: MetadataKind, file: PathBuf },
    /// Plan or upload exact local bytes; upload alone never updates on-chain metadata
    #[command(after_help = DOCS)]
    Upload { kind: MetadataKind, file: PathBuf },
}
#[derive(Subcommand, Debug)]
pub enum ImageCommand {
    /// Plan or upload immutable image bytes (maximum 2 MiB)
    #[command(after_help = DOCS)]
    Upload { file: PathBuf },
}
#[derive(Clone, Copy, ValueEnum, Debug)]
pub enum AccountKind {
    Market,
    Provider,
    Model,
    ModelMetadata,
    User,
    Terms,
    Capacity,
    Booking,
    Unbond,
    ReceiptBucket,
    Token,
}
#[derive(Subcommand, Debug)]
pub enum AccountCommand {
    /// Fetch typed account; rejects incorrect owner, size, magic or PDA binding
    #[command(after_help = DOCS)]
    Show {
        address: Pubkey,
        #[arg(long)]
        kind: AccountKind,
    },
}
#[derive(Subcommand, Debug)]
pub enum TransactionCommand {
    /// Reconcile saved signature with chain; never deletes or broadcasts
    #[command(after_help = DOCS)]
    Status {
        #[arg(long)]
        file: PathBuf,
    },
    /// Plan or resume exact signed bytes after status/expiry reconciliation
    #[command(after_help = DOCS)]
    Resume {
        #[arg(long)]
        file: PathBuf,
    },
}
#[derive(Subcommand, Debug)]
pub enum UserCommand {
    /// Inspect an existing user account
    #[command(after_help = DOCS)]
    Show {
        #[arg(long)]
        user: Pubkey,
    },
    /// Create user account and its USDC vault; returns generated account address
    #[command(after_help = DOCS)]
    Init,
    /// Deposit USDC into a user account owned by signer
    #[command(after_help = DOCS)]
    Deposit {
        #[arg(long)]
        user: Pubkey,
        #[arg(long, value_parser = parse_positive_usdc)]
        amount: u64,
        #[arg(long)]
        source: Option<Pubkey>,
    },
    /// Withdraw free USDC from a user account owned by signer
    #[command(after_help = DOCS)]
    Withdraw {
        #[arg(long)]
        user: Pubkey,
        #[arg(long, value_parser = parse_positive_usdc)]
        amount: u64,
        #[arg(long)]
        destination: Option<Pubkey>,
    },
}
#[derive(Subcommand, Debug)]
pub enum BookingCommand {
    /// Create one-shot booking from capped prices; never sends an inference request
    #[command(after_help = DOCS)]
    Create(BookArgs),
    /// Inspect booking state
    #[command(after_help = DOCS)]
    Show {
        #[arg(long)]
        booking: Pubkey,
    },
    /// Claim measured token usage for a one-shot booking; no inference is performed
    #[command(after_help = DOCS)]
    Claim {
        #[arg(long)]
        booking: Pubkey,
        #[arg(long)]
        uncached: u32,
        #[arg(long, default_value_t = 0)]
        cached: u32,
        #[arg(long)]
        output: u32,
    },
    /// Provider releases booking without charging; generation protects reusable bookings
    #[command(after_help = DOCS)]
    Release {
        #[arg(long)]
        booking: Pubkey,
    },
    /// Expire booking after its claim deadline
    #[command(after_help = DOCS)]
    Expire {
        #[arg(long)]
        booking: Pubkey,
    },
    /// Finalize booking after its dispute deadline
    #[command(after_help = DOCS)]
    Finalize {
        #[arg(long)]
        booking: Pubkey,
    },
}
pub fn parse_rate(value: &str) -> Result<u32, String> {
    amount::parse_usdc(value)
        .map_err(|_| "expected nonnegative USDC price with at most 6 decimals".into())
        .and_then(|n| {
            u32::try_from(n).map_err(|_| "price exceeds 4294.967295 USDC per million tokens".into())
        })
}
pub fn parse_positive_usdc(value: &str) -> Result<u64, String> {
    let n = amount::parse_usdc(value)
        .map_err(|_| "expected positive USDC amount with at most 6 decimals")?;
    if n == 0 {
        Err("amount must be positive".into())
    } else {
        Ok(n)
    }
}

#[derive(Args, Debug)]
pub struct BookArgs {
    #[arg(long)]
    pub user: Pubkey,
    #[command(flatten)]
    pub model: ModelArg,
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub max_input: u32,
    #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
    pub max_output: u32,
    /// Maximum reservation in USDC, at most 6 decimals
    #[arg(long, value_parser = parse_positive_usdc)]
    pub max_spend: u64,
    #[arg(long, value_parser = parse_positive_rate)]
    pub input_price_cap: u32,
    #[arg(long, value_parser = parse_rate)]
    pub cached_price_cap: u32,
    #[arg(long, value_parser = parse_positive_rate)]
    pub output_price_cap: u32,
    #[arg(long, default_value_t = 1)]
    pub min_speed: u16,
    #[arg(long)]
    pub private: bool,
    #[arg(long)]
    pub estimated_input: Option<u32>,
    #[arg(long)]
    pub estimated_output: Option<u32>,
    #[arg(long, default_value_t = 0)]
    pub estimated_cached: u32,
    #[arg(long)]
    pub cache_provider: Option<Pubkey>,
}
pub fn parse_positive_rate(value: &str) -> Result<u32, String> {
    let n = parse_rate(value)?;
    if n == 0 {
        Err("price must be positive".into())
    } else {
        Ok(n)
    }
}

impl Cli {
    pub fn parse_options() -> Result<Self, clap::Error> {
        use clap::{CommandFactory, FromArgMatches, parser::ValueSource};
        let matches = Self::command().try_get_matches()?;
        let mut cli = Self::from_arg_matches(&matches)?;
        if cli.options.hosted_rpc == Some(true) && cli.options.rpc_url.is_some() {
            match (
                matches.value_source("rpc_url"),
                matches.value_source("hosted_rpc"),
            ) {
                (Some(ValueSource::CommandLine), Some(ValueSource::EnvVariable)) => {
                    cli.options.hosted_rpc = Some(false)
                }
                (Some(ValueSource::EnvVariable), Some(ValueSource::CommandLine)) => {
                    cli.options.rpc_url = None
                }
                _ => {
                    return Err(clap::Error::raw(
                        clap::error::ErrorKind::ArgumentConflict,
                        "select one RPC transport: --rpc-url or --hosted-rpc",
                    ));
                }
            }
        }
        Ok(cli)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    #[test]
    fn command_tree_is_consistent_and_every_help_links_docs() {
        let mut command = Cli::command();
        command.clone().debug_assert();
        fn visit(command: &mut clap::Command) {
            if command.get_name() == "help" {
                return;
            }
            let help = command.render_long_help().to_string();
            for url in [
                "https://fossa.ag/docs/providers/cli.md",
                "https://fossa.ag/docs/providers/sdk.md",
                "https://fossa.ag/docs/providers/quick-start.md",
            ] {
                assert!(help.contains(url), "{} missing docs", command.get_name());
            }
            for sub in command.get_subcommands_mut() {
                visit(sub);
            }
        }
        visit(&mut command);
    }
    #[test]
    fn prices_and_amounts_never_use_float_rounding() {
        assert_eq!(parse_rate("1.234567").unwrap(), 1_234_567);
        assert_eq!(parse_rate("4294.967295").unwrap(), u32::MAX);
        for bad in ["1e2", "-1", "0.0000001", "NaN", "4294.967296"] {
            assert!(parse_rate(bad).is_err());
        }
        assert!(parse_positive_usdc("0").is_err());
        assert!(
            Cli::try_parse_from([
                "fossa-provider",
                "offer",
                "update",
                "--model",
                "example/model"
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "fossa-provider",
                "offer",
                "update",
                "--model",
                "example/model",
                "--input-price",
                "0"
            ])
            .is_err()
        );
    }
}
