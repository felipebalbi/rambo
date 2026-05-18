//! Full SRAM survey: write a known pattern, reset, classify the result.
//!
//! # Algorithm
//!
//! 1. Build a pattern such that `pattern[i] = start + 4*i` (so every
//!    word in RAM literally contains its own address). The
//!    `invert_pattern` knob produces `!(start + 4*i)` instead, which
//!    is used by the dual-pattern verdict to tell deterministic ROM
//!    overwrite (same value across two passes) from undriven RAM
//!    (different value each pass).
//! 2. Halt the core, bulk-write the pattern into the entire region in
//!    `CHUNK_WORDS`-sized SWD transactions.
//! 3. Issue `reset_and_halt`. The boot ROM runs, but no user code does
//!    — the only thing between our write and our read is the ROM.
//! 4. Bulk-read the region, classify each `block`-sized chunk via
//!    [`classify_all`], render a 1-KiB-cell heatmap, and print runs +
//!    totals.
//! 5. When `cycles > 1`, write once before step 3 and then repeat the
//!    reset/read/classify loop `cycles` times. The collected per-cycle
//!    class arrays feed `print_stability`, which highlights blocks
//!    whose verdict differs between resets.

use color_eyre::eyre::Result;
use comfy_table::{Cell, CellAlignment};
use owo_colors::OwoColorize;
use probe_rs::Session;

use crate::classify::{BlockResult, Class, classify_all};
use crate::format::human_bytes;
use crate::heatmap::{self, CellColor};
use crate::io::{read_words, reset_and_halt, write_words};
use crate::render::{class_cell, class_inline, header_cell, make_table, step};
use crate::stability::print_stability;

/// Result of one full survey pass on a region: the most recent
/// readback (needed by dual-pattern) and the per-block
/// classifications from that final readback.
pub struct SurveyResult {
    pub readback: Vec<u32>,
    pub blocks: Vec<BlockResult>,
}

/// Run a full SRAM survey on `[start, end)`.
pub fn full_sram_survey(
    s: &mut Session,
    start: u32,
    end: u32,
    block: u32,
    cycles: u32,
    invert_pattern: bool,
) -> Result<SurveyResult> {
    assert!(end > start, "end must be > start");
    assert!(
        (end - start).is_multiple_of(block),
        "(end - start) must be a multiple of block"
    );
    assert!(block.is_multiple_of(4), "block must be a multiple of 4");
    assert!(cycles >= 1, "cycles must be >= 1");

    let total_bytes = (end - start) as usize;
    let n_words = total_bytes / 4;
    let words_per_block = (block / 4) as usize;
    let n_blocks = total_bytes / block as usize;
    let pattern_label = if invert_pattern { "!addr" } else { "addr" };

    if cycles == 1 {
        step(&format!(
            "sweeping {} blocks of {} ({} total) with pattern '{}'",
            n_blocks,
            human_bytes(block as u64),
            human_bytes(total_bytes as u64),
            pattern_label,
        ));
    } else {
        step(&format!(
            "sweeping {} blocks of {} ({} total), {} reset cycles (write '{}' once, reset+read {} times)",
            n_blocks,
            human_bytes(block as u64),
            human_bytes(total_bytes as u64),
            cycles,
            pattern_label,
            cycles,
        ));
    }

    // Build the pattern: each word holds its own (possibly inverted) address.
    let mut pattern = vec![0u32; n_words];
    for (i, w) in pattern.iter_mut().enumerate() {
        let addr = start + (i as u32) * 4;
        *w = if invert_pattern { !addr } else { addr };
    }

    step(&format!(
        "writing {} of '{}'-as-data",
        human_bytes((n_words * 4) as u64),
        pattern_label,
    ));
    write_words(s, start, &pattern)?;

    let mut per_cycle_classes: Vec<Vec<Class>> = Vec::with_capacity(cycles as usize);
    let mut last_blocks: Vec<BlockResult> = Vec::new();
    let mut last_readback: Vec<u32> = Vec::new();

    for cycle in 1..=cycles {
        if cycles > 1 && !crate::render::is_quiet() {
            println!();
            println!(
                "{} {}",
                format!("Reset cycle {cycle}/{cycles}").bold().cyan(),
                "(write was only done before cycle 1)".dimmed()
            );
        }

        step("reset_and_halt (post-ROM, pre-user-code)");
        reset_and_halt(s)?;

        let readback = read_words(s, start, n_words)?;
        let blocks = classify_all(start, block, n_blocks, words_per_block, &readback, &pattern);
        render_heatmap(start, block, &blocks);

        if cycles > 1 && !crate::render::is_quiet() {
            let mut counts = [0usize; 4];
            for b in &blocks {
                counts[b.class as usize] += 1;
            }
            println!(
                "  {} {} | {} {} | {} {} | {} {}",
                class_inline(Class::Safe),
                counts[Class::Safe as usize],
                class_inline(Class::Zero),
                counts[Class::Zero as usize],
                class_inline(Class::Ones),
                counts[Class::Ones as usize],
                class_inline(Class::Changed),
                counts[Class::Changed as usize],
            );
        }

        per_cycle_classes.push(blocks.iter().map(|b| b.class).collect());
        last_blocks = blocks;
        last_readback = readback;
    }

    if cycles == 1 {
        print_runs_and_totals(&last_blocks, end, block);
    } else {
        print_stability(start, end, block, &per_cycle_classes);
    }

    Ok(SurveyResult {
        readback: last_readback,
        blocks: last_blocks,
    })
}

