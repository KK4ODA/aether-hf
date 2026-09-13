//! Cyclic redundancy checks on MSB-first bit slices, using the public 3GPP polynomials.
//!
//! TS 38.212 §5.1. Initial register zero, no final XOR, no reflection — the parity bits are
//! the remainder of `a(x)·x^L` divided by `g(x)`. This is a direct port of the reference
//! model's `fec/crc.py` and is checked bit-for-bit against it.

/// A CRC specified by its generator polynomial and width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Crc {
    /// Name as it appears in TS 38.212 §5.1.
    pub name: &'static str,
    /// Generator polynomial without the leading `x^L` term.
    pub poly: u32,
    /// Number of parity bits.
    pub width: u32,
}

/// TS 38.212 §5.1 CRC24A — the payload CRC of an Aether frame.
pub const CRC24A: Crc = Crc {
    name: "CRC24A",
    poly: 0x0086_4CFB,
    width: 24,
};
/// TS 38.212 §5.1 CRC24B.
pub const CRC24B: Crc = Crc {
    name: "CRC24B",
    poly: 0x0080_0063,
    width: 24,
};
/// TS 38.212 §5.1 CRC24C.
pub const CRC24C: Crc = Crc {
    name: "CRC24C",
    poly: 0x00B2_B117,
    width: 24,
};
/// TS 38.212 §5.1 CRC16.
pub const CRC16: Crc = Crc {
    name: "CRC16",
    poly: 0x0000_1021,
    width: 16,
};
/// TS 38.212 §5.1 CRC11.
pub const CRC11: Crc = Crc {
    name: "CRC11",
    poly: 0x0000_0621,
    width: 11,
};
/// TS 38.212 §5.1 CRC6.
pub const CRC6: Crc = Crc {
    name: "CRC6",
    poly: 0x0000_0021,
    width: 6,
};

impl Crc {
    /// Parity bits for `bits`, most significant first.
    #[must_use]
    pub fn remainder(&self, bits: &[u8]) -> Vec<u8> {
        let top = 1u32 << self.width;
        let mask = top - 1;
        let mut reg = 0u32;
        for &bit in bits {
            reg = ((reg << 1) | u32::from(bit & 1)) & (mask | top);
            if reg & top != 0 {
                reg = (reg ^ top) ^ self.poly;
            }
        }
        for _ in 0..self.width {
            reg = (reg << 1) & (mask | top);
            if reg & top != 0 {
                reg = (reg ^ top) ^ self.poly;
            }
        }
        reg &= mask;
        (0..self.width)
            .map(|i| ((reg >> (self.width - 1 - i)) & 1) as u8)
            .collect()
    }

    /// `bits` followed by its parity bits.
    #[must_use]
    pub fn attach(&self, bits: &[u8]) -> Vec<u8> {
        let mut out = bits.to_vec();
        out.extend_from_slice(&self.remainder(bits));
        out
    }

    /// Whether the trailing `width` bits are the correct parity for what precedes them.
    #[must_use]
    pub fn check(&self, bits_with_crc: &[u8]) -> bool {
        let width = self.width as usize;
        if bits_with_crc.len() < width {
            return false;
        }
        let split = bits_with_crc.len() - width;
        self.remainder(&bits_with_crc[..split]) == bits_with_crc[split..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attach_then_check_round_trips() {
        let bits: Vec<u8> = (0..200).map(|i| u8::from(i % 3 == 0)).collect();
        for crc in [CRC24A, CRC24B, CRC24C, CRC16, CRC11, CRC6] {
            let with_crc = crc.attach(&bits);
            assert_eq!(
                with_crc.len(),
                bits.len() + crc.width as usize,
                "{}",
                crc.name
            );
            assert!(crc.check(&with_crc), "{}", crc.name);
        }
    }

    #[test]
    fn a_single_flipped_bit_is_caught() {
        let bits: Vec<u8> = (0..120).map(|i| u8::from(i % 7 < 3)).collect();
        let mut with_crc = CRC24A.attach(&bits);
        for position in [0, 1, 57, 119] {
            with_crc[position] ^= 1;
            assert!(
                !CRC24A.check(&with_crc),
                "flip at {position} went undetected"
            );
            with_crc[position] ^= 1;
        }
        assert!(CRC24A.check(&with_crc));
    }

    #[test]
    fn empty_input_still_has_a_defined_remainder() {
        assert_eq!(CRC24A.remainder(&[]).len(), 24);
        assert!(CRC24A.check(&CRC24A.attach(&[])));
    }

    #[test]
    fn too_short_an_input_fails_rather_than_panicking() {
        assert!(!CRC24A.check(&[0, 1, 0]));
    }
}
