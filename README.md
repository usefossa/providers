# providers

rust sdk, noninteractive cli and metadata schema for fossa. repository stays private; packages are not published.

- [sdk/rust](sdk/rust/): protocol accounts, instructions, rpc and signing.
- [cli](cli/): provider management built on sdk.
- [schema](schema/): provider/model metadata contract and validation.

install cli:

```bash
curl -fsSL https://api.fossa.ag/v1/providers/install.sh | bash
fossa --help
```

check locally:

```bash
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

[cli docs](https://fossa.ag/docs/providers/cli.md) · [sdk docs](https://fossa.ag/docs/providers/sdk.md) · [metadata reference](https://fossa.ag/docs/technical/metadata.md)
