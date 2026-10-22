use base64::{Engine, engine::general_purpose::STANDARD};
use fossa_sdk::{
    accounts::*,
    hosted::*,
    inference,
    metadata::{Document, Kind},
    onboarding,
    transaction::PreparedTransaction,
    *,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn key(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
fn signer() -> Keypair {
    Keypair::new_from_array([7; 32])
}
fn protocol() -> Protocol {
    Protocol {
        program: key(1),
        market: key(2),
        mint: key(3),
    }
}
fn pending() -> PendingTransaction {
    let signer = signer();
    let ix = protocol().approve_provider(key(5), signer.pubkey(), true);
    PreparedTransaction::new(
        &[ix],
        signer.pubkey(),
        Hash::new_from_array([8; 32]),
        100,
        9,
    )
    .unwrap()
    .sign(&[&signer])
    .unwrap()
}
struct Mock {
    url: String,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Mock {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn spawn_mock(
    handler: impl Fn(&str, &HashMap<String, String>, &[u8]) -> (u16, Value) + Send + Sync + 'static,
) -> Mock {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handler = Arc::new(handler);
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            let handler = handler.clone();
            tokio::spawn(async move {
                let mut bytes = vec![];
                let (header_end, length, head) = loop {
                    let mut buf = [0; 4096];
                    let n = stream.read(&mut buf).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    bytes.extend(&buf[..n]);
                    if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                        let head = String::from_utf8(bytes[..end].to_vec()).unwrap();
                        let length = head
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|s| s.trim().parse::<usize>().unwrap())
                            })
                            .unwrap_or(0);
                        break (end + 4, length, head);
                    }
                };
                while bytes.len() < header_end + length {
                    let mut buf = [0; 4096];
                    let n = stream.read(&mut buf).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    bytes.extend(&buf[..n]);
                }
                let path = head
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();
                let headers = head
                    .lines()
                    .skip(1)
                    .filter_map(|line| {
                        line.split_once(':')
                            .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_owned()))
                    })
                    .collect();
                let (status, result) =
                    handler(path, &headers, &bytes[header_end..header_end + length]);
                let body = serde_json::to_vec(&result).unwrap();
                let response = format!(
                    "HTTP/1.1 {status} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(response.as_bytes()).await.unwrap();
                stream.write_all(&body).await.unwrap();
            });
        }
    });
    Mock { url, task }
}
fn rpc_response(body: &[u8], result: Value) -> (u16, Value) {
    let request: Value = serde_json::from_slice(body).unwrap();
    (
        200,
        json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
    )
}

#[test]
fn offline_signing_persistence_rejects_tampering_and_never_overwrites_journal() {
    let pending = pending();
    let tx = pending.transaction().unwrap();
    tx.verify().unwrap();
    assert_eq!(tx.signatures[0].to_string(), pending.signature);
    assert!(!format!("{pending:?}").contains(&pending.wire_base64));
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("pending.json");
    pending.save(&path).unwrap();
    assert!(pending.save(&path).is_err());
    let loaded = PendingTransaction::load(&path).unwrap();
    assert_eq!(loaded.wire_base64, pending.wire_base64);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    let mut bad = pending.clone();
    bad.signature = Signature::default().to_string();
    assert!(bad.validate().is_err());
    let mut bytes = STANDARD.decode(&pending.wire_base64).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    bad = pending.clone();
    bad.wire_base64 = STANDARD.encode(bytes);
    assert!(bad.validate().is_err());
    let signer = signer();
    let unsigned = PreparedTransaction::new(
        &[protocol().approve_provider(key(5), key(9), true)],
        signer.pubkey(),
        Hash::new_from_array([8; 32]),
        100,
        9,
    )
    .unwrap();
    assert!(unsigned.sign(&[&signer]).is_err());
    let ix = protocol().approve_provider(key(5), signer.pubkey(), true);
    let prepared = PreparedTransaction::new(
        &[ix],
        signer.pubkey(),
        Hash::new_from_array([8; 32]),
        100,
        9,
    )
    .unwrap();
    let signature = signer.sign_message(&prepared.message_bytes());
    assert!(
        prepared
            .attach_signatures(&[(signer.pubkey(), signature)])
            .is_ok()
    );
}

