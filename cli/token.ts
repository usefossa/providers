import { PublicKey, SystemProgram, TransactionInstruction } from "@solana/web3.js";

export const TOKEN_PROGRAM_ID = new PublicKey("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
const ASSOCIATED_TOKEN_PROGRAM_ID = new PublicKey("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

export function getAssociatedTokenAddressSync(mint: PublicKey, owner: PublicKey, allowOwnerOffCurve = false) {
  if (!allowOwnerOffCurve && !PublicKey.isOnCurve(owner.toBytes())) throw Error("Token owner is off curve");
  return PublicKey.findProgramAddressSync([owner.toBuffer(), TOKEN_PROGRAM_ID.toBuffer(), mint.toBuffer()], ASSOCIATED_TOKEN_PROGRAM_ID)[0];
}

export function createAssociatedTokenAccountIdempotentInstruction(payer: PublicKey, associatedToken: PublicKey, owner: PublicKey, mint: PublicKey) {
  return new TransactionInstruction({
    programId: ASSOCIATED_TOKEN_PROGRAM_ID, data: Buffer.from([1]),
    keys: [
      { pubkey: payer, isSigner: true, isWritable: true },
      { pubkey: associatedToken, isSigner: false, isWritable: true },
      ...[owner, mint, SystemProgram.programId, TOKEN_PROGRAM_ID].map(pubkey => ({ pubkey, isSigner: false, isWritable: false })),
    ],
  });
}
