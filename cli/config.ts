import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { Keypair, PublicKey, type Signer } from "@solana/web3.js";
import { parseMetadata, type MetadataKind } from "./metadata";

export type Document = { metadata_file: string; metadata_uri: string };
export type Offer = {
  input_rate: number; cached_rate: number; output_rate: number; speed_tps: number; capacity: number;
  private?: boolean; active?: boolean; bond_target_atomic: number | string;
};
export type Model = Document & Partial<Offer> & { id: string };
export type Config = {
  base: string;
  solana: { rpc_url: string; usdc_mint: string };
  marketplace: { program_id: string; market: string };
  operator?: { keypair?: string; payer?: string; admin?: string };
  provider: Document & Offer & { wallet: string; model_book_limit: number; models: Model[] };
};

export function loadConfig(path = process.env.FOSSA_CONFIG): Config {
  if (!path) throw Error("Pass --config <path> or set FOSSA_CONFIG.");
  const config = Bun.TOML.parse(readFileSync(resolve(path), "utf8")) as unknown as Config;
  if (typeof config.solana?.rpc_url !== "string") throw Error("Configure solana.rpc_url.");
  const url = new URL(config.solana?.rpc_url);
  if (!["http:", "https:"].includes(url.protocol)) throw Error("RPC URL must use HTTP or HTTPS.");
  if (typeof config.marketplace?.program_id !== "string") throw Error("Configure marketplace.program_id.");
  new PublicKey(config.marketplace?.program_id);
  return { ...config, base: dirname(resolve(path)) };
}

export function options(args: string[]) {
  let path: string | undefined, execute = false;
  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--execute") execute = true;
    else if (args[i] === "--config" && args[i + 1] && !args[i + 1]!.startsWith("--")) path = args[++i];
    else throw Error(`Unknown or incomplete argument: ${args[i]}`);
  }
  return { path, execute };
}

export function loadKeypair(base: string, path?: string, expected?: PublicKey): Signer {
  if (!path) throw Error("Configure a Solana JSON keypair file under [operator].");
  const raw = readFileSync(resolve(base, path));
  let values: unknown;
  try {
    try { values = JSON.parse(raw.toString("utf8")); }
    catch { throw Error("Invalid Solana JSON keypair."); }
    if (!Array.isArray(values) || values.length !== 64 || !values.every(v => Number.isInteger(v) && v >= 0 && v <= 255)) throw Error("Expected a 64-byte Solana JSON keypair.");
    const bytes = Uint8Array.from(values);
    try {
      const publicKey = Keypair.fromSecretKey(bytes).publicKey;
      if (expected && !publicKey.equals(expected)) throw Error("Signer does not match the required authority.");
      return { publicKey, secretKey: bytes };
    } catch (error) { bytes.fill(0); throw error; }
  } finally {
    raw.fill(0);
    if (Array.isArray(values)) values.fill(0);
  }
}

export function metadata(base: string, document: Document, kind: MetadataKind, modelId?: string): Buffer {
  if (typeof document.metadata_uri !== "string" || typeof document.metadata_file !== "string") throw Error("Configure metadata_file and metadata_uri.");
  const uri = Buffer.from(document.metadata_uri ?? "");
  if (!uri.length || uri.length > 256 || uri.some(byte => byte <= 32 || byte === 127)) throw Error("Metadata URI must be 1–256 UTF-8 bytes without ASCII whitespace or controls.");
  const body = readFileSync(resolve(base, document.metadata_file));
  const value = parseMetadata(kind, body);
  if (kind === "model" && value.id !== modelId) throw Error("Model metadata id must match configured model id.");
  return Buffer.concat([createHash("sha256").update(body).digest(), uri]);
}

export function integer(value: number, max = 0xffff_ffff, min = 0): number {
  if (!Number.isSafeInteger(value) || value < min || value > max) throw Error(`Expected integer between ${min} and ${max}.`);
  return value;
}

export function atomic(value: number | string): bigint {
  if (typeof value === "number" && !Number.isSafeInteger(value)) throw Error("Unsafe token amount; use a decimal string.");
  if (!/^\d+$/.test(String(value))) throw Error("Expected an unsigned token amount.");
  const amount = BigInt(value);
  if (amount > 0xffff_ffff_ffff_ffffn) throw Error("Token amount exceeds u64.");
  return amount;
}

export function terms(offer: Offer): Buffer {
  const data = Buffer.alloc(20);
  for (const [index, value] of [offer.input_rate, offer.cached_rate, offer.output_rate].entries()) data.writeUInt32LE(integer(value), index * 4);
  if (!offer.input_rate || !offer.output_rate || offer.cached_rate > offer.input_rate) throw Error("Invalid offer rates.");
  data.writeUInt16LE(integer(offer.speed_tps, 0xffff, 1), 12);
  for (const flag of [offer.private, offer.active]) if (flag !== undefined && typeof flag !== "boolean") throw Error("Offer flags must be boolean.");
  data[14] = Number(offer.private ?? false);
  data[15] = Number(offer.active ?? true);
  data.writeUInt32LE(integer(offer.capacity, 0xffff_ffff, 1), 16);
  atomic(offer.bond_target_atomic);
  return data;
}
