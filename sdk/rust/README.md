# rust sdk

`fossa-sdk` provides offline protocol constructors and optional async transport. maintained inside this private repository; no crates.io package.

```toml
[dependencies]
fossa-sdk = { path = "../providers/sdk/rust" }
```

modules cover typed accounts, pdas, exact token arithmetic, matching, all protocol instructions, receipt proofs, metadata, onboarding plans, direct inference authorization, pooled rpc, and transaction recovery. see [compiled example](examples/provider.rs).

```rust
use fossa_sdk::{Protocol, Pubkey, Result, Instruction};
use fossa_sdk::metadata::{Document, Kind};

fn update(protocol: Protocol, provider: Pubkey, bytes: Vec<u8>) -> Result<Instruction> {
    let document = Document::parse(Kind::Provider, bytes)?;
    let reference = document.reference("https://example.com/provider.json")?;
    protocol.update_provider_metadata(provider, &reference)
}
```

use `RpcClient::new(url)` for own rpc. alternatively create `HostedClient::new(api_url, signer)` and `RpcClient::hosted(client)` for wallet-signed hosted rpc. `PublicConfig::fetch(api_url)` discovers public program, market, mint and native endpoint fields. `HostedClient::upload_metadata` preserves exact validated bytes; `upload_image` accepts png, jpeg or webp. self-hosted metadata references remain supported. `Document::fetch` verifies exact hashes over public https with pinned dns and no redirects.

`onboarding::plan` reads current state and returns ordered instructions, newly generated book signers, and approval blockers. planning never submits. every account read validates owner, discriminator and layout; pda records also validate derived address. use `model_books` and `validate_market` for cross-account bindings.

transaction flow: `prepare` → `sign` → `simulate` → `PendingTransaction::save` → `broadcast` → `confirm`. save signed bytes before broadcast. `broadcast` sends freshly prepared bytes without preliminary rpc reads; `submit` reconciles a saved transaction before retrying identical bytes. expiry checks recheck history after finalized block height. timeout or transport failure leaves journal pending. never sign a replacement until previous signature outcome is resolved. journal files contain signed bytes, use owner-only permissions and never overwrite existing files.

user flow: initialize/fund lane → book → read matched booking → sign exact request bytes using `inference::Authorization` → send request directly to provider. authorization supports maintained v1/v2 statements and verifies owner, booking, generation, body hash and deadline. provider-signed on-chain claims carry authoritative settlement usage; streamed usage alone is not settlement proof. reusable claims publish canonical receipts; `receipt` constructs/verifies proofs and `Protocol::dispute_receipt` opens a case. provider runtimes must still validate request capabilities, admission and replay state.

```sh
cargo test --locked -p fossa-sdk
cargo clippy --locked -p fossa-sdk --all-targets -- -D warnings
```

tests use synthetic keys, deterministic account fixtures and loopback mock http only. no live transactions, provider processes or paid inference.