#[test]
fn transaction_builders_reject_packet_and_signer_overflow_without_panicking() {
    let signer = signer();
    let oversized = Instruction {
        program_id: key(1),
        accounts: vec![],
        data: vec![0; 1232],
    };
    assert!(
        PreparedTransaction::new(&[oversized], signer.pubkey(), Hash::default(), 1, 1).is_err()
    );
    let mut accounts = vec![];
    for n in 0..256u16 {
        let mut bytes = [0; 32];
        bytes[..2].copy_from_slice(&n.to_le_bytes());
        accounts.push(AccountMeta::new(Pubkey::new_from_array(bytes), true));
    }
    let ix = Instruction {
        program_id: accounts[0].pubkey,
        accounts,
        data: vec![],
    };
    assert!(
        PreparedTransaction::new(
            std::slice::from_ref(&ix),
            ix.accounts[0].pubkey,
            Hash::default(),
            1,
            1
        )
        .is_err()
    );
}

#[tokio::test]
async fn rpc_prepare_simulate_broadcast_and_resume_use_same_signed_bytes() {
    let calls = Arc::new(Mutex::new(vec![]));
    let seen = calls.clone();
    let sent = Arc::new(Mutex::new(None::<String>));
    let record = sent.clone();
    let mock=spawn_mock(move |_,_,body| {let request:Value=serde_json::from_slice(body).unwrap();let method=request["method"].as_str().unwrap();seen.lock().unwrap().push(method.to_owned());let result=match method {
        "getLatestBlockhash"=>json!({"context":{"slot":9},"value":{"blockhash":Hash::new_from_array([8;32]).to_string(),"lastValidBlockHeight":100}}),
        "simulateTransaction"=>{assert_eq!(request["params"][1]["sigVerify"],true);assert_eq!(request["params"][1]["minContextSlot"],9);json!({"context":{"slot":9},"value":{"err":null,"logs":[],"unitsConsumed":500}})},
        "sendTransaction"=>{let wire=request["params"][0].as_str().unwrap().to_owned();*record.lock().unwrap()=Some(wire.clone());let tx:Transaction=bincode::deserialize(&STANDARD.decode(wire).unwrap()).unwrap();tx.verify().unwrap();json!(tx.signatures[0].to_string())},
        "getSignatureStatuses"=>json!({"context":{"slot":10},"value":[{"slot":10,"confirmations":1,"err":null,"confirmationStatus":"confirmed"}]}),
        _=>panic!("unexpected method {method}")};rpc_response(body,result)}).await;
    let rpc = RpcClient::new(&mock.url).unwrap();
    let signer = signer();
    let prepared = rpc
        .prepare(
            &[protocol().approve_provider(key(5), signer.pubkey(), true)],
            signer.pubkey(),
        )
        .await
        .unwrap();
    let pending = prepared.sign(&[&signer]).unwrap();
    rpc.simulate(&pending).await.unwrap().check().unwrap();
    rpc.broadcast(&pending).await.unwrap();
    rpc.confirm(&pending, Duration::from_secs(1)).await.unwrap();
    rpc.submit(&pending).await.unwrap();
    assert_eq!(*sent.lock().unwrap(), Some(pending.wire_base64));
    assert_eq!(
        calls
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.as_str() == "sendTransaction")
            .count(),
        1
    );
    assert!(!calls.lock().unwrap().iter().any(|s| s == "getBlockHeight"));
}

#[tokio::test]
async fn expiry_rechecks_history_after_finalized_height() {
    let count = Arc::new(AtomicUsize::new(0));
    let n = count.clone();
    let mock=spawn_mock(move |_,_,body|{let q:Value=serde_json::from_slice(body).unwrap();let result=match q["method"].as_str().unwrap(){"getSignatureStatuses"=>json!({"context":{"slot":9},"value":[if n.fetch_add(1,Ordering::SeqCst)==0{Value::Null}else{json!({"slot":10,"confirmations":null,"err":null,"confirmationStatus":"finalized"})}]}),"getBlockHeight"=>{assert_eq!(q["params"][0]["commitment"],"finalized");json!(101)},m=>panic!("unexpected {m}")};rpc_response(body,result)}).await;
    RpcClient::new(&mock.url)
        .unwrap()
        .confirm(&pending(), Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 2);
    let mock = spawn_mock(|_, _, body| {
        let q: Value = serde_json::from_slice(body).unwrap();
        rpc_response(
            body,
            if q["method"] == "getBlockHeight" {
                json!(101)
            } else {
                json!({"context":{"slot":9},"value":[null]})
            },
        )
    })
    .await;
    assert!(matches!(
        RpcClient::new(&mock.url).unwrap().submit(&pending()).await,
        Err(Error::Expired)
    ));
}

