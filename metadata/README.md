# Metadata v1

`schema.json` is the canonical JSON Schema Draft 2020-12 contract for provider and model documents. Select `#/$defs/provider` or `#/$defs/model` according to the on-chain account. The root accepts either shape and must not replace account-specific validation.

The schema is served at `/schemas/metadata/v1.json`. [Metadata reference](https://fossa.ag/docs/technical/metadata) documents every field, defaults, semantic checks, commitment verification and complete examples. Existing valid documents without `schemaVersion` or `$schema` remain v1. New examples identify v1 explicitly. Unsupported versions are rejected; incompatible changes require a new schema URL/version.

The Rust crate and `../cli/metadata.ts` compile this same schema, enforce URI formats and apply four semantic checks: `inputBoundTokens <= contextTokens`, `settings.maxTokens <= outputTokens`, unique variant IDs, and variant IDs matching `effort` labels in the same order. A label may map to a different request value: `thinking` can send `reasoning_effort: high`. Variant bodies allow only reasoning effort and the listed sampling controls. Validation never rewrites the bytes used for the on-chain SHA-256. Documents are limited to 32,768 UTF-8 bytes. Unknown extension fields are retained; unknown request-body fields in variants are rejected.

Documents use Unicode scalar strings and finite JSON numbers, without a UTF-8 BOM, and at most 64 object/array nesting levels including the root. `fixtures.json` supplies shared valid/invalid cases for Rust and TypeScript, also served at `/schemas/metadata/v1.cases.json` for independent implementations. Run offline validation from the providers repository with:

```sh
cargo test --locked --manifest-path metadata/Cargo.toml
bun run --cwd cli test
bun run --cwd cli check
```

The API and maintained provider vendor this crate so each repository builds independently. The website vendors the schema for public download and imports its examples into documentation. Synchronize these byte-identical artifacts from this repository:

```sh
bun metadata/sync.ts --write --backend ../backend --stealth ../stealth --frontend ../frontend
bun metadata/sync.ts --check --backend ../backend --stealth ../stealth --frontend ../frontend
```

Targets are explicit; a single repository can be supplied. Edit canonical files here, then synchronize and verify affected consumers before releasing them. No schema registry or runtime schema download is required. The on-chain program continues to store only the document hash and URI; it does not execute JSON validation.
