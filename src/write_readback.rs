//! Write-readback diagnostic (no reset between write and read).
//!
//! Writes `pattern[i] = addr_i` to every word in a RAM region, then
//! reads it back *immediately* — no `reset_and_halt` in between. This
//! isolates the bus topology from anything the ROM does. The expected
//! result is "every word equals its address". The failure modes we
//! care about are:
//!
//! - **Unmapped**: a region of the declared SRAM has no physical RAM
//!   behind it and reads back as a constant (0x00000000 or
//!   0xFFFFFFFF).
//! - **Aliased**: writes to address `X` actually land at a different
//!   address `Y` (typically because the address decoder ignores some
//!   high bits, mirroring a smaller real RAM across a larger window).
//!   We detect this by looking, in every non-Safe block, for words
//!   whose value is a *plausible* "addr-as-data" from somewhere else
//!   in the surveyed range. If more than half of the words in a
//!   block look like such mirrored addresses, we flag the block as
//!   ALIASED and report the apparent source window.
//!
//! Once half a block is mirroring elsewhere it's almost certain that
//! real RAM ends before this address.

use color_eyre::eyre::Result;
use comfy_table::Cell;
use owo_colors::OwoColorize;
use probe_rs::Session;

use crate::classify::{Class, classify_all};
use crate::format::human_bytes;
use crate::io::{read_words, write_words};
use crate::render::{header_cell, make_table, step};
use crate::survey::render_heatmap;

pub fn write_readback_test(s: &mut Session, start: u32, end: u32, block: u32) -> Result<()> {
    assert!(end > start, "end must be > start");
    assert!(
        (end - start).is_multiple_of(block),
        "(end - start) must be a multiple of block"
    );
    assert!(block.is_multiple_of(4), "block must be a multiple of 4");

    let total_bytes = (end - start) as usize;
    let n_words = total_bytes / 4;
    let words_per_block = (block / 4) as usize;
    let n_blocks = total_bytes / block as usize;

    let mut pattern = vec![0u32; n_words];
    for (i, w) in pattern.iter_mut().enumerate() {
        *w = start + (i as u32) * 4;
    }

    step(&format!(
        "writing {} of addr-as-data ({} blocks of {})",
        human_bytes((n_words * 4) as u64),
        n_blocks,
        human_bytes(block as u64),
    ));
    write_words(s, start, &pattern)?;

    step("reading back immediately (no reset)");
    let readback = read_words(s, start, n_words)?;

    let blocks = classify_all(start, block, n_blocks, words_per_block, &readback, &pattern);
    render_heatmap(start, block, &blocks);

    let alias_rows = find_aliasing(start, end, block, &readback, &blocks);

    if alias_rows.is_empty() {
        let any_bad = blocks.iter().any(|b| !matches!(b.class, Class::Safe));
        println!();
        if any_bad {
            println!(
                "  {} no aliasing pattern found in non-Safe blocks. Bad blocks read as 0/FF/const,",
                "NOT ALIASED:".yellow().bold()
            );
            println!("  which usually means the address is unmapped (no SRAM physically present)");
            println!("  rather than mirrored elsewhere.");
        } else {
            println!(
                "  {} every block read back exactly what was written. Real RAM matches the surveyed range.",
                "ALL MATCH:".green().bold()
            );
        }
    } else {
        println!();
        println!(
            "  {} {} block(s) read back values that look like addr-as-data from a DIFFERENT region:",
            "ALIASING SUSPECTED:".magenta().bold(),
            alias_rows.len()
        );
        let mut t = make_table();
        t.set_header(vec![
            header_cell("Block (addr we wrote)"),
            header_cell("Aliased words"),
            header_cell("Apparent source window"),
            header_cell("Likely mirror of"),
        ]);
        for row in &alias_rows {
            let off = row.src_min.wrapping_sub(row.block_start);
            t.add_row(vec![
                Cell::new(format!("0x{:08X}", row.block_start)),
                Cell::new(format!("{}/{}", row.n_alias, words_per_block)),
                Cell::new(format!("0x{:08X}..0x{:08X}", row.src_min, row.src_max)),
                Cell::new(format!("0x{:08X} (offset {off:+#010X})", row.src_min)),
            ]);
        }
        println!("{t}");
        println!(
            "  {} an alias means: write to 0x{:08X} actually landed at the 'source' address.",
            "Interpretation:".cyan().bold(),
            alias_rows[0].block_start,
        );
        println!(
            "  Either real RAM ends before 0x{:08X} and addresses wrap, or this region is",
            alias_rows[0].block_start,
        );
        println!("  power-gated and the bus mirror returns whatever lives at the gated input.");
    }

    Ok(())
}

