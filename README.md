# Fossa provider tooling

Provider onboarding CLI and shared provider/model metadata contract. This repository is private while the SDK and CLI are developed. Packages are not published.

- `cli/`: onboarding, metadata validation, configuration and token/account utilities.
- `metadata/`: canonical JSON Schema v1, shared Rust validator, examples, conformance cases and synchronization utility.

Install and check locally:

```sh
bun install --cwd cli --frozen-lockfile
bun run check
bun run test
```

Validate a document offline:

```sh
bun run validate:metadata provider /absolute/path/provider.json
bun run validate:metadata model /absolute/path/model.json
```

Prepare onboarding using an explicit configuration based on `cli/config.example.toml`:

```sh
bun run onboard --config /absolute/path/config.toml
```

The command only inspects and plans by default. `--execute` signs and submits transactions and requires separate, explicit operator authorization. Keep signer files and real configuration outside Git. No credentials, RPC connection or wallet are needed for metadata validation or tests.

See [metadata details](metadata/README.md) and the [metadata reference](https://fossa.ag/docs/technical/metadata). Synchronize shared artifacts from this repository before releasing consumers:

```sh
bun metadata/sync.ts --write --backend ../backend --stealth ../stealth --frontend ../frontend
bun metadata/sync.ts --check --backend ../backend --stealth ../stealth --frontend ../frontend
```

Each target is optional and explicit. Solana program source, harnesses and program upgrade tooling remain in `usefossa/programs`; the maintained inference provider remains in `usefossa/stealth`.
