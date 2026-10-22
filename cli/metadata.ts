import Ajv2020 from "ajv/dist/2020";
import addFormats from "ajv-formats";
import schema from "../metadata/schema.json";

export type MetadataKind = "provider" | "model";
export const MAX_METADATA_BYTES = 32_768;
export const MAX_METADATA_DEPTH = 64;
const ajv = addFormats(new Ajv2020({ strict: true }));
ajv.addSchema(schema);
const validators = Object.fromEntries((["provider", "model"] as const).map(kind => [kind, ajv.getSchema(`${schema.$id}#/$defs/${kind}`)!]));

type ModelRules = {
  contextTokens: number; outputTokens: number; inputBoundTokens?: number;
  settings?: { maxTokens: number }; effort?: string[];
  variants?: { id: string; body: { reasoning_effort: string } }[];
};

export function parseMetadata(kind: MetadataKind, bytes: Uint8Array): Record<string, unknown> {
  if (bytes.byteLength > MAX_METADATA_BYTES) throw Error(`Metadata exceeds ${MAX_METADATA_BYTES} bytes.`);
  let value: unknown;
  try { value = JSON.parse(new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes)); }
  catch { throw Error("Metadata must be valid UTF-8 JSON."); }
  const pending: [unknown, number][] = [[value, 1]];
  while (pending.length) {
    const [item, depth] = pending.pop()!;
    if (typeof item === "string" && !item.isWellFormed() || typeof item === "number" && !Number.isFinite(item)) throw Error("Metadata must contain Unicode scalar strings and finite numbers.");
    if (item && typeof item === "object") {
      if (depth > MAX_METADATA_DEPTH) throw Error(`Metadata exceeds ${MAX_METADATA_DEPTH} container levels.`);
      for (const [key, child] of Object.entries(item)) {
        if (!key.isWellFormed()) throw Error("Metadata keys must be Unicode scalar strings.");
        pending.push([child, depth + 1]);
      }
    }
  }
  const validate = validators[kind]!;
  if (!validate(value)) {
    const error = validate.errors?.[0];
    throw Error(`Invalid ${kind} metadata at ${error?.instancePath || "/"}: ${error?.message}.`);
  }
  const document = value as Record<string, unknown>;
  if (kind === "model") {
    const model = value as ModelRules;
    if (model.inputBoundTokens !== undefined && model.inputBoundTokens > model.contextTokens) throw Error("Invalid model metadata: inputBoundTokens exceeds contextTokens.");
    if (model.settings && model.settings.maxTokens > model.outputTokens) throw Error("Invalid model metadata: settings.maxTokens exceeds outputTokens.");
    const ids = new Set<string>();
    for (const variant of model.variants ?? []) {
      if (ids.has(variant.id)) throw Error("Invalid model metadata: duplicate variant id.");
      ids.add(variant.id);
    }
    if (model.variants && (model.variants.length !== model.effort!.length || model.variants.some((variant, index) => variant.id !== model.effort![index]))) throw Error("Invalid model metadata: variant ids must match effort in order.");
  }
  return document;
}

if (import.meta.main) {
  const [kind, path, ...extra] = process.argv.slice(2);
  if (!(kind === "provider" || kind === "model") || !path || extra.length) throw Error("Usage: bun metadata.ts provider|model FILE");
  parseMetadata(kind, new Uint8Array(await Bun.file(path).arrayBuffer()));
  console.log(`Valid ${kind} metadata.`);
}
