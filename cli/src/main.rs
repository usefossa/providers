mod args;
mod commands;
mod config;
mod runtime;

use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Input(String),
    #[error(
        "{message}; signature {signature}; journal preserved at {journal}; use transaction status/resume before replacement"
    )]
    Pending {
        message: String,
        signature: String,
        journal: String,
    },
    #[error(transparent)]
    Sdk(#[from] fossa_sdk::Error),
    #[error("file operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
}
pub type Result<T> = std::result::Result<T, Error>;

#[tokio::main]
async fn main() {
    let cli = match args::Cli::parse_options() {
        Ok(cli) => cli,
        Err(error) => {
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                print!("{error}");
                return;
            }
            eprintln!(
                "{}",
                json!({"ok":false,"error":{"code":"arguments","message":"invalid arguments; run the command with --help","kind":format!("{:?}",error.kind()),"argument":if error.kind() == clap::error::ErrorKind::ValueValidation { error.get(clap::error::ContextKind::InvalidArg).map(ToString::to_string) } else { None },"docs":"https://fossa.ag/docs/providers/cli.md"}})
            );
            std::process::exit(2);
        }
    };
    match commands::run(cli).await {
        Ok(value) => println!("{}", json!({"ok":true,"result":value})),
        Err(error) => {
            let code = match &error {
                Error::Input(_) => "input",
                Error::Pending { .. } => "pending_transaction",
                Error::Io(_) => "io",
                Error::Json(_) => "json",
                Error::Sdk(_) => "sdk",
            };
            let recovery = match &error {
                Error::Pending {
                    signature, journal, ..
                } => Some(json!({"signature":signature,"journal":journal})),
                _ => None,
            };
            eprintln!(
                "{}",
                json!({"ok":false,"recovery":recovery,"error":{"code":code,"message":match &error { Error::Sdk(error) => sdk_message(error), _ => error.to_string() },"docs":"https://fossa.ag/docs/providers/cli.md"}})
            );
            std::process::exit(1);
        }
    }
}

pub fn sdk_message(error: &fossa_sdk::Error) -> String {
    match error {
        fossa_sdk::Error::Rpc { code, .. } => format!(
            "RPC error {code}; remote message omitted to protect request credentials and signed bytes"
        ),
        fossa_sdk::Error::Transaction(_) => {
            "transaction simulation or execution failed; signed bytes remain private".into()
        }
        _ => error.to_string(),
    }
}
