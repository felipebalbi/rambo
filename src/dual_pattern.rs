//! Dual-pattern verdict.
//!
//! The survey is run twice in succession with two complementary
//! patterns:
//! - Pass A writes `pattern[i] = addr_i`, resets, reads back as `a`.
//! - Pass B writes `pattern[i] = !addr_i`, resets, reads back as `b`.
//!
//! For each block we then look at every word pair `(a[i], b[i])` and
//! classify the whole block:
//!
//! | per-word condition                              | classification |
//! |-------------------------------------------------|----------------|
//! | `a == addr && b == !addr`                       | PRESERVED      |
//! | `a == b` and not preserved                      | OVERWRITTEN    |
//! | neither                                         | NOISE          |
//!
//! A block is `PRESERVED` if every word is preserved, `OVERWRITTEN`
//! if every word is overwritten, `PARTIAL` if it contains a mix of
//! overwritten and noise words, and `NOISE` otherwise.
//!
//! Why it matters: a deterministic ROM action produces the same value
//! both passes (so we see `OVERWRITTEN`), while undriven RAM reads
//! random/floating values that differ between passes (`NOISE`). The
//! plain survey cannot distinguish those — dual-pattern can.

use comfy_table::{Attribute, Cell};
use owo_colors::OwoColorize;

use crate::format::human_bytes;
use crate::heatmap::{self, CellColor};
use crate::render::{header_cell, make_table};

/// Per-block verdict produced by the dual-pattern comparison.
#[repr(usize)]
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Preserved = 0,
    Overwritten = 1,
    Noise = 2,
    Partial = 3,
}

/// Classify a single block from a dual-pattern run.
///
/// `block_start` is the byte address of `a[0]`/`b[0]`; the two slices
/// must have the same length.
pub fn classify_dual_block(block_start: u32, a: &[u32], b: &[u32]) -> Verdict {
    debug_assert_eq!(a.len(), b.len());

    let mut all_preserved = true;
    let mut all_overwritten = true;
    let mut any_overwritten = false;
    let mut any_noise = false;

    for (i, (&av, &bv)) in a.iter().zip(b.iter()).enumerate() {
        let addr = block_start + (i as u32) * 4;
        let preserved = av == addr && bv == !addr;
        let overwritten = av == bv && !preserved;
        let noise = !preserved && !overwritten;

        if !preserved {
            all_preserved = false;
        }
        if !overwritten {
            all_overwritten = false;
        }
        if overwritten {
            any_overwritten = true;
        }
        if noise {
            any_noise = true;
        }
    }

    if all_preserved {
        Verdict::Preserved
    } else if all_overwritten {
        Verdict::Overwritten
    } else if any_overwritten && any_noise {
        Verdict::Partial
    } else {
        Verdict::Noise
    }
}