#[tokio::test]
async fn rpc_rejects_wrong_envelopes_failed_transactions_and_bad_account_encoding() {
    let mock = spawn_mock(|_, _, _| (200, json!({"jsonrpc":"2.0","id":999,"result":1}))).await;
    assert!(
        RpcClient::new(&mock.url)
            .unwrap()
            .balance(key(1))
            .await
            .is_err()
    );
    let mock=spawn_mock(|_,_,body|rpc_response(body,json!({"context":{"slot":1},"value":{"owner":key(1).to_string(),"lamports":1,"executable":false,"data":["abc","base58"]}}))).await;
    assert!(
        RpcClient::new(&mock.url)
            .unwrap()
            .raw_account(key(1))
            .await
            .is_err()
    );
    let mock=spawn_mock(|_,_,body|rpc_response(body,json!({"context":{"slot":1},"value":[{"slot":1,"confirmations":1,"err":{"InstructionError":[0,{"Custom":4}]},"confirmationStatus":"confirmed"}]}))).await;
    assert!(matches!(
        RpcClient::new(&mock.url).unwrap().submit(&pending()).await,
        Err(Error::Transaction(_))
    ));
    let mock=spawn_mock(|_,_,body|{let q:Value=serde_json::from_slice(body).unwrap();(200,json!({"jsonrpc":"2.0","id":q["id"],"error":{"code":-32000,"message":"fixture","data":{"retry":true}}}))}).await;
    assert!(matches!(
        RpcClient::new(&mock.url).unwrap().balance(key(1)).await,
        Err(Error::Rpc { code: -32000, .. })
    ));
}

#[test]
fn hosted_signature_golden_vector_matches_backend() {
    let message = signing_message(
        "https://api.example.test/",
        RPC_PATH,
        &Pubkey::default(),
        2_000_000_000,
        "0123456789abcdef0123456789abcdef",
        br#"{"a":1}"#,
    )
    .unwrap();
    assert_eq!(
        message,
        "fossa-provider-v1\nhttps://api.example.test\nPOST\n/v1/providers/rpc\n11111111111111111111111111111111\n2000000000\n0123456789abcdef0123456789abcdef\n015abd7f5cc57a2dd94b7590f04ad8084273905ee33ec5cebeae62276a97f862"
    );
    assert!(HostedClient::new("http://api.example.test", Arc::new(signer())).is_err());
    assert!(HostedClient::new("https://user:secret@api.example.test", Arc::new(signer())).is_err());
}

