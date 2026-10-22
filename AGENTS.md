# providers

fossa cli, rust sdk and metadata schema for fossa. [documentation](https://fossa.ag/docs/providers/cli).

- `sdk/rust/`: complete protocol sdk; offline builders, typed accounts, pooled rpc and signing.
- `cli/`: noninteractive rust binary using sdk. help and linked website docs replace skills.
- `schema/`: canonical metadata schema, validator and conformance cases. preserve contract and exact document bytes.

## workflow

- read implementation first. use subagents only when requested.
- no commits, pushes, deployments or services unless requested. tooling releases never authorize onboarding, signing, program deployment or fund movement.
- keep prose/comments lowercase and compact. preserve exact identifiers, paths, commands and user-facing strings. label every fenced code block with its language.
- configuration and signer files stay outside git. examples use placeholders. never log secrets or signed transaction bytes.
- cli defaults to inspection/planning; execution requires explicit flag. never prompt interactively. emit machine-readable output and useful errors/help.
- keep sdk transport pooled and reusable; preserve exact accounting, account bindings and instruction compatibility. custom rpc and metadata hosting remain supported.
- schema consumers vendor independent copies; no sibling runtime dependencies or schema downloads. use `bun schema/sync.ts --check` with explicit affected targets.
- before requested commit/push, run `cargo fmt --all --check`, `cargo test --workspace --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings` and final diff review. tests use synthetic keys and mock rpc only.
- bump affected semver for behavior changes; tag requested releases `vX.Y.Z`. commits use `[fix]`, `[feat]`, `[add]`, `[update]`, lowercase single line, no attribution.
