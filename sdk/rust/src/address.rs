use crate::Pubkey;
use sha2::{Digest, Sha256};

pub const SYSTEM_PROGRAM: Pubkey = Pubkey::new_from_array([0; 32]);
pub const TOKEN_PROGRAM: Pubkey =
    Pubkey::from_str_const("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
pub const ASSOCIATED_TOKEN_PROGRAM: Pubkey =
    Pubkey::from_str_const("ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL");

pub fn model_id(id: &str) -> [u8; 32] {
    Sha256::digest(id.as_bytes()).into()
}
pub fn model(program: &Pubkey, market: &Pubkey, id: &[u8; 32]) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"model", market.as_ref(), id], program)
}
pub fn provider(program: &Pubkey, market: &Pubkey, authority: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"provider", market.as_ref(), authority.as_ref()], program)
}
pub fn model_metadata(program: &Pubkey, model: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"model_metadata", model.as_ref()], program)
}
pub fn bond(program: &Pubkey, model: &Pubkey, slot: u32) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"bond", model.as_ref(), &slot.to_le_bytes()], program)
}
pub fn vault(program: &Pubkey, user: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[b"vault", user.as_ref()], program)
}
pub fn receipt_case(program: &Pubkey, bucket: &Pubkey, epoch: i64, index: u64) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            b"receipt-case",
            bucket.as_ref(),
            &epoch.to_le_bytes(),
            &index.to_le_bytes(),
        ],
        program,
    )
}
pub fn associated_token(mint: &Pubkey, owner: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(
        &[owner.as_ref(), TOKEN_PROGRAM.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM,
    )
    .0
}
