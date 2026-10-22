use crate::{Error, args::Cli};
use clap::{
    CommandFactory,
    builder::styling::{AnsiColor, Style},
    error::{ContextKind, ErrorKind},
};
use serde_json::{Value, json};
use std::{
    fmt::Write as _,
    io::{self, IsTerminal, Write},
};

const DOCS: &str = "https://fossa.ag/docs/providers/cli.md";
#[derive(Clone, Copy)]
pub struct Output {
    pub human: bool,
    pub errors_human: bool,
    pub color: bool,
}
impl Output {
    pub fn detect(json: bool) -> Self {
        Self::new(
            json,
            io::stdout().is_terminal(),
            io::stderr().is_terminal(),
            color_enabled(),
        )
    }
    fn new(json: bool, stdout_tty: bool, stderr_tty: bool, color: bool) -> Self {
        Self {
            human: !json && stdout_tty,
            errors_human: !json && stdout_tty && stderr_tty,
            color: !json && stdout_tty && color,
        }
    }
    pub fn result(self, value: Value) -> io::Result<()> {
        writeln!(
            io::stdout().lock(),
            "{}",
            self.render_result(&json!({"ok":true,"result":value}))
        )
    }
    pub fn error(self, value: &Value) -> io::Result<()> {
        writeln!(io::stderr().lock(), "{}", self.render_error(value))
    }
    fn render_result(self, value: &Value) -> String {
        if !self.human {
            return value.to_string();
        }
        let pretty = serde_json::to_string_pretty(value).expect("JSON value serialization");
        if self.color {
            highlight(&pretty)
        } else {
            pretty
        }
    }
    fn render_error(self, value: &Value) -> String {
        if !self.errors_human {
            return value.to_string();
        }
        let mut out = String::new();
        paint(
            &mut out,
            "error",
            AnsiColor::Red.on_default().bold(),
            self.color,
        );
        write!(
            &mut out,
            ": {}",
            safe_text(
                value["error"]["message"]
                    .as_str()
                    .unwrap_or("command failed")
            )
        )
        .unwrap();
        if let Some(recovery) = value.get("recovery").filter(|v| v.is_object()) {
            write!(
                &mut out,
                "\nsignature: {}\njournal: {}",
                safe_text(recovery["signature"].as_str().unwrap_or_default()),
                safe_text(recovery["journal"].as_str().unwrap_or_default())
            )
            .unwrap();
        }
        if let Some(hint) = value["error"]["hint"].as_str() {
            out.push('\n');
            paint(
                &mut out,
                "hint",
                AnsiColor::Cyan.on_default().bold(),
                self.color,
            );
            write!(&mut out, ": {}", safe_text(hint)).unwrap();
        }
        out
    }
}
pub fn color_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
        && std::env::var_os("TERM").is_none_or(|v| v != "dumb")
        && std::env::var_os("CLICOLOR").is_none_or(|v| v != "0")
}
pub fn help_color() -> clap::ColorChoice {
    if color_enabled() && !json_requested() {
        clap::ColorChoice::Auto
    } else {
        clap::ColorChoice::Never
    }
}
pub fn json_requested() -> bool {
    std::env::args_os().skip(1).any(|arg| arg == "--json")
}
fn paint(out: &mut String, text: &str, style: Style, color: bool) {
    if color {
        write!(out, "{}{text}{}", style.render(), style.render_reset()).unwrap();
    } else {
        out.push_str(text);
    }
}
fn safe_text(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_control() {
            output.extend(ch.escape_default());
        } else {
            output.push(ch);
        }
    }
    output
}

