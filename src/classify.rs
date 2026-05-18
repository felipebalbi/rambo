//! Per-block classification of survey readbacks.
//!
//! The survey writes a known pattern (typically `pattern[i] == addr_i`,
//! i.e. "address as data") to every word in a RAM region, then resets
//! the chip so the boot ROM runs but no user code, and reads the region
//! back. Each `block`-sized chunk is then classified as one of:
//!
//! - [`Class::Safe`] — every word in the block still equals what we
//!   wrote. Nothing touched it between the write and the read.
//! - [`Class::Zero`] — every word reads back as `0x00000000`. Either
//!   the ROM scrubbed it or the SRAM partition is undriven and pulled
//!   low.
//! - [`Class::Ones`] — every word reads back as `0xFFFFFFFF`. Usually
//!   indicates an undriven / floating partition that pulls high (or a
//!   bus that returns all-ones for unmapped addresses).
//! - [`Class::Changed`] — none of the above; the block was modified in
//!   some other way (e.g. trampolines, stacks, RAM-resident ROM code).

/// Block classification.
///
/// The discriminants are explicit so they can also be used as `usize`
/// indices into small fixed-size count arrays (`[usize; 4]`).
#[repr(usize)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Safe = 0,
    Zero = 1,
    Ones = 2,
    Changed = 3,
}

/// Summary of a single classified block.
pub struct BlockResult {
    /// Byte address of the first word in the block.
    pub addr: u32,
    pub class: Class,
    /// First word whose readback differed from the expected value, as
    /// `(word_addr, expected, got)`. `None` when the block is `Safe`
    /// or its mismatch isn't a single word (e.g. `Zero` / `Ones`).
    pub first_diff: Option<(u32, u32, u32)>,
}

/// Classify a single block of words against the expected pattern.
///
/// - `block_start` is the byte address of `words[0]`.
/// - `words` and `expected` must be the same length.
///
/// Returns the [`Class`] plus, for `Changed` blocks, the first
/// `(addr, expected, got)` mismatch found (handy for printing a
/// "sample" diff in the runs table).
pub fn classify_block(
    block_start: u32,
    words: &[u32],
    expected: &[u32],
) -> (Class, Option<(u32, u32, u32)>) {
    debug_assert_eq!(words.len(), expected.len());

    let mut all_match = true;
    let mut all_zero = true;
    let mut all_ones = true;
    let mut first_diff: Option<(u32, u32, u32)> = None;

    for (i, (&w, &exp)) in words.iter().zip(expected.iter()).enumerate() {
        let addr = block_start + (i as u32) * 4;
        if w != exp {
            all_match = false;
            if first_diff.is_none() {
                first_diff = Some((addr, exp, w));
            }
        }
        if w != 0 {
            all_zero = false;
        }
        if w != 0xFFFF_FFFF {
            all_ones = false;
        }
    }

    let class = if all_match {
        Class::Safe
    } else if all_zero {
        Class::Zero
    } else if all_ones {
        Class::Ones
    } else {
        Class::Changed
    };
    (class, first_diff)
}

/// Classify every `block`-sized chunk in a region.
///
/// Assumes `readback.len() == pattern.len() == n_blocks * words_per_block`.
pub fn classify_all(
    start: u32,
    block: u32,
    n_blocks: usize,
    words_per_block: usize,
    readback: &[u32],
    pattern: &[u32],
) -> Vec<BlockResult> {
    let mut blocks = Vec::with_capacity(n_blocks);
    for b in 0..n_blocks {
        let block_start = start + (b as u32) * block;
        let range = b * words_per_block..(b + 1) * words_per_block;
        let (class, first_diff) =
            classify_block(block_start, &readback[range.clone()], &pattern[range]);
        blocks.push(BlockResult {
            addr: block_start,
            class,
            first_diff,
        });
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr_pattern(start: u32, n: usize) -> Vec<u32> {
        (0..n as u32).map(|i| start + i * 4).collect()
    }

    #[test]
    fn safe_when_readback_matches_pattern() {
        let p = addr_pattern(0x2000_0000, 4);
        let (c, d) = classify_block(0x2000_0000, &p, &p);
        assert_eq!(c, Class::Safe);
        assert!(d.is_none());
    }

    #[test]
    fn zero_when_all_zero() {
        let p = addr_pattern(0x2000_0000, 4);
        let r = vec![0u32; 4];
        let (c, _) = classify_block(0x2000_0000, &r, &p);
        assert_eq!(c, Class::Zero);
    }

    #[test]
    fn ones_when_all_ones() {
        let p = addr_pattern(0x2000_0000, 4);
        let r = vec![0xFFFF_FFFFu32; 4];
        let (c, _) = classify_block(0x2000_0000, &r, &p);
        assert_eq!(c, Class::Ones);
    }

    #[test]
    fn changed_when_partial_with_first_diff() {
        let p = addr_pattern(0x2000_0000, 4);
        let mut r = p.clone();
        r[2] = 0xDEAD_BEEF;
        let (c, d) = classify_block(0x2000_0000, &r, &p);
        assert_eq!(c, Class::Changed);
        assert_eq!(d, Some((0x2000_0008, 0x2000_0008, 0xDEAD_BEEF)));
    }

    #[test]
    fn classify_all_splits_by_block() {
        // 2 blocks of 4 words each.
        let pattern = addr_pattern(0x2000_0000, 8);
        let mut readback = pattern.clone();
        // Block 0 is SAFE, block 1 is ZERO.
        for w in &mut readback[4..] {
            *w = 0;
        }
        let res = classify_all(0x2000_0000, 16, 2, 4, &readback, &pattern);
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].class, Class::Safe);
        assert_eq!(res[1].class, Class::Zero);
        assert_eq!(res[0].addr, 0x2000_0000);
        assert_eq!(res[1].addr, 0x2000_0010);
    }
}