#[tokio::test]
async fn hosted_upload_and_rpc_sign_exact_bytes_and_fresh_nonces() {
    let nonces = Arc::new(Mutex::new(std::collections::HashSet::new()));
    let seen = nonces.clone();
    let mock=spawn_mock(move |path,headers,body|{let api=format!("http://{}",headers["host"]);let wallet:Pubkey=headers["x-fossa-wallet"].parse().unwrap();let expires=headers["x-fossa-expires"].parse().unwrap();let nonce=&headers["x-fossa-nonce"];assert!(seen.lock().unwrap().insert(nonce.clone()));let message=signing_message(&api,path,&wallet,expires,nonce,body).unwrap();let signature:Signature=headers["x-fossa-signature"].parse().unwrap();assert!(signature.verify(wallet.as_ref(),message.as_bytes()));
        if path==RPC_PATH {rpc_response(body,json!({"context":{"slot":1},"value":42}))}else {assert_eq!(body,b"{ \"name\": \"Example\", \"endpoint\": \"https://example.com/infer\" }\n");let hash=metadata::sha256_hex(body);(200,json!({"sha256":hash,"uri":format!("{api}/v1/market/metadata/{hash}"),"bytes":body.len()}))}
    }).await;
    let hosted = HostedClient::new(&mock.url, Arc::new(signer())).unwrap();
    let doc = Document::parse(
        Kind::Provider,
        b"{ \"name\": \"Example\", \"endpoint\": \"https://example.com/infer\" }\n".to_vec(),
    )
    .unwrap();
    let upload = hosted.upload_metadata(&doc).await.unwrap();
    doc.verify(&upload.reference().unwrap()).unwrap();
    assert_eq!(RpcClient::hosted(hosted).balance(key(1)).await.unwrap(), 42);
    assert_eq!(nonces.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn onboarding_plans_registration_then_requires_real_admin_approval() {
    let p = protocol();
    let admin = key(20);
    let mut data = vec![0; 72];
    data[..8].copy_from_slice(b"FOSSAV20");
    data[8..40].copy_from_slice(admin.as_ref());
    data[40..72].copy_from_slice(p.mint.as_ref());
    let mock=spawn_mock(move |_,_,body|{let q:Value=serde_json::from_slice(body).unwrap();assert_eq!(q["method"],"getAccountInfo");rpc_response(body,json!({"context":{"slot":1},"value":if q["params"][0]==p.market.to_string(){json!({"owner":p.program.to_string(),"lamports":1,"executable":false,"data":[STANDARD.encode(&data),"base64"]})}else{Value::Null}}))}).await;
    let config = onboarding::OnboardConfig {
        provider: key(10),
        payer: key(11),
        admin: None,
        document: DocumentRef::new([1; 32], "https://example.com/provider.json").unwrap(),
        book_limit: 4,
        models: vec![],
    };
    let plan = onboarding::plan(&RpcClient::new(&mock.url).unwrap(), &p, &config)
        .await
        .unwrap();
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].instructions[0].data[0], 19);
    assert_eq!(plan.blockers.len(), 1);
}

#[test]
fn inference_v1_v2_headers_verify_exact_body_owner_booking_and_deadline() {
    let p = protocol();
    let signer = signer();
    let address = key(8);
    let mut raw = RawAccount {
        owner: p.program,
        lamports: 1,
        executable: false,
        data: vec![0; 368],
    };
    raw.data[..8].copy_from_slice(b"BOOK0V20");
    raw.data[8..40].copy_from_slice(signer.pubkey().as_ref());
    raw.data[40..72].copy_from_slice(key(9).as_ref());
    raw.data[72..104].copy_from_slice(p.market.as_ref());
    raw.data[132..140].copy_from_slice(&1000i64.to_le_bytes());
    let body = br#"{"model":"example","messages":[]}"#;
    for generation in [0u64, 1, 72_057_594_037_927_935] {
        raw.data[152..160].copy_from_slice(&generation.to_le_bytes());
        let state = Booking::decode(&address, &raw, &p.program).unwrap();
        let auth = inference::Authorization::sign(&p, address, &state, body, &signer, 500).unwrap();
        let decoded = inference::Authorization::from_headers(&auth.headers().unwrap()).unwrap();
        assert_eq!(decoded, auth);
        decoded.verify(&p, address, &state, body, 500).unwrap();
        assert!(decoded.verify(&p, address, &state, b"wrong", 500).is_err());
        assert!(decoded.verify(&p, key(7), &state, body, 500).is_err());
        assert!(decoded.verify(&p, address, &state, body, 1001).is_err());
        assert!(
            auth.request(
                "https://provider.example/v1/chat/completions",
                body.to_vec()
            )
            .is_ok()
        );
        assert!(
            auth.request("http://provider.example/v1/chat/completions", body.to_vec())
                .is_err()
        );
    }
}

