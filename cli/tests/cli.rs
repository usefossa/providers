use base64::{Engine, engine::general_purpose::STANDARD};
use fossa_sdk::{
    Hash, Keypair, PendingTransaction, Protocol, Pubkey, Signer, transaction::PreparedTransaction,
};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::{Command, Output},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
struct Mock {
    url: String,
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.url.trim_start_matches("http://"));
        if let Some(j) = self.join.take() {
            j.join().unwrap();
        }
    }
}
fn mock(handler: impl Fn(&str, &Value) -> Value + Send + 'static) -> Mock {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let stop = Arc::new(AtomicBool::new(false));
    let exit = stop.clone();
    let join = thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            if exit.load(Ordering::SeqCst) {
                break;
            }
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buf = [0; 4096];
            let end = loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend(&buf[..n]);
                if let Some(n) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                    break n + 4;
                }
            };
            let head = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let len = head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|s| s.parse::<usize>().ok())
                })
                .unwrap_or(0);
            while bytes.len() < end + len {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                bytes.extend(&buf[..n]);
            }
            let request: Value = serde_json::from_slice(&bytes[end..end + len]).unwrap();
            let path = head.split_whitespace().nth(1).unwrap();
            let body = handler(path, &request).to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        }
    });
    Mock {
        url,
        stop,
        join: Some(join),
    }
}
fn cmd(dir: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_fossa-provider"));
    c.env_clear().current_dir(dir);
    c
}
fn result(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice::<Value>(&output.stdout).unwrap()["result"].clone()
}
fn protocol_args(c: &mut Command, rpc: &str, wallet: Pubkey) {
    c.args([
        "--rpc-url",
        rpc,
        "--program-id",
        &key(1).to_string(),
        "--market",
        &key(2).to_string(),
        "--usdc-mint",
        &key(3).to_string(),
        "--wallet",
        &wallet.to_string(),
    ]);
}
fn rpc(req: &Value, value: Value) -> Value {
    json!({"jsonrpc":"2.0","id":req["id"],"result":value})
}
fn market_account(admin: Pubkey) -> Value {
    let mut data = b"FOSSAV20".to_vec();
    data.extend(admin.as_ref());
    data.extend(key(3).as_ref());
    json!({"owner":key(1).to_string(),"lamports":1,"executable":false,"data":[STANDARD.encode(data),"base64"]})
}
fn signed() -> PendingTransaction {
    let signer = Keypair::new();
    let protocol = Protocol {
        program: key(1),
        market: key(2),
        mint: key(3),
    };
    PreparedTransaction::new(
        &[protocol.approve_provider(key(5), signer.pubkey(), false)],
        signer.pubkey(),
        Hash::new_from_array([8; 32]),
        100,
        9,
    )
    .unwrap()
    .sign(&[&signer])
    .unwrap()
}
#[test]
fn version_and_offline_validation_need_no_config_network_or_keys() {
    let dir = tempfile::tempdir().unwrap();
    let version = cmd(dir.path()).arg("--version").output().unwrap();
    assert_eq!(
        String::from_utf8(version.stdout).unwrap(),
        "fossa-provider 0.2.0\n"
    );
    let raw = b"{ \"name\": \"Example\", \"endpoint\": \"https://example.com/infer\" }\n";
    std::fs::write(dir.path().join("provider.json"), raw).unwrap();
    let value = result(
        cmd(dir.path())
            .args([
                "--config",
                "missing.toml",
                "metadata",
                "validate",
                "provider",
                "provider.json",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(value["sha256"], fossa_sdk::metadata::sha256_hex(raw));
    assert_eq!(value["bytes"], raw.len());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}
#[test]
fn env_config_overrides_and_urls_remain_redacted() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"),format!("[solana]\nrpc_url='https://example.com/?api-key=never-print-this'\n[provider]\nwallet='{}'\n",key(5))).unwrap();
    let value = result(
        cmd(dir.path())
            .env("FOSSA_CONFIG", "config.toml")
            .env("FOSSA_WALLET", key(6).to_string())
            .args(["config", "show"])
            .output()
            .unwrap(),
    );
    assert_eq!(value["wallet"], key(6).to_string());
    assert!(!value.to_string().contains("never-print-this"));
    assert!(!dir.path().join("transactions").exists());
}
#[test]
fn invalid_metadata_and_prices_fail_before_rpc_or_key_loading() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("bad.json"), "{}").unwrap();
    let output = cmd(dir.path())
        .args([
            "--rpc-url",
            "http://127.0.0.1:1",
            "--keypair",
            "missing.json",
            "--execute",
            "provider",
            "register",
            "--metadata-file",
            "bad.json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let value: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(
        value["error"]["message"]
            .as_str()
            .unwrap()
            .contains("metadata")
    );
    assert!(!dir.path().join("transactions").exists());
    let output = cmd(dir.path())
        .args(["offer", "update", "--model", "m", "--input-price", "1.1e3"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(serde_json::from_slice::<Value>(&output.stderr).is_ok());
}
#[test]
fn provider_registration_default_plan_never_uploads_signs_or_writes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("provider.json"),
        r#"{"name":"Example","endpoint":"https://example.com/infer"}"#,
    )
    .unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let seen = calls.clone();
    let mock = mock(move |_, req| {
        let method = req["method"].as_str().unwrap();
        seen.lock().unwrap().push(method.to_owned());
        assert_eq!(method, "getAccountInfo");
        rpc(
            req,
            json!({"context":{"slot":9},"value":if req["params"][0]==key(2).to_string(){market_account(key(4))}else{Value::Null}}),
        )
    });
    let mut command = cmd(dir.path());
    protocol_args(&mut command, &mock.url, key(5));
    let value = result(
        command
            .args([
                "--keypair",
                "missing.json",
                "provider",
                "register",
                "--metadata-file",
                "provider.json",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(value["mode"], "plan");
    assert_eq!(value["instructions"][0]["opcode"], 19);
    assert_eq!(calls.lock().unwrap().len(), 2);
    assert!(!dir.path().join("transactions").exists());
}
#[test]
fn fresh_execution_simulates_saves_before_broadcast_and_archives_confirmed_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let signer = Keypair::new();
    let wallet = signer.pubkey();
    std::fs::write(
        dir.path().join("key.json"),
        serde_json::to_vec(&signer.to_bytes().to_vec()).unwrap(),
    )
    .unwrap();
    let pending_path = dir.path().join("transactions/pending.json");
    let journal = pending_path.clone();
    let wire = Arc::new(Mutex::new(None::<String>));
    let seen_wire = wire.clone();
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = calls.clone();
    let mock = mock(move |_, req| {
        let method = req["method"].as_str().unwrap();
        seen.lock().unwrap().push(method.into());
        let value = match method {
            "getAccountInfo" => json!({"context":{"slot":9},"value":market_account(key(4))}),
            "getMinimumBalanceForRentExemption" => json!(1000),
            "getLatestBlockhash" => {
                json!({"context":{"slot":9},"value":{"blockhash":Hash::new_from_array([8;32]).to_string(),"lastValidBlockHeight":100}})
            }
            "simulateTransaction" => {
                assert_eq!(req["params"][1]["sigVerify"], true);
                assert!(!journal.exists());
                *seen_wire.lock().unwrap() = Some(req["params"][0].as_str().unwrap().into());
                json!({"context":{"slot":9},"value":{"err":null,"logs":[],"unitsConsumed":100}})
            }
            "sendTransaction" => {
                let pending = PendingTransaction::load(&journal).unwrap();
                assert_eq!(pending.wire_base64, req["params"][0]);
                assert_eq!(
                    Some(pending.wire_base64.clone()),
                    *seen_wire.lock().unwrap()
                );
                json!(pending.signature)
            }
            "getSignatureStatuses" => {
                json!({"context":{"slot":10},"value":[{"slot":10,"confirmations":1,"err":null,"confirmationStatus":"confirmed"}]})
            }
            _ => panic!("unexpected {method}"),
        };
        rpc(req, value)
    });
    let mut command = cmd(dir.path());
    protocol_args(&mut command, &mock.url, wallet);
    let output = command
        .args(["--keypair", "key.json", "--execute", "user", "init"])
        .output()
        .unwrap();
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains(wire.lock().unwrap().as_ref().unwrap())
    );
    let value = result(output);
    let archive = value["journal"].as_str().unwrap();
    assert_eq!(value["mode"], "executed");
    assert!(!pending_path.exists());
    assert!(Path::new(archive).exists());
    assert_eq!(
        PendingTransaction::load(archive).unwrap().wire_base64,
        wire.lock().unwrap().as_ref().unwrap().to_owned()
    );
    assert!(!calls.lock().unwrap().iter().any(|s| s == "getBlockHeight"));
}
#[test]
fn resume_reconciles_before_send_preserves_uncertain_signature_and_never_replaces() {
    let dir = tempfile::tempdir().unwrap();
    let pending = signed();
    let path = dir.path().join("pending.json");
    pending.save(&path).unwrap();
    let original = std::fs::read(&path).unwrap();
    let expected = pending.wire_base64.clone();
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = calls.clone();
    let mock = mock(move |_, req| {
        let method = req["method"].as_str().unwrap();
        seen.lock().unwrap().push(method.into());
        match method {
            "getSignatureStatuses" => rpc(req, json!({"context":{"slot":9},"value":[null]})),
            "getBlockHeight" => rpc(req, json!(99)),
            "sendTransaction" => {
                assert_eq!(req["params"][0], expected);
                json!({"jsonrpc":"2.0","id":req["id"],"error":{"code":-32000,"message":"unknown delivery"}})
            }
            _ => panic!("unexpected {method}"),
        }
    });
    let output = cmd(dir.path())
        .args([
            "--rpc-url",
            &mock.url,
            "transaction",
            "resume",
            "--file",
            "pending.json",
            "--execute",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(String::from_utf8_lossy(&output.stderr).contains(&pending.signature));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(&pending.wire_base64));
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["getSignatureStatuses", "getBlockHeight", "sendTransaction"]
    );
}
#[test]
fn expired_resume_checks_status_twice_and_keeps_journal_without_broadcast() {
    let dir = tempfile::tempdir().unwrap();
    let pending = signed();
    let path = dir.path().join("pending.json");
    pending.save(&path).unwrap();
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = calls.clone();
    let mock = mock(move |_, req| {
        let method = req["method"].as_str().unwrap();
        seen.lock().unwrap().push(method.into());
        rpc(
            req,
            match method {
                "getSignatureStatuses" => json!({"context":{"slot":9},"value":[null]}),
                "getBlockHeight" => json!(101),
                _ => panic!("unexpected {method}"),
            },
        )
    });
    let output = cmd(dir.path())
        .args([
            "--rpc-url",
            &mock.url,
            "transaction",
            "resume",
            "--file",
            "pending.json",
            "--execute",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(path.exists());
    assert_eq!(
        PendingTransaction::load(path).unwrap().signature,
        pending.signature
    );
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            "getSignatureStatuses",
            "getBlockHeight",
            "getSignatureStatuses"
        ]
    );
}
#[test]
fn confirmed_resume_archives_without_rebroadcast() {
    let dir = tempfile::tempdir().unwrap();
    let pending = signed();
    pending.save(dir.path().join("pending.json")).unwrap();
    let mock = mock(|_, req| {
        assert_eq!(req["method"], "getSignatureStatuses");
        rpc(
            req,
            json!({"context":{"slot":10},"value":[{"slot":10,"confirmations":1,"err":null,"confirmationStatus":"confirmed"}]}),
        )
    });
    let value = result(
        cmd(dir.path())
            .args([
                "--rpc-url",
                &mock.url,
                "transaction",
                "resume",
                "--file",
                "pending.json",
                "--execute",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(value["signature"], pending.signature);
    assert!(!dir.path().join("pending.json").exists());
}

#[test]
fn help_never_displays_environment_values_and_parse_errors_name_safe_flags() {
    let dir = tempfile::tempdir().unwrap();
    for args in [
        vec!["--help"],
        vec!["provider", "update", "--help"],
        vec!["transaction", "resume", "--help"],
    ] {
        let output = cmd(dir.path())
            .env(
                "FOSSA_RPC_URL",
                "https://example.com/?api-key=rpc-help-secret",
            )
            .env("FOSSA_API_URL", "https://api-help-secret@example.com")
            .env("FOSSA_KEYPAIR", "/private/path-help-secret.json")
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(!text.contains("help-secret"));
        assert!(text.contains("https://fossa.ag/docs/providers/cli.md"));
    }
    let output = cmd(dir.path())
        .args([
            "offer",
            "update",
            "--model",
            "m",
            "--input-price",
            "not-a-price",
        ])
        .output()
        .unwrap();
    let error: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert!(
        error["error"]["argument"]
            .as_str()
            .unwrap()
            .contains("--input-price")
    );
}
#[test]
fn offer_update_plans_keep_all_omitted_chain_terms_and_parse_real_rpc_strings() {
    let dir = tempfile::tempdir().unwrap();
    let wallet = key(5);
    let (model, bump) = fossa_sdk::address::model(
        &key(1),
        &key(2),
        &fossa_sdk::address::model_id("example/model"),
    );
    let terms_key = key(6);
    let capacity_key = key(7);
    let raw = |bytes: Vec<u8>| json!({"owner":key(1).to_string(),"lamports":1,"executable":false,"data":[STANDARD.encode(bytes),"base64"]});
    let mut m = vec![0; 144];
    m[..8].copy_from_slice(b"MODELV20");
    m[8..40].copy_from_slice(key(2).as_ref());
    m[40..72].copy_from_slice(&fossa_sdk::address::model_id("example/model"));
    m[72..104].copy_from_slice(terms_key.as_ref());
    m[104..136].copy_from_slice(capacity_key.as_ref());
    m[136..140].copy_from_slice(&1u32.to_le_bytes());
    m[140] = bump;
    let mut t = vec![0; 168];
    t[..8].copy_from_slice(b"TERMSV20");
    t[8..40].copy_from_slice(model.as_ref());
    t[40..44].copy_from_slice(&1u32.to_le_bytes());
    t[44..48].copy_from_slice(&1u32.to_le_bytes());
    t[48..80].copy_from_slice(wallet.as_ref());
    t[80..112].copy_from_slice(key(8).as_ref());
    t[112..144].copy_from_slice(key(9).as_ref());
    t[144..148].copy_from_slice(&1_000_000u32.to_le_bytes());
    t[148..152].copy_from_slice(&500_000u32.to_le_bytes());
    t[152..156].copy_from_slice(&2_000_000u32.to_le_bytes());
    t[156..158].copy_from_slice(&300u16.to_le_bytes());
    t[158] = 1;
    t[159] = 0;
    t[160] = 1;
    t[161] = fossa_sdk::address::bond(&key(1), &model, 0).1;
    t[164..168].copy_from_slice(&7u32.to_le_bytes());
    let mut c = vec![0; 80];
    c[..8].copy_from_slice(b"CAPACV20");
    c[8..40].copy_from_slice(model.as_ref());
    c[40..44].copy_from_slice(&1u32.to_le_bytes());
    c[44..48].copy_from_slice(&1u32.to_le_bytes());
    let (m, t, c) = (raw(m), raw(t), raw(c));
    let mock = mock(move |_, req| {
        rpc(
            req,
            match req["method"].as_str().unwrap() {
                "getAccountInfo" => {
                    json!({"context":{"slot":9},"value":if req["params"][0]==key(2).to_string(){market_account(key(4))}else{m.clone()}})
                }
                "getMultipleAccounts" => json!({"context":{"slot":9},"value":[t,c]}),
                method => panic!("unexpected {method}"),
            },
        )
    });
    let mut command = cmd(dir.path());
    protocol_args(&mut command, &mock.url, wallet);
    let value = result(
        command
            .args([
                "offer",
                "update",
                "--model",
                "example/model",
                "--output-price",
                "3.123456",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(
        value["details"]["next"],
        json!({"rates":{"input":1_000_000,"cached":500_000,"output":3_123_456},"speed_tps":300,"private":true,"active":false,"capacity":7})
    );
    assert_eq!(value["instructions"][0]["opcode"], 5);
    assert!(!dir.path().join("transactions").exists());
}

#[test]
fn explicit_rpc_choice_overrides_other_transport_environment() {
    let dir = tempfile::tempdir().unwrap();
    let value = result(
        cmd(dir.path())
            .env("FOSSA_RPC_URL", "https://rpc-secret.example")
            .args(["--hosted-rpc", "config", "show"])
            .output()
            .unwrap(),
    );
    assert_eq!(value["rpc"], "hosted");
    let value = result(
        cmd(dir.path())
            .env("FOSSA_HOSTED_RPC", "true")
            .args(["--rpc-url", "https://own.example", "config", "show"])
            .output()
            .unwrap(),
    );
    assert_eq!(value["rpc"], "custom (URL redacted)");
}
#[test]
fn invalid_api_origin_never_leaks_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let output = cmd(dir.path())
        .args([
            "--api-url",
            "https://origin-secret@example.com/?token=origin-secret",
            "config",
            "show",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stderr).contains("origin-secret"));
    assert!(output.stdout.is_empty());
}

#[test]
fn simulation_failure_never_creates_journal_or_broadcasts() {
    let dir = tempfile::tempdir().unwrap();
    let signer = Keypair::new();
    let wallet = signer.pubkey();
    std::fs::write(
        dir.path().join("key.json"),
        serde_json::to_vec(&signer.to_bytes().to_vec()).unwrap(),
    )
    .unwrap();
    let mock = mock(|_, req| {
        rpc(
            req,
            match req["method"].as_str().unwrap() {
                "getAccountInfo" => json!({"context":{"slot":9},"value":market_account(key(4))}),
                "getMinimumBalanceForRentExemption" => json!(1000),
                "getLatestBlockhash" => {
                    json!({"context":{"slot":9},"value":{"blockhash":Hash::new_from_array([8;32]).to_string(),"lastValidBlockHeight":100}})
                }
                "simulateTransaction" => {
                    json!({"context":{"slot":9},"value":{"err":{"InstructionError":[0,"Custom"]},"logs":[],"unitsConsumed":100}})
                }
                method => panic!("unexpected {method}"),
            },
        )
    });
    let mut command = cmd(dir.path());
    protocol_args(&mut command, &mock.url, wallet);
    let output = command
        .args(["--keypair", "key.json", "--execute", "user", "init"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!dir.path().join("transactions").exists());
}
#[test]
fn onboarding_approval_is_explicit_and_blocked_execution_cannot_upload() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("provider.json"),
        r#"{"name":"Example","endpoint":"https://example.com/infer"}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("config.toml"),
        "[provider]\nmetadata_file='provider.json'\n",
    )
    .unwrap();
    let mock = mock(|_, req| {
        assert_eq!(req["method"], "getAccountInfo");
        rpc(
            req,
            json!({"context":{"slot":9},"value":if req["params"][0]==key(2).to_string(){market_account(key(5))}else{Value::Null}}),
        )
    });
    let mut command = cmd(dir.path());
    protocol_args(&mut command, &mock.url, key(5));
    let value = result(
        command
            .args(["--config", "config.toml", "onboard"])
            .output()
            .unwrap(),
    );
    assert!(
        value["blockers"][0]
            .as_str()
            .unwrap()
            .contains("--allow-approval")
    );
    let mut command = cmd(dir.path());
    protocol_args(&mut command, &mock.url, key(5));
    let output = command
        .args(["--config", "config.toml", "onboard", "--execute"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!dir.path().join("transactions").exists());
    let mut command = cmd(dir.path());
    protocol_args(&mut command, &mock.url, key(5));
    let value = result(
        command
            .args(["--config", "config.toml", "onboard", "--allow-approval"])
            .output()
            .unwrap(),
    );
    assert_eq!(value["blockers"], json!([]));
    assert!(
        value["steps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["instructions"][0]["opcode"] == 20)
    );
}
#[test]
fn metadata_upload_is_exact_immutable_and_independent_of_rpc() {
    let dir = tempfile::tempdir().unwrap();
    let signer = Keypair::new();
    std::fs::write(
        dir.path().join("key.json"),
        serde_json::to_vec(&signer.to_bytes().to_vec()).unwrap(),
    )
    .unwrap();
    let bytes = b"{ \"name\":\"Example\",\"endpoint\":\"https://example.com/infer\" }\n";
    std::fs::write(dir.path().join("provider.json"), bytes).unwrap();
    let sha = fossa_sdk::metadata::sha256_hex(bytes);
    let base = Arc::new(Mutex::new(String::new()));
    let url = base.clone();
    let hash = sha.clone();
    let mock = mock(move |path, req| {
        assert_eq!(path, "/v1/providers/metadata/provider");
        assert_eq!(req["name"], "Example");
        json!({"sha256":hash,"uri":format!("{}/v1/market/metadata/{hash}",url.lock().unwrap()),"bytes":bytes.len()})
    });
    *base.lock().unwrap() = mock.url.clone();
    let value = result(
        cmd(dir.path())
            .args([
                "--api-url",
                &mock.url,
                "--keypair",
                "key.json",
                "metadata",
                "upload",
                "provider",
                "provider.json",
                "--execute",
            ])
            .output()
            .unwrap(),
    );
    assert_eq!(value["upload"]["sha256"], sha);
    assert_eq!(value["upload"]["bytes"], bytes.len());
    assert!(!dir.path().join("transactions").exists());
}
