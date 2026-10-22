# provider cli

`fossa` plans mutations until `--execute`. no arguments opens help. terminals show colored help, readable errors and highlighted json; pipes retain compact json. `--json` forces structured output everywhere; `NO_COLOR` disables color.

[quick start](https://fossa.ag/docs/providers/quick-start.md) · [commands/config](https://fossa.ag/docs/providers/cli.md) · [sdk](https://fossa.ag/docs/providers/sdk.md)

```bash
curl -fsSL https://api.fossa.ag/v1/providers/install.sh | bash
fossa
fossa --json config show
```

or build from repository root with `cargo build --release -p fossa-provider --locked`; executable is `target/release/fossa`. copy [config.example.toml](config.example.toml) outside git. minimal config:

```toml
[operator]
keypair = "/absolute/path/provider-keypair.json"
```

hosted rpc defaults on without an own rpc url; reads sign request authentication. api defaults to `https://api.fossa.ag`; public protocol addresses are pinned locally. no public-config fetch. override addresses when needed; rpc validates market ownership and mint. `--rpc-url URL` selects own rpc, `--hosted-rpc=false` disables hosted rpc. flags override environment, then config. paths in config resolve beside config; flag/environment paths use working directory.

```bash
fossa metadata validate provider ./provider.json
fossa --config ./provider.toml provider register --metadata-file ./provider.json
fossa --config ./provider.toml provider update --name "Example Provider" --image-file ./logo.png --execute
fossa --config ./provider.toml offer update --model example/model --input-price 1.25 --output-price 2.50
fossa --config ./provider.toml transaction resume --file ./transactions/pending.json --execute
```

price flags use usdc per million tokens; config rates use micro-usdc per million. omitted offer fields remain unchanged. profile edits verify current metadata, preserve extensions and publish new immutable hashes. registration requires separate market-admin approval.

execution simulates signed bytes, saves private `transactions/pending.json`, then sends. confirmed journals archive by signature; uncertain/expired transactions stay pending. resume reconciles before resending identical bytes. no automatic replacement. reusable receipt publication uses sdk.

checks: `cargo +1.99.0 test -p fossa-provider --locked` and `cargo +1.99.0 clippy -p fossa-provider --all-targets --locked -- -D warnings`. tests use synthetic keys and local mock rpc.
