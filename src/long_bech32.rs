//! bech32 and bech32m without BIP-173's length limit.
//!
//! LNURLs (LUD-01) and LUD-25's `ck1`, `cw1`, `cs1` and `cx1` all run past the
//! 90 characters BIP-173 allows, and a `cw1` or a long LNURL can run past the
//! 1023 the `bech32` crate checks a code against since 0.10. These are its
//! own checksums with that limit lifted, which is how bech32 0.9 encoded and
//! decoded them. Past 1023 characters the checksum no longer guarantees it
//! catches every 4-character error, only that a random string fails it with
//! odds of about 2^-30; callers cap the input length themselves.
//!
//! Decoding also enforces what bech32 0.9's `FromBase32` did: no more than
//! four padding bits, and all of them zero.

use bech32::primitives::decode::CheckedHrpstring;
use bech32::{Checksum, Fe1024, Fe32, Hrp};

// The generator from Bitcoin Core's src/bech32.cpp, as bech32 itself uses.
const GEN: [u32; 5] = [
    0x3b6a_57b2,
    0x2650_8e6d,
    0x1ea1_19fa,
    0x3d42_33dd,
    0x2a14_62b3,
];

/// BIP-173's checksum, what an LNURL carries.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) enum Bech32 {}

impl Checksum for Bech32 {
    type MidstateRepr = u32;
    type CorrectionField = Fe1024;
    const ROOT_GENERATOR: Self::CorrectionField = Fe1024::new([Fe32::P, Fe32::X]);
    const ROOT_EXPONENTS: core::ops::RangeInclusive<usize> = 24..=26;
    const CODE_LENGTH: usize = usize::MAX;
    const CHECKSUM_LENGTH: usize = 6;
    const GENERATOR_SH: [u32; 5] = GEN;
    const TARGET_RESIDUE: u32 = 1;
}

/// BIP-350's checksum, what LUD-25's strings carry.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub(crate) enum Bech32m {}

impl Checksum for Bech32m {
    type MidstateRepr = u32;
    type CorrectionField = Fe1024;
    const ROOT_GENERATOR: Self::CorrectionField = Fe1024::new([Fe32::P, Fe32::X]);
    const ROOT_EXPONENTS: core::ops::RangeInclusive<usize> = 24..=26;
    const CODE_LENGTH: usize = usize::MAX;
    const CHECKSUM_LENGTH: usize = 6;
    const GENERATOR_SH: [u32; 5] = GEN;
    const TARGET_RESIDUE: u32 = 0x2bc8_30a3;
}

/// Lowercase. `None` only for an `hrp` bech32 cannot carry.
pub(crate) fn encode<Ck: Checksum>(hrp: &str, bytes: &[u8]) -> Option<String> {
    bech32::encode::<Ck>(Hrp::parse(hrp).ok()?, bytes).ok()
}

/// The payload of a `Ck` string whose prefix is `hrp`, either all lowercase or
/// all uppercase. Never panics.
pub(crate) fn decode<Ck: Checksum>(hrp: &str, value: &str) -> Option<Vec<u8>> {
    let checked = CheckedHrpstring::new::<Ck>(value).ok()?;
    if checked.hrp().to_lowercase() != hrp {
        return None;
    }
    checked.validate_segwit_padding().ok()?;
    Some(checked.byte_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_constants_are_consistent() {
        Bech32::sanity_check();
        Bech32m::sanity_check();
    }

    #[test]
    fn they_agree_with_the_bounded_checksums_below_the_limit() {
        let hrp = Hrp::parse("lnurl").expect("valid");
        let bytes = b"https://mint.example/w?k1=00";
        assert_eq!(
            encode::<Bech32>("lnurl", bytes),
            bech32::encode::<bech32::Bech32>(hrp, bytes).ok()
        );
        assert_eq!(
            encode::<Bech32m>("lnurl", bytes),
            bech32::encode::<bech32::Bech32m>(hrp, bytes).ok()
        );
    }

    #[test]
    fn past_the_bounded_limit_round_trips() {
        let bytes = vec![0x5a; 2000];
        let encoded = encode::<Bech32m>("cw", &bytes).expect("encodes");
        assert!(encoded.len() > 1023);
        assert_eq!(decode::<Bech32m>("cw", &encoded), Some(bytes.clone()));
        assert_eq!(
            decode::<Bech32m>("cw", &encoded.to_ascii_uppercase()),
            Some(bytes)
        );
        assert_eq!(decode::<Bech32>("cw", &encoded), None);
        assert_eq!(decode::<Bech32m>("cp", &encoded), None);
    }
}
