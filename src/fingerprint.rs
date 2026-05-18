//! Per-block fingerprinting of CHANGED survey blocks.
//!
//! For each block that the survey classified as `CHANGED` we try a few
//! cheap heuristics to describe *how* it was changed. The goal isn't
//! formal reverse engineering — just enough signal to recognize the
//! common ROM artifacts at a glance:
//!
//! - **Constant** value across the whole block.
//! - **Dominant** value (≥ 90 % of words).
//! - **`addr + k` aliasing**: every word equals its address plus the
//!   same fixed offset (often a copy of a different region).
//! - **Repeating motif** of period 1..=4 (e.g. a 2-word interrupt
//!   trampoline replicated through the block).
//! - **Noise / floating cells**: nothing above matched, so we look at
//!   the bit-1 density. A density near 50 % is the telltale signature
//!   of undriven SRAM; anything else is just "partial random".

use std::collections::HashMap;

use comfy_table::{Attribute, Cell};
use owo_colors::OwoColorize;

use crate::render::{header_cell, make_table};

/// Maximum number of CHANGED blocks displayed in the fingerprint table.
const ROW_LIMIT: usize = 32;

/// Threshold (percent) above which a single value is reported as
/// "dominant" rather than just "frequent".
const DOMINANT_PCT: usize = 90;

pub fn fingerprint_changed(start: u32, end: u32, block: u32, readback: &[u32]) {
    let words_per_block = (block / 4) as usize;
    let n_blocks = ((end - start) / block) as usize;

    let mut t = make_table();
    t.set_header(vec![
        header_cell("Block"),
        header_cell("Bit-1 %"),
        header_cell("Top values (count)"),
        header_cell("Pattern"),
        header_cell("Verdict"),
    ]);

    let mut shown = 0usize;
    let mut total_changed = 0usize;

    for b in 0..n_blocks {
        let block_start = start + (b as u32) * block;
        let words = &readback[b * words_per_block..(b + 1) * words_per_block];

        // Quickly skip blocks that match one of the survey classes
        // (those are reported elsewhere and aren't "changed" in the
        // sense the fingerprint cares about).
        let all_zero = words.iter().all(|&w| w == 0);
        let all_ones = words.iter().all(|&w| w == 0xFFFF_FFFF);
        let all_addr = words
            .iter()
            .enumerate()
            .all(|(i, &w)| w == block_start + (i as u32) * 4);
        let all_naddr = words
            .iter()
            .enumerate()
            .all(|(i, &w)| w == !(block_start + (i as u32) * 4));
        if all_zero || all_ones || all_addr || all_naddr {
            continue;
        }
        total_changed += 1;
        if shown >= ROW_LIMIT {
            continue;
        }

        let bits_set: u32 = words.iter().map(|w| w.count_ones()).sum();
        let bit1_pct = 100.0 * bits_set as f64 / (words.len() * 32) as f64;

        let top = top_values(words);
        let top_str = top
            .iter()
            .take(3)
            .map(|(v, c)| format!("0x{v:08X}({c})"))
            .collect::<Vec<_>>()
            .join("  ");

        let (pattern, verdict) = detect_pattern(block_start, words, &top);

        t.add_row(vec![
            Cell::new(format!("0x{block_start:08X}")),
            Cell::new(format!("{bit1_pct:5.1}")),
            Cell::new(top_str),
            Cell::new(pattern),
            verdict,
        ]);
        shown += 1;
    }

    if total_changed == 0 {
        println!("  {}", "No CHANGED blocks to fingerprint.".dimmed());
        return;
    }

    println!("{t}");
    if total_changed > shown {
        println!(
            "  {}",
            format!("... {} more CHANGED blocks omitted", total_changed - shown).dimmed()
        );
    }
}

/// Count how often each value appears in `words`, sorted descending by count.
fn top_values(words: &[u32]) -> Vec<(u32, u32)> {
    let mut counts: HashMap<u32, u32> = HashMap::new();
    for &w in words {
        *counts.entry(w).or_insert(0) += 1;
    }
    let mut top: Vec<(u32, u32)> = counts.into_iter().collect();
    top.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
    top
}

