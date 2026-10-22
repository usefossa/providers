import { createHash } from "node:crypto";
import { Connection, Keypair, PublicKey, SystemProgram, type Signer, type TransactionInstruction } from "@solana/web3.js";
import { atomic, integer, loadConfig, loadKeypair, metadata, options, terms, type Config } from "./config";
import { book, instruction, meta, pda, record, send, u32 } from "./chain";
import { createAssociatedTokenAccountIdempotentInstruction, getAssociatedTokenAddressSync, TOKEN_PROGRAM_ID } from "./token";

function matches(data: Buffer, hash: number, uri: number, document: Buffer) {
  const length = data.readUInt16LE(hash + 32);
  return data.subarray(hash, hash + 32).equals(document.subarray(0, 32)) && data.subarray(uri, uri + length).equals(document.subarray(32));
}

export async function onboard(config: Config, execute = false, rpc = new Connection(config.solana.rpc_url, "confirmed")) {
  if (!config.provider) throw Error("Configure [provider] and [[provider.models]] for onboarding.");
  const program = new PublicKey(config.marketplace.program_id), market = new PublicKey(config.marketplace.market);
  const mint = new PublicKey(config.solana.usdc_mint), provider = new PublicKey(config.provider.wallet);
  const profile = pda(program, "provider", market.toBuffer(), provider.toBuffer());
  const profileDoc = metadata(config.base, config.provider, "provider");
  const limit = integer(config.provider.model_book_limit, 4096, 1);
  if (!Array.isArray(config.provider.models)) throw Error("Configure provider.models as an array.");
  const models = config.provider.models.map(model => {
    if (typeof model.id !== "string" || !model.id) throw Error("Model id is required.");
    const offer = { ...config.provider, ...model };
    return { id: model.id, document: metadata(config.base, model, "model", model.id), terms: terms(offer), target: atomic(offer.bond_target_atomic) };
  });
  if (new Set(models.map(model => model.id)).size !== models.length) throw Error("Duplicate model ids.");
  const marketData = record(await rpc.getAccountInfo(market), program, "FOSSAV20", 72);
  if (!marketData.subarray(40, 72).equals(mint.toBuffer())) throw Error("Payment mint does not match market.");
  const adminAddress = new PublicKey(marketData.subarray(8, 40));
  const canAdmin = provider.equals(adminAddress) || Boolean(config.operator?.admin);
  const keys: Signer[] = [], actions: string[] = [];
  try {
    let signer: Signer | undefined, payer: Signer | undefined;
    if (execute) {
      keys.push(signer = loadKeypair(config.base, config.operator?.keypair, provider));
      payer = signer;
      if (config.operator?.payer) keys.push(payer = loadKeypair(config.base, config.operator.payer));
      if (config.operator?.admin) keys.push(loadKeypair(config.base, config.operator.admin, adminAddress));
    }
    const payerAddress = payer?.publicKey ?? provider;
    const ix = (op: number, data: Buffer, accounts: ReturnType<typeof meta>[]) => instruction(program, op, data, accounts);
    async function action(label: string, instructions: TransactionInstruction[], extra: Keypair[] = []) {
      actions.push(label);
      if (execute) await send(rpc, payer!, label, instructions, [...keys, ...extra]);
    }
    let account = await rpc.getAccountInfo(profile);
    let profileData: Buffer | undefined;
    if (account) {
      profileData = record(account, program, "PROV0V20", 364);
      if (!profileData.subarray(8, 40).equals(market.toBuffer()) || !profileData.subarray(40, 72).equals(provider.toBuffer())) throw Error("Provider profile binding mismatch.");
      if (!matches(profileData, 72, 108, profileDoc)) await action("update provider metadata", [ix(21, profileDoc, [meta(market), meta(profile, true), meta(provider, false, true)])]);
    } else {
      await action("register provider", [ix(19, profileDoc, [meta(market), meta(profile, true), meta(provider, false, true), meta(payerAddress, true, true), meta(SystemProgram.programId)])]);
    }
    if (profileData?.[106] !== 1) {
      if (!canAdmin) return [...actions, `provider approval required from ${adminAddress.toBase58()}`];
      await action("approve provider", [ix(20, Buffer.from([1]), [meta(market), meta(profile, true), meta(adminAddress, false, true)])]);
    }
    for (const model of models) {
      const id = createHash("sha256").update(model.id).digest();
      const address = pda(program, "model", market.toBuffer(), id), modelMetadata = pda(program, "model_metadata", address.toBuffer());
      account = await rpc.getAccountInfo(address);
      if (!account) {
        if (!canAdmin) { actions.push(`model ${model.id} must be initialized by market admin`); continue; }
        if (!execute) {
          actions.push(`initialize model ${model.id}`, `register offer ${model.id}`);
          if (model.target > 0n) actions.push(`fund bond ${model.id}: ${model.target} atomic units`);
          continue;
        }
        const termsKey = Keypair.generate(), capacityKey = Keypair.generate();
        try {
          const sizes = [48 + limit * 120, 48 + limit * 32];
          const rents = await Promise.all(sizes.map(size => rpc.getMinimumBalanceForRentExemption(size)));
          const accounts = [meta(market), meta(address, true), meta(termsKey.publicKey, true, true), meta(capacityKey.publicKey, true, true), meta(provider, false, true), meta(payerAddress, true, true), meta(SystemProgram.programId), meta(profile), meta(modelMetadata, true)];
          if (!provider.equals(adminAddress)) accounts.push(meta(adminAddress, false, true));
          await action(`initialize model ${model.id}`, [
            ...[termsKey, capacityKey].map((key, index) => SystemProgram.createAccount({ fromPubkey: payerAddress, newAccountPubkey: key.publicKey, lamports: rents[index]!, space: sizes[index]!, programId: program })),
            ix(18, Buffer.concat([id, u32(limit), model.document]), accounts),
          ], [termsKey, capacityKey]);
        } finally { termsKey.secretKey.fill(0); capacityKey.secretKey.fill(0); }
        account = await rpc.getAccountInfo(address);
      }
      const data = record(account, program, "MODELV20", 144);
      if (!data.subarray(8, 40).equals(market.toBuffer()) || !data.subarray(40, 72).equals(id)) throw Error("Model binding mismatch.");
      const termsAddress = new PublicKey(data.subarray(72, 104)), capacityAddress = new PublicKey(data.subarray(104, 136));
      const [metadataAccount, termsAccount, capacityAccount] = await rpc.getMultipleAccountsInfo([modelMetadata, termsAddress, capacityAddress]);
      const document = record(metadataAccount ?? null, program, "MMETAV20", 396);
      if (!document.subarray(8, 40).equals(market.toBuffer()) || !document.subarray(40, 72).equals(address.toBuffer())) throw Error("Model metadata binding mismatch.");
      if (!matches(document, 104, 140, model.document) && document.subarray(72, 104).equals(provider.toBuffer())) {
        await action(`update model metadata ${model.id}`, [ix(22, model.document, [meta(market), meta(address), meta(modelMetadata, true), meta(provider, false, true)])]);
      }
      const current = book(termsAccount ?? null, program, address, "TERMSV20", 120);
      const capacity = book(capacityAccount ?? null, program, address, "CAPACV20", 32);
      const count = current.readUInt32LE(44);
      if (count !== capacity.readUInt32LE(44) || current.readUInt32LE(40) !== capacity.readUInt32LE(40) || current.readUInt32LE(40) !== data.readUInt32LE(136)) throw Error("Offer books disagree.");
      let slot = 0;
      while (slot < count && !current.subarray(48 + slot * 120, 80 + slot * 120).equals(provider.toBuffer())) slot++;
      const existing = slot < count, slotBytes = u32(slot), at = 48 + slot * 120;
      const authority = pda(program, "bond", address.toBuffer(), slotBytes);
      const source = getAssociatedTokenAddressSync(mint, provider);
      let bond = getAssociatedTokenAddressSync(mint, authority, true);
      if (existing) {
        bond = new PublicKey(current.subarray(at + 64, at + 96));
        if (!current.subarray(at + 96, at + 112).equals(model.terms.subarray(0, 16)) || current.readUInt32LE(at + 116) !== model.terms.readUInt32LE(16)) {
          await action(`update offer ${model.id}`, [ix(5, Buffer.concat([slotBytes, model.terms]), [meta(market), meta(address), meta(termsAddress, true), meta(provider, false, true)])]);
        }
      } else {
        if (count >= current.readUInt32LE(40)) throw Error(`Offer book full for ${model.id}.`);
        await action(`register offer ${model.id}`, [
          createAssociatedTokenAccountIdempotentInstruction(payerAddress, source, provider, mint),
          createAssociatedTokenAccountIdempotentInstruction(payerAddress, bond, authority, mint),
          ix(2, Buffer.concat([slotBytes, model.terms]), [meta(market), meta(address), meta(termsAddress, true), meta(capacityAddress, true), meta(profile), meta(provider, false, true), meta(source), meta(bond)]),
        ]);
      }
      const funded = existing ? capacity.readBigUInt64LE(48 + slot * 32) : 0n;
      if (funded < model.target) {
        const amount = model.target - funded, payload = Buffer.alloc(12);
        payload.writeUInt32LE(slot); payload.writeBigUInt64LE(amount, 4);
        if (execute && BigInt((await rpc.getTokenAccountBalance(source)).value.amount) < amount) throw Error(`Insufficient payment tokens to fund ${model.id}.`);
        await action(`fund bond ${model.id}: ${amount} atomic units`, [ix(4, payload, [meta(market), meta(address), meta(termsAddress), meta(capacityAddress, true), meta(provider, false, true), meta(source, true), meta(bond, true), meta(mint), meta(TOKEN_PROGRAM_ID)])]);
      }
    }
    return actions;
  } finally { for (const key of keys) key.secretKey.fill(0); }
}

if (import.meta.main) {
  const args = options(process.argv.slice(2));
  console.log(JSON.stringify({ execute: args.execute, actions: await onboard(loadConfig(args.path), args.execute) }, null, 2));
}
