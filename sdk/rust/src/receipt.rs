use crate::{
    Error, Instruction, Pubkey, Result,
    accounts::{Booking, BookingStatus, ReceiptBucket, i64le, key, n64},
    amount::Usage,
    ensure,
};
use sha2::{Digest, Sha256};

pub const RECEIPT_LEN: usize = 120;
pub const DEPTH: usize = 16;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub owner: Pubkey,
    pub user: Pubkey,
    pub booking: Pubkey,
    pub generation: u64,
    pub charge: u64,
    pub dispute_end: i64,
}
impl Receipt {
    pub fn encode(&self) -> [u8; RECEIPT_LEN] {
        let mut p = [0; RECEIPT_LEN];
        p[..32].copy_from_slice(self.owner.as_ref());
        p[32..64].copy_from_slice(self.user.as_ref());
        p[64..96].copy_from_slice(self.booking.as_ref());
        p[96..104].copy_from_slice(&self.generation.to_le_bytes());
        p[104..112].copy_from_slice(&self.charge.to_le_bytes());
        p[112..].copy_from_slice(&self.dispute_end.to_le_bytes());
        p
    }
    pub fn decode(p: &[u8]) -> Result<Self> {
        ensure(p.len() == RECEIPT_LEN, "receipt length")?;
        Ok(Self {
            owner: key(p, 0),
            user: key(p, 32),
            booking: key(p, 64),
            generation: n64(p, 96),
            charge: n64(p, 104),
            dispute_end: i64le(p, 112),
        })
    }
    pub fn from_booking(address: Pubkey, booking: &Booking, usage: Usage) -> Result<Self> {
        ensure(
            booking.status == BookingStatus::Reserved && booking.generation > 0,
            "reusable reserved booking",
        )?;
        ensure(
            u64::from(usage.uncached) + u64::from(usage.cached) <= u64::from(booking.max_input)
                && usage.output <= booking.max_output
                && (usage.cached == 0 || booking.cache_allowed),
            "claim usage",
        )?;
        let charge = booking.rates.charge(usage);
        ensure(charge <= booking.reserve, "claim charge")?;
        Ok(Self {
            owner: booking.owner,
            user: booking.user,
            booking: address,
            generation: booking.generation,
            charge,
            dispute_end: booking.dispute_end,
        })
    }
}
fn hash(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p);
    }
    h.finalize().into()
}
pub fn leaf(
    program: &Pubkey,
    bucket: &Pubkey,
    epoch: i64,
    index: u64,
    receipt: &Receipt,
) -> [u8; 32] {
    hash(&[
        b"fossa/receipt/v1",
        program.as_ref(),
        bucket.as_ref(),
        &epoch.to_le_bytes(),
        &index.to_le_bytes(),
        &receipt.encode(),
    ])
}
pub fn node(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    hash(&[b"fossa/node/v1", left, right])
}
fn mountain(count: u64, index: u64) -> Result<(u64, usize)> {
    ensure(
        count < (1 << DEPTH) && index < count,
        "receipt index or count",
    )?;
    let mut base = 0;
    for height in (0..DEPTH).rev() {
        let size = 1u64 << height;
        if count & size != 0 {
            if index < base + size {
                return Ok((base, height));
            }
            base += size;
        }
    }
    Err(Error::Invalid("receipt mountain"))
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiptProof {
    pub epoch: i64,
    pub index: u64,
    pub receipt: Receipt,
    pub siblings: Vec<[u8; 32]>,
}
impl ReceiptProof {
    pub fn encode(&self) -> Result<Vec<u8>> {
        ensure(
            self.siblings.len() < DEPTH && self.index < 65535,
            "receipt proof depth or index",
        )?;
        let mut p = Vec::with_capacity(136 + self.siblings.len() * 32);
        p.extend(self.epoch.to_le_bytes());
        p.extend(self.index.to_le_bytes());
        p.extend(self.receipt.encode());
        for s in &self.siblings {
            p.extend(s);
        }
        Ok(p)
    }
    pub fn verify(&self, program: &Pubkey, bucket: &Pubkey, state: &ReceiptBucket) -> Result<()> {
        ensure(
            state.epoch == self.epoch && !state.finalized && self.receipt.charge > 0,
            "receipt epoch, charge or finalized state",
        )?;
        let (base, height) = mountain(state.count, self.index)?;
        ensure(self.siblings.len() == height, "receipt proof length")?;
        let mut h = leaf(program, bucket, self.epoch, self.index, &self.receipt);
        for (level, sibling) in self.siblings.iter().enumerate() {
            h = if (self.index - base) & (1 << level) == 0 {
                node(&h, sibling)
            } else {
                node(sibling, &h)
            };
        }
        ensure(h == state.frontier[height], "receipt proof root")
    }
    pub fn build(
        program: &Pubkey,
        bucket: &Pubkey,
        epoch: i64,
        receipts: &[Receipt],
        index: u64,
    ) -> Result<Self> {
        let (base, height) = mountain(receipts.len() as u64, index)?;
        let mut hashes = (base..base + (1 << height))
            .map(|i| leaf(program, bucket, epoch, i, &receipts[i as usize]))
            .collect::<Vec<_>>();
        let mut position = (index - base) as usize;
        let mut siblings = Vec::with_capacity(height);
        while hashes.len() > 1 {
            siblings.push(hashes[position ^ 1]);
            hashes = hashes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| node(&pair[0], &pair[1]))
                .collect();
            position /= 2;
        }
        Ok(Self {
            epoch,
            index,
            receipt: receipts[index as usize].clone(),
            siblings,
        })
    }
}
pub fn frontier(
    program: &Pubkey,
    bucket: &Pubkey,
    epoch: i64,
    receipts: &[Receipt],
) -> Result<[[u8; 32]; DEPTH]> {
    ensure(receipts.len() < 65536, "receipt bucket capacity")?;
    let mut out = [[0; 32]; DEPTH];
    for (count, receipt) in receipts.iter().enumerate() {
        ensure(receipt.charge > 0, "positive receipt charge")?;
        let mut digest = leaf(program, bucket, epoch, count as u64, receipt);
        let mut height = 0;
        while count & (1 << height) != 0 {
            digest = node(&out[height], &digest);
            height += 1;
        }
        out[height] = digest;
    }
    Ok(out)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedReceipt {
    pub bucket: Pubkey,
    pub epoch: i64,
    pub index: u64,
    pub receipt: Receipt,
    pub usage: Usage,
}
impl PublishedReceipt {
    pub fn from_claim(program: &Pubkey, ix: &Instruction) -> Result<Self> {
        ensure(
            ix.program_id == *program
                && ix.data.len() == 157
                && ix.data[0] == 7
                && ix.accounts.len() == 12,
            "published claim instruction",
        )?;
        let p = &ix.data[1..];
        let receipt = Receipt::decode(&p[36..])?;
        ensure(
            receipt.generation == n64(p, 12)
                && receipt.booking == ix.accounts[4].pubkey
                && receipt.user == ix.accounts[3].pubkey,
            "published receipt bindings",
        )?;
        Ok(Self {
            bucket: ix.accounts[11].pubkey,
            epoch: i64le(p, 20),
            index: n64(p, 28),
            receipt,
            usage: Usage {
                uncached: crate::accounts::n32(p, 0),
                cached: crate::accounts::n32(p, 4),
                output: crate::accounts::n32(p, 8),
            },
        })
    }
}