/// Render the dual-pattern heatmap, per-class totals, and the final
/// English verdict.
pub fn print_dual_pattern_verdict(start: u32, end: u32, block: u32, a: &[u32], b: &[u32]) {
    let words_per_block = (block / 4) as usize;
    let n_blocks = ((end - start) / block) as usize;

    let mut verdicts = Vec::with_capacity(n_blocks);
    for bi in 0..n_blocks {
        let bs = start + (bi as u32) * block;
        let aw = &a[bi * words_per_block..(bi + 1) * words_per_block];
        let bw = &b[bi * words_per_block..(bi + 1) * words_per_block];
        verdicts.push(classify_dual_block(bs, aw, bw));
    }

    heatmap::render(
        "Dual-pattern verdict heatmap",
        "(each cell = 1 block)",
        start,
        block,
        n_blocks,
        |cell| match verdicts[cell] {
            Verdict::Preserved => CellColor::Green,
            Verdict::Overwritten => CellColor::Red,
            Verdict::Noise => CellColor::Magenta,
            Verdict::Partial => CellColor::Yellow,
        },
    );
    println!();
    heatmap::legend(&[
        (CellColor::Green, "preserved (writes survived)"),
        (CellColor::Red, "overwritten (deterministic ROM action)"),
        (CellColor::Magenta, "noise (floating cells)"),
        (CellColor::Yellow, "partial"),
    ]);

    let mut counts = [0usize; 4];
    for v in &verdicts {
        counts[*v as usize] += 1;
    }

    let mut t = make_table();
    t.set_header(vec![
        header_cell("Verdict"),
        header_cell("Blocks"),
        header_cell("Bytes"),
    ]);
    for (name, color, count) in [
        (
            "PRESERVED",
            comfy_table::Color::Green,
            counts[Verdict::Preserved as usize],
        ),
        (
            "OVERWRITTEN",
            comfy_table::Color::Red,
            counts[Verdict::Overwritten as usize],
        ),
        (
            "NOISE",
            comfy_table::Color::Magenta,
            counts[Verdict::Noise as usize],
        ),
        (
            "PARTIAL",
            comfy_table::Color::Yellow,
            counts[Verdict::Partial as usize],
        ),
    ] {
        t.add_row(vec![
            Cell::new(name).fg(color).add_attribute(Attribute::Bold),
            Cell::new(count.to_string()),
            Cell::new(human_bytes(count as u64 * block as u64)),
        ]);
    }
    println!("{t}");

    let noise_or_partial = counts[Verdict::Noise as usize] + counts[Verdict::Partial as usize];
    let overwritten = counts[Verdict::Overwritten as usize];
    println!();
    if noise_or_partial > overwritten && noise_or_partial > 0 {
        println!(
            "  {} most non-preserved blocks read differently between the two passes — strong evidence of {}.",
            "VERDICT:".red().bold(),
            "undriven RAM (power-gated partitions)".bold()
        );
    } else if overwritten > 0 && noise_or_partial == 0 {
        println!(
            "  {} non-preserved blocks read identically between both passes — strong evidence of {}.",
            "VERDICT:".yellow().bold(),
            "deterministic ROM overwrite".bold()
        );
    } else if overwritten > 0 && noise_or_partial > 0 {
        println!(
            "  {} mixed: {} blocks deterministic-overwrite, {} blocks floating/partial.",
            "VERDICT:".yellow().bold(),
            overwritten,
            noise_or_partial
        );
    } else {
        println!(
            "  {} all blocks were preserved — nothing to explain.",
            "VERDICT:".green().bold()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(addr: u32, n: usize) -> (Vec<u32>, Vec<u32>) {
        let a: Vec<u32> = (0..n as u32).map(|i| addr + i * 4).collect();
        let b: Vec<u32> = a.iter().map(|&w| !w).collect();
        (a, b)
    }

    #[test]
    fn preserved_when_both_patterns_intact() {
        let (a, b) = pair(0x2000_0000, 4);
        assert_eq!(classify_dual_block(0x2000_0000, &a, &b), Verdict::Preserved);
    }

    #[test]
    fn overwritten_when_both_reads_equal_but_wrong() {
        let a = vec![0u32; 4];
        let b = vec![0u32; 4];
        assert_eq!(
            classify_dual_block(0x2000_0000, &a, &b),
            Verdict::Overwritten
        );
    }

    #[test]
    fn noise_when_reads_differ_and_dont_match_pattern() {
        let a = vec![0xAAAA_AAAA, 0xBBBB_BBBB, 0xCCCC_CCCC, 0xDDDD_DDDD];
        let b = vec![0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444];
        assert_eq!(classify_dual_block(0x2000_0000, &a, &b), Verdict::Noise);
    }

    #[test]
    fn partial_mixes_overwritten_and_noise() {
        // 2 words preserved is impossible here; instead mix
        // overwritten (a==b) with noise (a!=b, neither matches).
        let a = vec![0u32, 0xAAAA_AAAA, 0u32, 0xCCCC_CCCC];
        let b = vec![0u32, 0xBBBB_BBBB, 0u32, 0xDDDD_DDDD];
        assert_eq!(classify_dual_block(0x2000_0000, &a, &b), Verdict::Partial);
    }
}
