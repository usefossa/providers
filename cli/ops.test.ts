import { afterEach, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { Connection, Keypair, PublicKey, SystemProgram, Transaction, type AccountInfo, type Signer } from "@solana/web3.js";
import { atomic, integer, loadConfig, loadKeypair, metadata, options, terms, type Config, type Offer } from "./config";
import { book, pda, record } from "./chain";
import { onboard } from "./onboard";

const directories: string[] = [];
afterEach(() => { for (const path of directories.splice(0)) rmSync(path, { recursive: true, force: true }); });
const key = (seed: number) => Keypair.fromSeed(new Uint8Array(32).fill(seed));
const program = key(1).publicKey, market = key(2).publicKey, provider = key(3).publicKey, mint = key(4).publicKey;
const offer: Offer = { input_rate: 1_000_000, cached_rate: 500_000, output_rate: 2_000_000, speed_tps: 100, capacity: 4, bond_target_atomic: "1000000" };
function fixture() {
  const base = mkdtempSync(join(tmpdir(), "fossa-ops-")); directories.push(base);
  const body = '{ "name": "Example", "endpoint": "https://example.test/v1" }\n';
  writeFileSync(join(base, "metadata.json"), body);
  writeFileSync(join(base, "model.json"), JSON.stringify({ id: "example/model", name: "Example Model", modalities: { input: ["text"], output: ["text"] }, contextTokens: 131072, outputTokens: 8192 }));
  const document = { metadata_file: "metadata.json", metadata_uri: "https://example.test/metadata.json" };
  const config: Config = {
    base, solana: { rpc_url: "http://rpc.invalid", usdc_mint: mint.toBase58() },
    marketplace: { program_id: program.toBase58(), market: market.toBase58() },
    operator: { keypair: "missing-key.json", admin: "missing-admin.json" },
    provider: { ...offer, ...document, wallet: provider.toBase58(), model_book_limit: 16, models: [{ ...document, metadata_file: "model.json", id: "example/model" }] },
  };
  return { base, body, document, config };
}
function account(magic: string, size: number, owner = program): AccountInfo<Buffer> {
  const data = Buffer.alloc(size); data.write(magic);
  return { data, owner, lamports: 1_000_000, executable: false, rentEpoch: 0 };
}
function marketAccount(admin = provider) {
  const value = account("FOSSAV20", 72);
  admin.toBuffer().copy(value.data, 8); mint.toBuffer().copy(value.data, 40);
  return value;
}
function rpcFixture(accounts: Map<string, AccountInfo<Buffer>>) {
  const calls: string[] = [];
  const rpc = {
    getAccountInfo: async (address: PublicKey) => { calls.push(address.toBase58()); return accounts.get(address.toBase58()) ?? null; },
    getMultipleAccountsInfo: async (addresses: PublicKey[]) => addresses.map(address => accounts.get(address.toBase58()) ?? null),
    getMinimumBalanceForRentExemption: async (bytes: number) => bytes * 10,
    getBalance: async () => 1_000_000_000,
    sendTransaction: async () => { throw Error("Unexpected broadcast in a read-only plan."); },
  } as unknown as Connection;
  return { rpc, calls };
}

test("configuration resolves relative to its file without loading keys or contacting services", () => {
  const { base } = fixture(), path = join(base, "operator.toml");
  writeFileSync(path, `[solana]\nrpc_url = "http://rpc.invalid"\n[marketplace]\nprogram_id = "${program}"\n`);
  expect(loadConfig(path).base).toBe(base);
  writeFileSync(path, `[solana]\nrpc_url = "file:///tmp/rpc"\n[marketplace]\nprogram_id = "${program}"\n`);
  expect(() => loadConfig(path)).toThrow("HTTP");
});

test("command options require explicit execution and reject typos or missing values", () => {
  expect(options([])).toEqual({ path: undefined, execute: false });
  expect(options(["--config", "a path/operator.toml", "--execute"])).toEqual({ path: "a path/operator.toml", execute: true });
  for (const args of [["--config"], ["--config", "--execute"], ["--exectue"], ["extra"]]) expect(() => options(args)).toThrow();
});

test("metadata hashes exact published bytes, preserving whitespace and trailing newline", () => {
  const { base, body, document } = fixture(), value = metadata(base, document, "provider");
  expect(value.subarray(0, 32).toString("hex")).toBe(createHash("sha256").update(body).digest("hex"));
  expect(value.subarray(0, 32).toString("hex")).not.toBe(createHash("sha256").update(JSON.stringify(JSON.parse(body))).digest("hex"));
  expect(value.subarray(32).toString()).toBe(document.metadata_uri);
  expect(metadata(base, { ...document, metadata_uri: "é".repeat(128) }, "provider").length).toBe(288);
});

test.each([["empty", ""], ["oversized ASCII", "x".repeat(257)], ["oversized UTF-8", "é".repeat(129)], ["space", "https://x/ a"], ["newline", "https://x/\n"], ["null", "https://x/\0"], ["control", "https://x/\x7f"]])("metadata rejects %s URI", (_label, uri) => {
  const { base, document } = fixture();
  expect(() => metadata(base, { ...document, metadata_uri: uri }, "provider")).toThrow("Metadata URI");
});

test.each(["null", "[]", '"value"', "42", "{"])("metadata rejects invalid JSON document %s", body => {
  const { base, document } = fixture(); writeFileSync(join(base, document.metadata_file), body);
  expect(() => metadata(base, document, "provider")).toThrow();
});

test("standard Solana keypair files produce usable, clearable signers", () => {
  const { base } = fixture(), source = key(7);
  writeFileSync(join(base, "signer.json"), JSON.stringify([...source.secretKey]));
  const signer = loadKeypair(base, "signer.json", source.publicKey);
  const tx = new Transaction({ feePayer: signer.publicKey, recentBlockhash: key(8).publicKey.toBase58() }).add(SystemProgram.transfer({ fromPubkey: signer.publicKey, toPubkey: market, lamports: 1 }));
  tx.sign(signer); expect(tx.verifySignatures()).toBe(true);
  expect(() => loadKeypair(base, "signer.json", provider)).toThrow("authority");
  signer.secretKey.fill(0); expect(signer.secretKey.every(byte => byte === 0)).toBe(true);
});

test.each([null, [], Array(63).fill(0), Array(64).fill(-1), Array(64).fill(256), Array(64).fill(0.5), Array(64).fill("1"), Array(64).fill(0)].map(value => [value]))("signer validation rejects malformed key data %#", value => {
  const { base } = fixture(); writeFileSync(join(base, "bad.json"), JSON.stringify(value));
  expect(() => loadKeypair(base, "bad.json")).toThrow();
});

test("token amounts preserve u64 precision and reject unsafe numeric inputs", () => {
  expect(atomic("18446744073709551615")).toBe(0xffff_ffff_ffff_ffffn);
  expect(atomic(0)).toBe(0n); expect(atomic(Number.MAX_SAFE_INTEGER)).toBe(BigInt(Number.MAX_SAFE_INTEGER));
  for (const value of [-1, 1.1, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1, "-1", "1.5", "", "0x10", "18446744073709551616"]) expect(() => atomic(value)).toThrow();
  for (const value of [-1, 1.1, NaN, Infinity, 0x1_0000_0000]) expect(() => integer(value)).toThrow();
});

test("offer encoding matches fixed little-endian ABI vector", () => {
  const data = terms({ ...offer, input_rate: 0x01020304, cached_rate: 2, output_rate: 0x05060708, speed_tps: 0x1234, private: true, active: false, capacity: 0x0a0b0c0d });
  expect(data.toString("hex")).toBe("040302010200000008070605341201000d0c0b0a");
});

test.each([
  { input_rate: 0 }, { input_rate: -1 }, { input_rate: 1.5 }, { cached_rate: 1_000_001 }, { output_rate: 0 },
  { speed_tps: 0 }, { speed_tps: 65_536 }, { capacity: 0 }, { capacity: 0x1_0000_0000 }, { bond_target_atomic: "-1" },
])("invalid offer rejected before RPC %#", change => expect(() => terms({ ...offer, ...change })).toThrow());

test("account readers reject wrong owners, magic, lengths, model bindings and book counts", () => {
  const value = account("TERMSV20", 48 + 120);
  market.toBuffer().copy(value.data, 8); value.data.writeUInt32LE(1, 40); value.data.writeUInt32LE(1, 44);
  expect(book(value, program, market, "TERMSV20", 120)).toBe(value.data);
  expect(() => book(value, program, provider, "TERMSV20", 120)).toThrow();
  expect(() => record({ ...value, owner: mint }, program, "TERMSV20", value.data.length)).toThrow();
  expect(() => record(value, program, "CAPACV20", value.data.length)).toThrow();
  expect(() => record(value, program, "TERMSV20", value.data.length + 1)).toThrow();
  expect(() => book(null, program, market, "TERMSV20", 120)).toThrow();
  value.data.writeUInt32LE(2, 44); expect(() => book(value, program, market, "TERMSV20", 120)).toThrow();
});

test("onboarding read-only plan needs no keys and performs no writes", async () => {
  const { config } = fixture(), { rpc } = rpcFixture(new Map([[market.toBase58(), marketAccount()]]));
  expect(await onboard(config, false, rpc)).toEqual(["register provider", "approve provider", "initialize model example/model", "register offer example/model", "fund bond example/model: 1000000 atomic units"]);
});

test("ordinary providers can register without possessing the market admin key", async () => {
  const { config } = fixture(); config.operator = undefined;
  const admin = key(9).publicKey, { rpc } = rpcFixture(new Map([[market.toBase58(), marketAccount(admin)]]));
  const actions = await onboard(config, false, rpc);
  expect(actions).toEqual(["register provider", `provider approval required from ${admin.toBase58()}`]);
});

test("invalid offer configuration and duplicate model ids fail before RPC", async () => {
  const { config } = fixture(), { rpc, calls } = rpcFixture(new Map());
  config.provider.models.push({ ...config.provider.models[0]! });
  await expect(onboard(config, false, rpc)).rejects.toThrow("Duplicate");
  config.provider.models.pop(); config.provider.models[0]!.capacity = 0;
  await expect(onboard(config, false, rpc)).rejects.toThrow("integer");
  expect(calls).toHaveLength(0);
});

test("invalid provider/model metadata and mismatched model IDs fail before RPC or signer access", async () => {
  const { config, base, body } = fixture(), { rpc, calls } = rpcFixture(new Map());
  writeFileSync(join(base, "metadata.json"), JSON.stringify({ name: "Provider", endpoint: "http://example.test/v1" }));
  await expect(onboard(config, true, rpc)).rejects.toThrow("provider metadata");
  writeFileSync(join(base, "metadata.json"), body);
  writeFileSync(join(base, "model.json"), JSON.stringify({ id: "example/model" }));
  await expect(onboard(config, true, rpc)).rejects.toThrow("model metadata");
  writeFileSync(join(base, "model.json"), JSON.stringify({ id: "different/model", name: "Model", modalities: { input: ["text"], output: ["text"] }, contextTokens: 100, outputTokens: 10 }));
  await expect(onboard(config, true, rpc)).rejects.toThrow("configured model id");
  expect(calls).toHaveLength(0);
});

test("onboarding rejects market/payment mismatches", async () => {
  const { config } = fixture(), value = marketAccount(); value.data.fill(0, 40);
  const { rpc } = rpcFixture(new Map([[market.toBase58(), value]]));
  await expect(onboard(config, false, rpc)).rejects.toThrow("mint");
});

test("existing provider metadata compares URI as well as document hash", async () => {
  const { config } = fixture(); config.provider.models = [];
  const profile = pda(program, "provider", market.toBuffer(), provider.toBuffer()), value = account("PROV0V20", 364);
  market.toBuffer().copy(value.data, 8); provider.toBuffer().copy(value.data, 40); value.data[106] = 1;
  const payload = metadata(config.base, config.provider, "provider");
  payload.subarray(0, 32).copy(value.data, 72); value.data.writeUInt16LE(1, 104); value.data.write("x", 108);
  const { rpc } = rpcFixture(new Map([[market.toBase58(), marketAccount()], [profile.toBase58(), value]]));
  expect(await onboard(config, false, rpc)).toEqual(["update provider metadata"]);
  value.data.writeUInt16LE(payload.length - 32, 104); payload.subarray(32).copy(value.data, 108);
  expect(await onboard(config, false, rpc)).toEqual([]);
});

test("onboarding supports distinct provider/admin/payer keys, exact funding and idempotent reruns", async () => {
  const { config, base } = fixture(), admin = key(9), payer = key(12);
  config.operator = { keypair: "provider.json", admin: "admin.json", payer: "payer.json" };
  for (const [file, signer] of [["provider.json", key(3)], ["admin.json", admin], ["payer.json", payer]] as const) writeFileSync(join(base, file), JSON.stringify([...signer.secretKey]));
  config.provider.bond_target_atomic = "9007199254740993";
  const accounts = new Map([[market.toBase58(), marketAccount(admin.publicKey)]]), { rpc } = rpcFixture(accounts);
  const operations: number[] = [], loaded: Signer[] = [];
  rpc.getTokenAccountBalance = async () => ({ context: { slot: 1 }, value: { amount: "18446744073709551615", decimals: 6, uiAmount: null } });
  rpc.confirmTransaction = (async () => ({ context: { slot: 1 }, value: { err: null } })) as Connection["confirmTransaction"];
  rpc.sendTransaction = (async (tx: Transaction, signers: Signer[]) => {
    tx.feePayer = payer.publicKey; tx.recentBlockhash = key(13).publicKey.toBase58(); tx.sign(...signers);
    expect(tx.verifySignatures()).toBe(true); expect(tx.serialize().length).toBeLessThanOrEqual(1232);
    loaded.push(...signers.filter(signer => !(signer instanceof Keypair)));
    for (const ix of tx.instructions.filter(ix => ix.programId.equals(program))) {
      const op = ix.data[0]!; operations.push(op);
      const address = (index: number) => ix.keys[index]!.pubkey.toBase58();
      const data = ix.data.subarray(1);
      if (op === 19) {
        const profile = account("PROV0V20", 364);
        market.toBuffer().copy(profile.data, 8); provider.toBuffer().copy(profile.data, 40);
        data.subarray(0, 32).copy(profile.data, 72); profile.data.writeUInt16LE(data.length - 32, 104); data.subarray(32).copy(profile.data, 108);
        accounts.set(address(1), profile);
      } else if (op === 20) accounts.get(address(1))!.data[106] = data[0]!;
      else if (op === 18) {
        expect(ix.keys[9]!.pubkey.equals(admin.publicKey)).toBe(true);
        const model = account("MODELV20", 144), document = account("MMETAV20", 396), limit = data.readUInt32LE(32);
        market.toBuffer().copy(model.data, 8); data.subarray(0, 32).copy(model.data, 40);
        ix.keys[2]!.pubkey.toBuffer().copy(model.data, 72); ix.keys[3]!.pubkey.toBuffer().copy(model.data, 104); model.data.writeUInt32LE(limit, 136);
        market.toBuffer().copy(document.data, 8); ix.keys[1]!.pubkey.toBuffer().copy(document.data, 40); provider.toBuffer().copy(document.data, 72);
        data.subarray(36, 68).copy(document.data, 104); document.data.writeUInt16LE(data.length - 68, 136); data.subarray(68).copy(document.data, 140);
        accounts.set(address(1), model); accounts.set(address(8), document);
        for (const [index, magic, size] of [[2, "TERMSV20", 120], [3, "CAPACV20", 32]] as const) {
          const value = account(magic, 48 + limit * size);
          ix.keys[1]!.pubkey.toBuffer().copy(value.data, 8); value.data.writeUInt32LE(limit, 40); accounts.set(address(index), value);
        }
      } else if (op === 2) {
        const terms = accounts.get(address(2))!.data, capacity = accounts.get(address(3))!.data;
        provider.toBuffer().copy(terms, 48); ix.keys[7]!.pubkey.toBuffer().copy(terms, 112);
        data.subarray(4, 20).copy(terms, 144); data.subarray(20).copy(terms, 164);
        terms.writeUInt32LE(1, 44); capacity.writeUInt32LE(1, 44);
      } else if (op === 4) accounts.get(address(3))!.data.writeBigUInt64LE(data.readBigUInt64LE(4), 48);
      else throw Error(`Unexpected opcode ${op}`);
    }
    return "offline-signature";
  }) as Connection["sendTransaction"];
  await onboard(config, true, rpc);
  expect(operations).toEqual([19, 20, 18, 2, 4]);
  expect(loaded.every(signer => signer.secretKey.every(byte => byte === 0))).toBe(true);
  expect(await onboard(config, true, rpc)).toEqual([]);
  expect(operations).toEqual([19, 20, 18, 2, 4]);
});