/// Survey heatmap: one cell per user-block, colored by its [`Class`].
///
/// Reuses the already-classified `blocks` slice so block-level state
/// (ZERO, ONES, CHANGED, SAFE) is preserved verbatim in the heatmap.
/// Cell size always equals `block`, matching the runs/totals table
/// granularity below.
pub fn render_heatmap(start: u32, block: u32, blocks: &[BlockResult]) {
    let n_cells = blocks.len();
    let subtitle = format!("(each cell = {})", human_bytes(block as u64));

    heatmap::render("Heatmap", &subtitle, start, block, n_cells, |cell| {
        crate::render::class_color(blocks[cell].class)
    });

    println!();
    heatmap::legend(&[
        (CellColor::Green, "SAFE (unmodified)"),
        (CellColor::Blue, "ZERO (scrubbed to 0x00)"),
        (CellColor::Magenta, "ONES (filled with 0xFF / undriven)"),
        (CellColor::Red, "CHANGED (ROM working data)"),
    ]);
}

/// Collapse adjacent blocks with the same class into runs and print
/// them as a table, then print per-class totals.
fn print_runs_and_totals(blocks: &[BlockResult], end: u32, block: u32) {
    type Run = (u32, u32, Class, Option<(u32, u32, u32)>);
    if crate::render::is_quiet() {
        return;
    }
    let mut runs: Vec<Run> = Vec::new();
    let mut run_start = blocks[0].addr;
    let mut run_class = blocks[0].class;
    let mut run_sample = blocks[0].first_diff;
    for b in &blocks[1..] {
        if b.class != run_class {
            runs.push((run_start, b.addr, run_class, run_sample));
            run_start = b.addr;
            run_class = b.class;
            run_sample = b.first_diff;
        }
    }
    runs.push((run_start, end, run_class, run_sample));

    println!();
    println!("{}", "Runs".bold());
    let mut t = make_table();
    t.set_header(vec![
        header_cell("Start"),
        header_cell("End"),
        header_cell("Size"),
        header_cell("Class"),
        header_cell("Sample mismatch"),
    ]);
    for (rs, re, class, sample) in &runs {
        let size = human_bytes((re - rs) as u64);
        let sample_str = match (class, sample) {
            (Class::Changed, Some((addr, expected, got))) => {
                format!("@0x{addr:08X}: exp={expected:08x} got={got:08x}")
            }
            (Class::Safe, _) => String::from("(all words match address)"),
            (Class::Zero, _) => String::from("(all 0x00000000)"),
            (Class::Ones, _) => String::from("(all 0xFFFFFFFF)"),
            _ => String::new(),
        };
        t.add_row(vec![
            Cell::new(format!("0x{rs:08X}")),
            Cell::new(format!("0x{re:08X}")),
            Cell::new(size).set_alignment(CellAlignment::Right),
            class_cell(*class),
            Cell::new(sample_str).fg(comfy_table::Color::DarkGrey),
        ]);
    }
    println!("{t}");

    let mut counts = [0usize; 4];
    for b in blocks {
        counts[b.class as usize] += 1;
    }

    println!();
    println!("{}", "Totals".bold());
    let mut t = make_table();
    t.set_header(vec![
        header_cell("Class"),
        header_cell("Blocks"),
        header_cell("Size"),
    ]);
    for (i, &count) in counts.iter().enumerate() {
        if count == 0 {
            continue;
        }
        let class = match i {
            0 => Class::Safe,
            1 => Class::Zero,
            2 => Class::Ones,
            _ => Class::Changed,
        };
        let size = human_bytes(count as u64 * block as u64);
        t.add_row(vec![
            class_cell(class),
            Cell::new(count.to_string()).set_alignment(CellAlignment::Right),
            Cell::new(size).set_alignment(CellAlignment::Right),
        ]);
    }
    println!("{t}");
}