/// One row of the alias report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AliasRow {
    pub block_start: u32,
    pub n_alias: u32,
    pub src_min: u32,
    pub src_max: u32,
}

/// Pure helper: walk every non-Safe block and report blocks whose
/// readback looks like at least 50% addr-as-data values pointing
/// somewhere *else* inside `[start, end)`.
///
/// The 50% threshold is intentionally lenient — partial mirroring
/// (some words latched, some not) is still strong evidence and we
/// don't want to miss it.
pub fn find_aliasing(
    start: u32,
    end: u32,
    block: u32,
    readback: &[u32],
    blocks: &[crate::classify::BlockResult],
) -> Vec<AliasRow> {
    let words_per_block = (block / 4) as usize;
    let mut out = Vec::new();
    for (bi, b) in blocks.iter().enumerate() {
        if matches!(b.class, Class::Safe) {
            continue;
        }
        let words = &readback[bi * words_per_block..(bi + 1) * words_per_block];
        let block_start = start + (bi as u32) * block;
        let mut n_alias = 0u32;
        let mut src_min = u32::MAX;
        let mut src_max = 0u32;
        for (wi, &w) in words.iter().enumerate() {
            if w >= start && w < end && w.is_multiple_of(4) {
                let here = block_start + (wi as u32) * 4;
                if w != here {
                    n_alias += 1;
                    src_min = src_min.min(w);
                    src_max = src_max.max(w);
                }
            }
        }
        if n_alias as usize >= words_per_block / 2 {
            out.push(AliasRow {
                block_start,
                n_alias,
                src_min,
                src_max,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::BlockResult;

    fn block(addr: u32, class: Class) -> BlockResult {
        BlockResult {
            addr,
            class,
            first_diff: None,
        }
    }

    #[test]
    fn no_aliasing_when_all_safe() {
        let blocks = vec![
            block(0x2000_0000, Class::Safe),
            block(0x2000_1000, Class::Safe),
        ];
        let readback = vec![0u32; 8 * 2]; // contents irrelevant for Safe blocks
        let rows = find_aliasing(0x2000_0000, 0x2000_2000, 0x1000, &readback, &blocks);
        assert!(rows.is_empty());
    }

    #[test]
    fn aliasing_detected_when_block_mirrors_another_region() {
        // 4 words per block, 2 blocks (so block bytes = 16).
        let start = 0x2000_0000u32;
        let block_bytes = 16u32;
        let end = start + 2 * block_bytes;
        // Block 0 looks Safe (own addresses).
        // Block 1 (`Changed`) contains addresses *from block 0*.
        let mut rb = vec![0u32; 8];
        for i in 0..4u32 {
            rb[i as usize] = start + i * 4; // block 0: identity (Safe)
            rb[4 + i as usize] = start + i * 4; // block 1: aliased to block 0
        }
        let blocks = vec![
            block(start, Class::Safe),
            block(start + block_bytes, Class::Changed),
        ];
        let rows = find_aliasing(start, end, block_bytes, &rb, &blocks);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.block_start, start + block_bytes);
        assert_eq!(r.n_alias, 4);
        assert_eq!(r.src_min, start);
        assert_eq!(r.src_max, start + 12);
    }

    #[test]
    fn below_threshold_not_reported() {
        // Only 1 of 4 words is plausibly an alias from elsewhere.
        let start = 0x2000_0000u32;
        let block_bytes = 16u32;
        let end = start + 2 * block_bytes;
        let mut rb = vec![0u32; 8];
        rb[4] = start; // block 1 word 0 mirrors block 0 word 0
        let blocks = vec![
            block(start, Class::Safe),
            block(start + block_bytes, Class::Changed),
        ];
        let rows = find_aliasing(start, end, block_bytes, &rb, &blocks);
        assert!(rows.is_empty(), "1/4 < 2/4 threshold");
    }
}