#[tokio::test]
async fn uncertain_status_preserves_pending_bytes_and_reports_timeout() {
    let mock = spawn_mock(|_, _, body| {
        let request: Value = serde_json::from_slice(body).unwrap();
        let result = if request["method"] == "getBlockHeight" {
            json!(99)
        } else {
            assert_eq!(request["method"], "getSignatureStatuses");
            json!({"context":{"slot":1},"value":[null]})
        };
        rpc_response(body, result)
    })
    .await;
    let pending = pending();
    let before = pending.wire_base64.clone();
    assert!(matches!(
        RpcClient::new(&mock.url)
            .unwrap()
            .confirm(&pending, Duration::ZERO)
            .await,
        Err(Error::Timeout)
    ));
    assert_eq!(pending.wire_base64, before);
    let mock=spawn_mock(|_,_,body|rpc_response(body,json!({"context":{"slot":1},"value":[{"slot":1,"confirmations":0,"err":{"InstructionError":[0,{"Custom":4}]},"confirmationStatus":"processed"}]}))).await;
    assert!(matches!(
        RpcClient::new(&mock.url)
            .unwrap()
            .confirm(&pending, Duration::ZERO)
            .await,
        Err(Error::Timeout)
    ));
}

#[test]
fn keypair_reader_validates_full_key_and_uploaded_hash_never_panics() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("keypair.json");
    let original = signer();
    std::fs::write(
        &path,
        serde_json::to_vec(&original.to_bytes().to_vec()).unwrap(),
    )
    .unwrap();
    assert_eq!(
        transaction::read_keypair(&path).unwrap().pubkey(),
        original.pubkey()
    );
    let mut corrupted = original.to_bytes();
    corrupted[63] ^= 1;
    std::fs::write(&path, serde_json::to_vec(&corrupted.to_vec()).unwrap()).unwrap();
    assert!(transaction::read_keypair(&path).is_err());
    for sha256 in ["é".repeat(32), "g".repeat(64), "a".repeat(63)] {
        assert!(
            Upload {
                sha256,
                uri: "https://example.com/a".into(),
                bytes: 1
            }
            .reference()
            .is_err()
        );
    }
}

#[tokio::test]
async fn metadata_fetch_rejects_private_destinations_and_non_https_before_http() {
    for uri in [
        "http://example.com/a.json",
        "https://127.0.0.1/a.json",
        "https://[::1]/a.json",
        "https://user:secret@example.com/a.json",
        "file:///etc/passwd",
    ] {
        let reference = DocumentRef::new([0; 32], uri).unwrap();
        assert!(
            Document::fetch(Kind::Provider, &reference).await.is_err(),
            "{uri}"
        );
    }
}

#[tokio::test]
async fn real_json_wire_uses_base58_keys_for_config_accounts_and_discovery() {
    let p = protocol();
    let owner = key(20);
    let mut data = vec![0; 72];
    data[..8].copy_from_slice(b"FOSSAV20");
    data[8..40].copy_from_slice(owner.as_ref());
    data[40..72].copy_from_slice(p.mint.as_ref());
    let config:PublicConfig=serde_json::from_value(json!({"programId":p.program.to_string(),"market":p.market.to_string(),"usdcMint":p.mint.to_string(),"providerApi":"https://api.example.test","providerRpc":"https://api.example.test/v1/providers/rpc"})).unwrap();
    assert_eq!(config.protocol(), p);
    assert_eq!(
        serde_json::to_value(p).unwrap()["program"],
        p.program.to_string()
    );
    let mock=spawn_mock(move |_,_,body| {let q:Value=serde_json::from_slice(body).unwrap();let account=json!({"owner":p.program.to_string(),"lamports":1,"executable":false,"data":[STANDARD.encode(&data),"base64"]});rpc_response(body,if q["method"]=="getProgramAccounts" {assert_eq!(q["params"][1]["encoding"],"base64");json!([{"pubkey":p.market.to_string(),"account":account}])}else{json!({"context":{"slot":1},"value":account})})}).await;
    let rpc = RpcClient::new(&mock.url).unwrap();
    let market = rpc.validate_market(&p).await.unwrap();
    assert_eq!(market.admin, owner);
    assert_eq!(
        serde_json::to_value(&market).unwrap()["admin"],
        owner.to_string()
    );
    assert_eq!(
        rpc.program_accounts::<Market>(p.program, vec![])
            .await
            .unwrap(),
        vec![(p.market, market)]
    );
}
