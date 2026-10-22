mod args;
mod commands;
mod config;
mod output;
mod runtime;

use clap::CommandFactory;
use std::io::Write;

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

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if std::env::args_os().len() == 1 {
        let _ = args::Cli::command()
            .color(output::help_color())
            .print_help();
        let _ = writeln!(std::io::stdout());
        return;
    }
    let mode = output::Output::detect(output::json_requested());
    let cli = match args::Cli::parse_options() {
        Ok(cli) => cli,
        Err(error) => {
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) {
                let _ = error.print();
                return;
            }
            let _ = mode.error(&output::parse_error(&error));
            std::process::exit(2);
        }
    };
    let mode = output::Output::detect(cli.options.json);
    match commands::run(cli).await {
        Ok(value) => {
            let _ = mode.result(value);
        }
        Err(error) => {
            let _ = mode.error(&output::runtime_error(&error));
            std::process::exit(1);
        }
    }
}
pub fn sdk_message(error: &fossa_sdk::Error) -> String {
    match error {
        fossa_sdk::Error::Rpc{code,..}=>format!("RPC error {code}; check RPC availability and request permissions"),
        fossa_sdk::Error::Transaction(_)=>"transaction simulation or execution failed; check account state and balances before retrying".into(),
        fossa_sdk::Error::Json(error)=>format!("invalid server or keypair JSON at line {}, column {}",error.line(),error.column()),
        _=>error.to_string(),
    }
}
