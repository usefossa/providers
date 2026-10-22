use crate::{Error, Result, ensure};
use serde::{Deserialize, Serialize};

pub const UNIT: u64 = 1_000_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rates {
    pub input: u32,
    pub cached: u32,
    pub output: u32,
}

impl Rates {
    pub fn validate(self) -> Result<()> {
        ensure(
            self.input > 0 && self.output > 0 && self.cached <= self.input,
            "offer rates",
        )
    }
    pub fn charge(self, usage: Usage) -> u64 {
        fee(usage.uncached, self.input)
            + fee(usage.cached, self.cached)
            + fee(usage.output, self.output)
    }
    pub fn reserve(self, max_input: u32, max_output: u32) -> u64 {
        fee(max_input, self.input) + fee(max_output, self.output) + 1
    }
    pub(crate) fn encode(self, out: &mut Vec<u8>) {
        for value in [self.input, self.cached, self.output] {
            out.extend(value.to_le_bytes());
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub uncached: u32,
    pub cached: u32,
    pub output: u32,
}
impl Usage {
    pub(crate) fn encode(self, out: &mut Vec<u8>) {
        for value in [self.uncached, self.cached, self.output] {
            out.extend(value.to_le_bytes());
        }
    }
}

pub fn fee(tokens: u32, rate: u32) -> u64 {
    (u64::from(tokens) * u64::from(rate)).div_ceil(UNIT)
}
pub fn collateral(exposure: u64) -> Result<u64> {
    exposure
        .checked_add(exposure.div_ceil(100))
        .ok_or(Error::Arithmetic)
}
pub fn parse_atomic(value: &str) -> Result<u64> {
    ensure(
        !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit()),
        "atomic amount",
    )?;
    value.parse().map_err(|_| Error::Arithmetic)
}
pub fn parse_usdc(value: &str) -> Result<u64> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    ensure(
        fraction.len() <= 6 && fraction.bytes().all(|c| c.is_ascii_digit()),
        "USDC decimals",
    )?;
    let whole = parse_atomic(whole)?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        parse_atomic(fraction)? * 10u64.pow(6 - fraction.len() as u32)
    };
    whole
        .checked_mul(UNIT)
        .and_then(|n| n.checked_add(fraction))
        .ok_or(Error::Arithmetic)
}
pub fn format_usdc(atomic: u64) -> String {
    format!("{}.{:06}", atomic / UNIT, atomic % UNIT)
}
