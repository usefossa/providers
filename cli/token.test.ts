import { expect, test } from "bun:test";
import { Connection, PublicKey } from "@solana/web3.js";
import { createAssociatedTokenAccountIdempotentInstruction, getAssociatedTokenAddressSync, TOKEN_PROGRAM_ID } from "./token";

const mint = new PublicKey("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
const owner = new PublicKey("GmaDrppBC7P5ARKV8g3djiwP89vz1jLK23V2GBjuAEGB");

test("token account address and instruction match SPL Token reference vector", () => {
  const address = getAssociatedTokenAddressSync(mint, owner);
  expect(address.toBase58()).toBe("7woc3ajaGMMXczFYjxon4aQoHH3j126fMUR9c58eHRsK");
  const instruction = createAssociatedTokenAccountIdempotentInstruction(owner, address, owner, mint);
  expect(instruction.programId.toBase58()).toBe("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");
  expect(instruction.data.toString("hex")).toBe("01");
  expect(instruction.keys.map(key => [key.pubkey.toBase58(), key.isSigner, key.isWritable])).toEqual([
    [owner.toBase58(), true, true], [address.toBase58(), false, true], [owner.toBase58(), false, false],
    [mint.toBase58(), false, false], ["11111111111111111111111111111111", false, false], [TOKEN_PROGRAM_ID.toBase58(), false, false],
  ]);
});

test("program-owned token account requires explicit off-curve permission", () => {
  const authority = new PublicKey("AispKP3Ls5d2m6t411kMtP1xkPemdZPGPJUvemsBqmXA");
  expect(() => getAssociatedTokenAddressSync(mint, authority)).toThrow("off curve");
  expect(getAssociatedTokenAddressSync(mint, authority, true).toBase58()).toBe("6fQcUCMPgjKE6STVFXsyRXH98B3At9cg8468wozEUjtF");
});

test("web3 JSON-RPC transport works with jayson 5", async () => {
  const rpc = new Connection("http://127.0.0.1:8899", {
    commitment: "confirmed",
    fetch: Object.assign(async (_url: Parameters<typeof fetch>[0], init?: Parameters<typeof fetch>[1]) => {
      const request = JSON.parse(String(init?.body));
      expect(request.method).toBe("getBalance");
      expect(request.params[0]).toBe(owner.toBase58());
      return new Response(JSON.stringify({ jsonrpc: "2.0", id: request.id, result: { context: { slot: 1 }, value: 42 } }));
    }, { preconnect: fetch.preconnect }),
  });
  expect(await rpc.getBalance(owner)).toBe(42);
});
