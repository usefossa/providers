# provider cli

noninteractive rust binary; json stdout/errors, text help. mutations plan until `--execute`. hosted rpc signs request authentication during reads. own rpc and custom metadata hosting remain supported.

[quick start](https://fossa.ag/docs/providers/quick-start.md) · [commands/config](https://fossa.ag/docs/providers/cli.md) · [sdk](https://fossa.ag/docs/providers/sdk.md)

```bash
curl -fsSL https://api.fossa.ag/v1/providers/install.sh | bash
fossa-provider --help
```

or build from repository root with `cargo build --release -p fossa-provider --locked`. copy [config.example.toml](config.example.toml) outside git; keep keypairs and journals private. minimal config:

```toml
[solana]
rpc_url = "https://rpc.example.com"
[operator]
keypair = "/absolute/path/provider-keypair.json"
```

config paths resolve beside config; flag/environment paths use working directory. explicit flags override environment, then config. omit protocol addresses to read public configuration. optional hosted rpc replaces `solana.rpc_url` with `[hosted] rpc = true`.

```bash
fossa-provider metadata validate provider ./provider.json
fossa-provider --config ./provider.toml provider register --metadata-file ./provider.json
fossa-provider --config ./provider.toml provider update --name "Example Provider" --image-file ./logo.png --execute
fossa-provider --config ./provider.toml offer update --model example/model --input-price 1.25 --output-price 2.50
fossa-provider --config ./provider.toml transaction resume --file ./transactions/pending.json --execute
```

price flags use usdc per million tokens; config rates use micro-usdc per million. omitted offer fields remain unchanged. profile edits verify current metadata, preserve extensions and publish new immutable hashes. registration requires separate market-admin approval.

execution simulates signed bytes, saves `transactions/pending.json`, then sends. confirmed journals archive by signature; uncertain/expired transactions stay pending. resume reconciles before resending identical bytes. no automatic replacement. one-shot booking/user workflows available; reusable receipt publication uses sdk.

checks: `cargo test -p fossa-provider --locked` and `cargo clippy -p fossa-provider --all-targets --locked -- -D warnings`. tests use synthetic keys and local mock rpc.
