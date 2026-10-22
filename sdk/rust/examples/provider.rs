use fossa_sdk::{
    Instruction, Keypair, Protocol, Pubkey, Result, RpcClient, Signer,
    accounts::Booking,
    hosted::{HostedClient, PublicConfig},
    inference::{RequestVerifier, VerifiedAuthorization},
    metadata::{Document, Kind},
};
use std::sync::Arc;

// reuse the verifier and streamed account cache; persist replay admission before inference.
pub fn authorize_cached(
    verifier: &RequestVerifier,
    headers: &reqwest::header::HeaderMap,
    body: &[u8],
    address: Pubkey,
    booking: &Booking,
    now: i64,
) -> Result<VerifiedAuthorization> {
    let verified = verifier.verify(headers, body)?;
    verified.verify_booking(address, booking, now)?;
    Ok(verified)
}

pub fn profile_update(
    protocol: &Protocol,
    provider: Pubkey,
    bytes: Vec<u8>,
    uri: &str,
) -> Result<Instruction> {
    let document = Document::parse(Kind::Provider, bytes)?;
    protocol.update_provider_metadata(provider, &document.reference(uri)?)
}

pub async fn hosted_profile_update(
    api_url: &str,
    signer: Arc<Keypair>,
    bytes: Vec<u8>,
) -> Result<Instruction> {
    let document = Document::parse(Kind::Provider, bytes)?;
    let hosted = HostedClient::new(api_url, signer.clone())?;
    let uploaded = hosted.upload_metadata(&document).await?;
    let protocol = PublicConfig::fetch(api_url).await?.protocol();
    protocol.update_provider_metadata(signer.pubkey(), &uploaded.reference()?)
}

pub async fn prepare_and_save(
    rpc: &RpcClient,
    signer: &Keypair,
    instruction: Instruction,
    path: &std::path::Path,
) -> Result<()> {
    let pending = rpc
        .prepare(&[instruction], signer.pubkey())
        .await?
        .sign(&[signer])?;
    rpc.simulate(&pending).await?.check()?;
    pending.save(path)
}

fn main() -> Result<()> {
    let protocol = Protocol {
        program: Pubkey::new_from_array([1; 32]),
        market: Pubkey::new_from_array([2; 32]),
        mint: Pubkey::new_from_array([3; 32]),
    };
    let instruction = profile_update(
        &protocol,
        Pubkey::new_from_array([4; 32]),
        br#"{"name":"Example Provider","endpoint":"https://example.com/infer"}"#.to_vec(),
        "https://example.com/provider.json",
    )?;
    println!(
        "prepared opcode {} with {} accounts",
        instruction.data[0],
        instruction.accounts.len()
    );
    Ok(())
}
