import { Connection, PublicKey, Transaction, TransactionInstruction, sendAndConfirmTransaction, type AccountInfo, type AccountMeta, type Signer } from "@solana/web3.js";

export function record(account: AccountInfo<Buffer> | null, program: PublicKey, magic: string, size: number): Buffer {
  if (!account || !account.owner.equals(program) || account.data.length !== size || account.data.subarray(0, 8).toString() !== magic) throw Error(`Invalid ${magic} account.`);
  return account.data;
}

export function book(account: AccountInfo<Buffer> | null, program: PublicKey, model: PublicKey, magic: string, entrySize: number): Buffer {
  if (!account || account.data.length < 48) throw Error(`Invalid ${magic} book.`);
  const limit = account.data.readUInt32LE(40), count = account.data.readUInt32LE(44);
  const data = record(account, program, magic, 48 + limit * entrySize);
  if (limit < 1 || limit > 4096 || count > limit || !data.subarray(8, 40).equals(model.toBuffer())) throw Error(`Invalid ${magic} book binding or count.`);
  return data;
}

export const meta = (pubkey: PublicKey, isWritable = false, isSigner = false): AccountMeta => ({ pubkey, isWritable, isSigner });
export const pda = (program: PublicKey, name: string, ...seeds: Buffer[]) => PublicKey.findProgramAddressSync([Buffer.from(name), ...seeds], program)[0];
export const instruction = (program: PublicKey, op: number, data: Buffer, keys: AccountMeta[]) => new TransactionInstruction({ programId: program, data: Buffer.concat([Buffer.from([op]), data]), keys });
export const u32 = (value: number) => { const data = Buffer.alloc(4); data.writeUInt32LE(value); return data; };

export async function send(rpc: Connection, payer: Signer, label: string, instructions: TransactionInstruction[], signers: Signer[]) {
  const required = new Set([payer.publicKey.toBase58(), ...instructions.flatMap(ix => ix.keys.filter(key => key.isSigner).map(key => key.pubkey.toBase58()))]);
  const keys = [...new Map([payer, ...signers].filter(key => required.has(key.publicKey.toBase58())).map(key => [key.publicKey.toBase58(), key])).values()];
  const signature = await sendAndConfirmTransaction(rpc, new Transaction().add(...instructions), keys, { commitment: "confirmed" });
  console.log(label, signature);
}
