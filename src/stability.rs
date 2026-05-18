//! Stability heatmap and drift table for multi-reset surveys.
//!
//! When the survey runs `N > 1` reset cycles, each block has `N`
//! classifications (one per cycle). A block is "stable" if every
//! cycle produced the same [`Class`], otherwise it "drifted".
//!
//! We render two things:
//!
//! - A grid where each cell == one `block` (green = stable across all
//!   N cycles, red = drifted at least once). The grid is the standard
//!   64-column heatmap, so widths line up with the survey heatmap.
//! - A small "first 32 unstable blocks" table showing, for each
//!   drifting block, the [`Class`] reported in each reset cycle.
//!
//! Drifting blocks are strong evidence of undriven (floating) SRAM
//! cells, while stable blocks reflect deterministic ROM behavior.

use comfy_table::{Attribute, Cell};
use owo_colors::OwoColorize;

use crate::classify::Class;
use crate::format::human_bytes;
use crate::heatmap::{self, CellColor};
use crate::render::{class_cell, header_cell, make_table};

#[allow(clippy::too_many_lines)]
pub fn print_stability(start: u32, end: u32, block: u32, per_cycle: &[Vec<Class>]) {
    const DRIFT_LIMIT: usize = 32;

    assert!(per_cycle.len() >= 2);
    let n_blocks = per_cycle[0].len();
    assert!(per_cycle.iter().all(|c| c.len() == n_blocks));

    // Decide which blocks are stable.
    let mut stable = vec![true; n_blocks];
    for b in 0..n_blocks {
        let first = per_cycle[0][b];
        for cycle in &per_cycle[1..] {
            if cycle[b] != first {
                stable[b] = false;
                break;
            }
        }
    }

    heatmap::render(
        "Stability heatmap",
        &format!(
            "(each cell = {}, across {} resets)",
            human_bytes(block as u64),
            per_cycle.len()
        ),
        start,
        block,
        n_blocks,
        |i| {
            if stable[i] {
                CellColor::Green
            } else {
                CellColor::Red
            }
        },
    );
    println!();
    heatmap::legend(&[
        (CellColor::Green, "same class every cycle"),
        (CellColor::Red, "class drifted between resets"),
    ]);

    // Totals table.
    let stable_count = stable.iter().filter(|&&s| s).count();
    let drift_count = n_blocks - stable_count;

    println!();
    println!("{}", "Stability totals".bold());
    let mut t = make_table();
    t.set_header(vec![
        header_cell("Result"),
        header_cell("Blocks"),
        header_cell("Size"),
    ]);
    t.add_row(vec![
        Cell::new("STABLE")
            .fg(comfy_table::Color::Green)
            .add_attribute(Attribute::Bold),
        Cell::new(stable_count.to_string()),
        Cell::new(human_bytes(stable_count as u64 * block as u64)),
    ]);
    t.add_row(vec![
        Cell::new("DRIFTED")
            .fg(comfy_table::Color::Red)
            .add_attribute(Attribute::Bold),
        Cell::new(drift_count.to_string()),
        Cell::new(human_bytes(drift_count as u64 * block as u64)),
    ]);
    println!("{t}");
    let _ = end; // kept in the signature for symmetry with survey caller.

    if drift_count == 0 {
        return;
    }
    println!();
    println!(
        "{} {}",
        "Per-cell drift (first 32)".bold(),
        format!("({drift_count} unstable blocks total)").dimmed()
    );
    let mut t = make_table();
    let mut header = vec![header_cell("Address")];
    for cycle in 1..=per_cycle.len() {
        header.push(header_cell(&format!("Cycle {cycle}")));
    }
    t.set_header(header);

    let mut shown = 0;
    for (i, &is_stable) in stable.iter().enumerate() {
        if is_stable {
            continue;
        }
        let addr = start + (i as u32) * block;
        let mut row = vec![Cell::new(format!("0x{addr:08X}"))];
        for cycle in per_cycle {
            row.push(class_cell(cycle[i]));
        }
        t.add_row(row);
        shown += 1;
        if shown >= DRIFT_LIMIT {
            break;
        }
    }
    println!("{t}");
    if drift_count > DRIFT_LIMIT {
        println!(
            "  {}",
            format!(
                "... {} more unstable blocks omitted",
                drift_count - DRIFT_LIMIT
            )
            .dimmed()
        );
    }
}
