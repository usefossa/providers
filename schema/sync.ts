import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

const files = ["Cargo.toml", "schema.json", "fixtures.json", "src/lib.rs"];
const args = process.argv.slice(2), write = args[0] === "--write";
if (!["--write", "--check"].includes(args.shift() ?? "") || !args.length || args.length % 2) throw Error("Usage: bun schema/sync.ts --check|--write --backend PATH --stealth PATH --frontend PATH");
for (let index = 0; index < args.length; index += 2) {
  const kind = args[index]!, root = resolve(args[index + 1]!);
  if (!["--backend", "--stealth", "--frontend"].includes(kind)) throw Error(`Unknown target ${kind}`);
  const targets = kind === "--frontend" ? [["schema.json", "public/schemas/metadata/v1.json"], ["fixtures.json", "public/schemas/metadata/v1.cases.json"]] : files.map(file => [file, `vendor/fossa-metadata/${file}`]);
  for (const [source, destination] of targets) {
    const bytes = readFileSync(resolve(import.meta.dir, source!)), path = resolve(root, destination!);
    if (write) { mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, bytes); }
    else if (!readFileSync(path).equals(bytes)) throw Error(`Metadata artifact differs: ${path}`);
  }
}
console.log(`Metadata artifacts ${write ? "synchronized" : "match canonical source"}.`);
