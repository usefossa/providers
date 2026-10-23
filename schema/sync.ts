import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

const files = ["Cargo.toml", "schema.json", "fixtures.json", "src/lib.rs"];
const args = process.argv.slice(2), write = args[0] === "--write";
if (!["--write", "--check"].includes(args.shift() ?? "") || !args.length || args.length % 2) throw Error("Usage: bun schema/sync.ts --check|--write --backend PATH --stealth PATH --frontend PATH --indexer PATH");
for (let index = 0; index < args.length; index += 2) {
  const kind = args[index]!, root = resolve(args[index + 1]!);
  if (!["--backend", "--stealth", "--frontend", "--indexer"].includes(kind)) throw Error(`Unknown target ${kind}`);
  const targets = kind === "--frontend" ? [["schema.json", "public/schemas/metadata/v1.json"], ["fixtures.json", "public/schemas/metadata/v1.cases.json"]] : kind === "--indexer" ? [["schema.json", "indexer/src/metadata/schema.json"], ["fixtures.json", "indexer/src/metadata/fixtures.json"], ["src/lib.rs", "indexer/src/metadata/schema.rs"]] : files.map(file => [file, `vendor/fossa-metadata/${file}`]);
  for (const [source, destination] of targets) {
    let bytes = readFileSync(resolve(import.meta.dir, source!));
    if (kind === "--indexer" && source === "src/lib.rs") bytes = Buffer.from(bytes.toString().replaceAll('include_str!("../', 'include_str!("'));
    const path = resolve(root, destination!);
    if (write) { mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, bytes); }
    else if (!readFileSync(path).equals(bytes)) throw Error(`Metadata artifact differs: ${path}`);
  }
}
console.log(`Metadata artifacts ${write ? "synchronized" : "match canonical source"}.`);