/// Describe the structure of a single CHANGED block and emit a
/// colored verdict cell.
///
/// Returned tuple: `(human-readable description, verdict cell)`.
pub fn detect_pattern(block_start: u32, words: &[u32], top: &[(u32, u32)]) -> (String, Cell) {
    let n = words.len();

    // 1. Constant value or dominant value.
    if let Some(&(v, c)) = top.first() {
        if c as usize == n {
            return (
                format!("constant 0x{v:08X}"),
                Cell::new("PATTERNED")
                    .fg(comfy_table::Color::Magenta)
                    .add_attribute(Attribute::Bold),
            );
        }
        if (c as usize) * 100 / n >= DOMINANT_PCT {
            return (
                format!("dominant 0x{v:08X} ({c}/{n})"),
                Cell::new("PATTERNED")
                    .fg(comfy_table::Color::Magenta)
                    .add_attribute(Attribute::Bold),
            );
        }
    }

    // 2. addr + constant offset.
    let off0 = words[0].wrapping_sub(block_start);
    if words
        .iter()
        .enumerate()
        .all(|(i, &w)| w.wrapping_sub(block_start + (i as u32) * 4) == off0)
    {
        return (
            format!("addr + 0x{off0:08X}"),
            Cell::new("ALIASED")
                .fg(comfy_table::Color::Cyan)
                .add_attribute(Attribute::Bold),
        );
    }

    // 3. Repeating motif of period p in 1..=4.
    for p in 1..=4usize {
        if !n.is_multiple_of(p) {
            continue;
        }
        let ok = (p..n).all(|i| words[i] == words[i % p]);
        if ok {
            let motif = (0..p)
                .map(|i| format!("0x{:08X}", words[i]))
                .collect::<Vec<_>>()
                .join(",");
            return (
                format!("motif×{p} [{motif}]"),
                Cell::new("PATTERNED")
                    .fg(comfy_table::Color::Magenta)
                    .add_attribute(Attribute::Bold),
            );
        }
    }

    // 4. Fall through: noise. ~50% bit density => almost certainly
    // floating cells; anything else is just partial random.
    let bits_set: u32 = words.iter().map(|w| w.count_ones()).sum();
    let pct = 100.0 * bits_set as f64 / (n * 32) as f64;
    let text = if (40.0..=60.0).contains(&pct) {
        "UNDRIVEN"
    } else {
        "PARTIAL_RANDOM"
    };
    (
        format!("noise (bit1={pct:.1}%)"),
        Cell::new(text)
            .fg(comfy_table::Color::Yellow)
            .add_attribute(Attribute::Bold),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Strip ANSI styling from `Cell` debug to read its `content` field
    /// for testing — comfy-table's `Cell` doesn't expose the inner
    /// string directly, but we can render it via a one-cell table.
    fn cell_text(c: &Cell) -> String {
        c.content().clone()
    }

    #[test]
    fn constant_value_detected() {
        let words = vec![0xCAFE_BABE_u32; 8];
        let top = top_values(&words);
        let (desc, v) = detect_pattern(0x2000_0000, &words, &top);
        assert!(desc.contains("constant"));
        assert_eq!(cell_text(&v), "PATTERNED");
    }

    #[test]
    fn dominant_value_detected() {
        let mut words = vec![0xCAFE_BABE_u32; 10];
        words[9] = 0xDEAD_BEEF;
        let top = top_values(&words);
        let (desc, v) = detect_pattern(0x2000_0000, &words, &top);
        assert!(desc.contains("dominant"));
        assert_eq!(cell_text(&v), "PATTERNED");
    }

    #[test]
    fn addr_plus_offset_detected() {
        let base = 0x2000_0000u32;
        let off = 0x100u32;
        let words: Vec<u32> = (0..4).map(|i| base + (i * 4) + off).collect();
        let top = top_values(&words);
        let (desc, v) = detect_pattern(base, &words, &top);
        assert!(desc.contains("addr + 0x00000100"));
        assert_eq!(cell_text(&v), "ALIASED");
    }

    #[test]
    fn motif_period_2_detected() {
        let words = vec![0x1111_1111, 0x2222_2222, 0x1111_1111, 0x2222_2222];
        let top = top_values(&words);
        let (desc, v) = detect_pattern(0x2000_0000, &words, &top);
        assert!(desc.contains("motif×2"));
        assert_eq!(cell_text(&v), "PATTERNED");
    }

    #[test]
    fn noise_with_balanced_bits_is_undriven() {
        // 5 distinct values, each with 16 bits set => exactly 50% bit
        // density. Five words is intentional: it can't be a motif of
        // period 1..=4 because 4 isn't a factor of 5, and no two
        // words are equal so period-1 fails too.
        let words = vec![
            0x5555_5555,
            0xAAAA_AAAA,
            0xCCCC_CCCC,
            0x3333_3333,
            0xF0F0_F0F0,
        ];
        let top = top_values(&words);
        let (desc, v) = detect_pattern(0x2000_0000, &words, &top);
        assert!(desc.starts_with("noise"), "got: {desc}");
        assert_eq!(cell_text(&v), "UNDRIVEN");
    }

    #[test]
    fn noise_with_skewed_bits_is_partial_random() {
        // 5 distinct sparse values: ~6% bit density.
        let words = vec![
            0x0000_0001,
            0x0000_0003,
            0x0001_0000,
            0x0300_0001,
            0x0000_0007,
        ];
        let top = top_values(&words);
        let (desc, v) = detect_pattern(0x2000_0000, &words, &top);
        assert!(desc.starts_with("noise"), "got: {desc}");
        assert_eq!(cell_text(&v), "PARTIAL_RANDOM");
    }
}
