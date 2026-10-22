# metadata

canonical provider/model metadata contract: `schema.json`, rust validator and shared `fixtures.json`. select provider or model definition according to on-chain account; schema root accepts either.

v1 accepts existing documents without version fields. unknown extensions remain valid. validator checks field formats, token limits and reasoning variants without changing committed bytes. maximum document size: 32,768 bytes; maximum container depth: 64.

```bash
cargo test --locked -p fossa-metadata
```

synchronize independent consumers after contract changes:

```bash
bun schema/sync.ts --write --backend ../backend --stealth ../stealth --frontend ../frontend
bun schema/sync.ts --check --backend ../backend --stealth ../stealth --frontend ../frontend
```

targets are optional and explicit. public schema lives at `/schemas/metadata/v1.json`; cases at `/schemas/metadata/v1.cases.json`. [metadata reference](https://fossa.ag/docs/technical/metadata.md) covers fields, examples and commitment rules.