fn highlight(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len() + 128);
    let mut at = 0;
    while at < bytes.len() {
        let start = at;
        let style = match bytes[at] {
            b'"' => {
                at += 1;
                while at < bytes.len() {
                    if bytes[at] == b'\\' {
                        at += 2;
                    } else if bytes[at] == b'"' {
                        at += 1;
                        break;
                    } else {
                        at += 1;
                    }
                }
                let key = bytes[at..].iter().find(|b| !b.is_ascii_whitespace()) == Some(&b':');
                if key {
                    AnsiColor::Cyan.on_default()
                } else {
                    AnsiColor::Green.on_default()
                }
            }
            b'-' | b'0'..=b'9' => {
                at += 1;
                while at < bytes.len()
                    && matches!(bytes[at], b'0'..=b'9' | b'.' | b'e' | b'E' | b'+' | b'-')
                {
                    at += 1;
                }
                AnsiColor::Yellow.on_default()
            }
            b't' | b'f' | b'n' => {
                at += 1;
                while at < bytes.len() && bytes[at].is_ascii_alphabetic() {
                    at += 1;
                }
                AnsiColor::Magenta.on_default()
            }
            _ => {
                out.push(bytes[at] as char);
                at += 1;
                continue;
            }
        };
        paint(&mut out, &text[start..at], style, true);
    }
    out
}
fn safe_arguments(error: &clap::Error) -> Vec<String> {
    fn collect(command: &clap::Command, labels: &mut Vec<String>) {
        for arg in command.get_arguments() {
            if let Some(long) = arg.get_long() {
                labels.push(format!("--{long}"));
            }
            if arg.is_positional() {
                labels.push(format!("<{}>", arg.get_id().as_str().to_uppercase()));
            }
        }
        for sub in command.get_subcommands() {
            collect(sub, labels);
        }
    }
    let mut allowed = Vec::new();
    collect(&Cli::command(), &mut allowed);
    let mut found = Vec::new();
    for kind in [ContextKind::InvalidArg, ContextKind::PriorArg] {
        if let Some(value) = error.get(kind) {
            for token in value
                .to_string()
                .split(|c: char| c.is_whitespace() || matches!(c, ',' | '|' | '[' | ']'))
            {
                if allowed.iter().any(|label| label == token)
                    && !found.iter().any(|label| label == token)
                {
                    found.push(token.to_owned());
                }
            }
        }
    }
    found
}
pub fn parse_error(error: &clap::Error) -> Value {
    let args = safe_arguments(error);
    let argument = args.first().map(String::as_str);
    let label = argument.unwrap_or("argument");
    let message = match error.kind() {
        ErrorKind::MissingRequiredArgument => format!(
            "missing required {}",
            if args.is_empty() {
                "argument(s)".into()
            } else {
                args.join(", ")
            }
        ),
        ErrorKind::MissingSubcommand | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
            "missing command".into()
        }
        ErrorKind::UnknownArgument => "unknown option; check the command's --help".into(),
        ErrorKind::InvalidSubcommand => "unknown command; run fossa --help to list commands".into(),
        ErrorKind::ArgumentConflict => {
            if args.len() > 1 {
                format!("{} cannot be used together", args.join(" and "))
            } else {
                "conflicting arguments; check allowed combinations in command help".into()
            }
        }
        ErrorKind::InvalidValue
            if error
                .get(ContextKind::InvalidValue)
                .is_some_and(|value| value.to_string().is_empty()) =>
        {
            format!("{label} requires a value")
        }
        ErrorKind::ValueValidation | ErrorKind::InvalidValue => format!(
            "invalid {label}: {}",
            match argument {
                Some("--amount" | "--max-spend") =>
                    "expected positive USDC amount with at most 6 decimals",
                Some(
                    "--input-price" | "--output-price" | "--input-price-cap" | "--output-price-cap",
                ) =>
                    "expected price above 0 and at most 4294.967295 USDC per million tokens, with at most 6 decimals",
                Some("--cached-price" | "--cached-price-cap") =>
                    "expected price from 0 through 4294.967295 USDC per million tokens, with at most 6 decimals",
                Some(
                    "--wallet" | "--program-id" | "--market" | "--usdc-mint" | "--user"
                    | "--booking" | "--source" | "--destination" | "--request" | "--cache-provider"
                    | "<ADDRESS>",
                ) => "expected base58 Solana public key",
                Some("--active" | "--private" | "--hosted-rpc") => "expected true or false",
                Some("--kind" | "<KIND>") =>
                    "choose a supported account or metadata kind from --help",
                Some(
                    "--slot"
                    | "--capacity"
                    | "--speed-tps"
                    | "--max-input"
                    | "--max-output"
                    | "--min-speed"
                    | "--estimated-input"
                    | "--estimated-output"
                    | "--estimated-cached"
                    | "--uncached"
                    | "--cached"
                    | "--output"
                    | "--confirmation-seconds",
                ) => "expected unsigned integer within the range shown in --help",
                _ => "value does not match the required format; see --help",
            }
        ),
        ErrorKind::TooFewValues => format!("{label} requires a value"),
        ErrorKind::TooManyValues | ErrorKind::WrongNumberOfValues => {
            format!("wrong number of values for {label}")
        }
        _ => "invalid command arguments; check --help".into(),
    };
    json!({"ok":false,"error":{"code":"arguments","message":message,"kind":format!("{:?}",error.kind()),"argument":argument,"hint":"run fossa <command> --help for required flags and examples","docs":DOCS}})
}
pub fn runtime_error(error: &Error) -> Value {
    let code = match error {
        Error::Input(_) => "input",
        Error::Pending { .. } => "pending_transaction",
        Error::Io(_) => "io",
        Error::Json(_) => "json",
        Error::Sdk(_) => "sdk",
    };
    let (message, recovery, hint) = match error {
        Error::Pending {
            message,
            signature,
            journal,
        } => (
            message.clone(),
            Some(json!({"signature":signature,"journal":journal})),
            "run fossa transaction status --file <journal>, then resume the saved transaction before any replacement",
        ),
        Error::Io(_) => (
            error.to_string(),
            None,
            "check config, input and keypair file paths and permissions",
        ),
        Error::Sdk(sdk) => (
            crate::sdk_message(sdk),
            None,
            "check command arguments and connection settings; see fossa <command> --help",
        ),
        _ => (
            error.to_string(),
            None,
            "check fossa <command> --help or https://fossa.ag/docs/providers/cli.md",
        ),
    };
    json!({"ok":false,"recovery":recovery,"error":{"code":code,"message":message,"hint":hint,"docs":DOCS}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[test]
    fn terminal_pretty_json_colors_never_change_values_or_emit_input_controls() {
        let value = json!({"text":"héllo \"quote\" \u{1b}[31m","n":9007199254740993u64,"bool":true,"none":null});
        let plain = Output::new(false, true, true, false).render_result(&value);
        assert_eq!(serde_json::from_str::<Value>(&plain).unwrap(), value);
        assert!(plain.contains('\n'));
        assert!(!plain.contains('\u{1b}'));
        let colored = Output::new(false, true, true, true).render_result(&value);
        assert!(colored.contains("\u{1b}["));
        assert!(colored.contains("héllo"));
        assert!(colored.contains("\\u001b"));
        for mode in [
            Output::new(true, true, true, true),
            Output::new(false, false, true, true),
        ] {
            assert_eq!(mode.render_result(&value), value.to_string());
        }
    }
    #[test]
    fn terminal_errors_preserve_recovery_and_json_mode_is_structured() {
        let error = Error::Pending {
            message: "delivery uncertain".into(),
            signature: "signature".into(),
            journal: "/private/pending.json".into(),
        };
        let value = runtime_error(&error);
        let colored = Output::new(false, true, true, true).render_error(&value);
        assert!(colored.contains("\u{1b}["));
        assert!(colored.contains("signature: signature"));
        assert!(colored.contains("journal: /private/pending.json"));
        assert!(colored.contains("transaction status --file"));
        let plain = Output::new(false, true, true, false).render_error(&value);
        assert!(!plain.contains('\u{1b}'));
        assert!(plain.starts_with("error: delivery uncertain"));
        assert_eq!(
            Output::new(true, true, true, true).render_error(&value),
            value.to_string()
        );
        assert_eq!(
            Output::new(false, false, true, true).render_error(&value),
            value.to_string()
        );
    }
    #[test]
    fn terminal_messages_escape_untrusted_control_characters() {
        let error = Error::Pending {
            message: "bad\n\u{1b}]0;title\u{7}".into(),
            signature: "signature".into(),
            journal: "/tmp/a\r\u{1b}[2J.json".into(),
        };
        let rendered = Output::new(false, true, true, false).render_error(&runtime_error(&error));
        assert!(!rendered.contains('\u{1b}'));
        assert!(!rendered.contains('\r'));
        assert!(!rendered.contains('\u{7}'));
        assert!(rendered.contains("bad\\n"));
        assert!(rendered.contains("/tmp/a\\r"));
    }
    #[test]
    fn invalid_values_and_unknown_arguments_never_reach_diagnostics() {
        for args in [
            vec!["fossa", "--wallet", "secret-value"],
            vec!["fossa", "--unknown=secret-value"],
            vec![
                "fossa",
                "offer",
                "update",
                "--model",
                "m",
                "--input-price",
                "secret-value",
            ],
        ] {
            let error = Cli::try_parse_from(args).unwrap_err();
            let value = parse_error(&error);
            assert!(!value.to_string().contains("secret-value"));
            assert!(value["error"]["hint"].is_string());
        }
        let error = Cli::try_parse_from([
            "fossa",
            "offer",
            "update",
            "--model",
            "m",
            "--input-price",
            "bad",
        ])
        .unwrap_err();
        assert!(
            parse_error(&error)["error"]["message"]
                .as_str()
                .unwrap()
                .contains("6 decimals")
        );
    }
}
