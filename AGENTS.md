# providers

private `usefossa/providers`; provider onboarding cli in `cli/`, canonical provider/model metadata contract in `metadata/`. follow [workspace rules](../AGENTS.md). this repository builds and tests independently.

- keep repository private; do not publish packages without explicit authorization.
- use bun. no services, subagents, commits or pushes unless requested. signing, onboarding, fund movement and program deployment require explicit authorization; moving or releasing tooling never authorizes them.
- configuration requires `--config <path>` or `FOSSA_CONFIG`; paths resolve relative to that file. examples contain placeholders only. preserve ignored configuration, signer files and caches; never commit keys or private state.
- onboarding defaults to inspection/planning. only explicit `--execute` authorizes tool execution; never use operator credentials or live rpc in tests.
- `metadata/schema.json`, Rust validator and conformance cases are canonical. `metadata/sync.ts` distributes byte-identical copies to explicit backend/provider/frontend targets. consumers build independently; do not add runtime schema downloads or sibling-checkout dependencies.
- edit schema, semantic rules, fixtures and documentation together. preserve exact committed document bytes. incompatible schema changes need a new version; existing conforming documents without a version remain v1.
- program upgrade tooling belongs in sibling `programs/ops/`; provider runtime belongs in sibling `stealth/`.
- checks before requested commit/push: `bun install --cwd cli --frozen-lockfile`, `bun run check`, `bun run test`; use `bun metadata/sync.ts --check` with affected target paths. do not start services or run paid integrations.
- bump affected package/crate semver for behavior changes. commits: `[fix]`, `[feat]`, `[add]`, `[update]`, lowercase single line, no attribution. requested releases use `vX.Y.Z`.

prose and code comments lowercase; preserve identifiers and product/documentation casing. no browser screenshots. honor a user's request to stop verification after push.
