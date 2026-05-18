//! Generic 1-KiB-cell, 64-column heatmap.
//!
//! Every diagnostic renders a colored grid where each cell summarizes
//! a small chunk of memory. The grid is always 64 columns wide; one
//! visual row is therefore `64 * cell_bytes` of address space, so the
//! row address marches up by a constant per line and the layout stays
//! easy to scan.
//!
//! Callers supply a closure that turns a cell index into a color
//! (`owo_colors::Style`-friendly) — the cell-bytes-to-words math and
//! row/column layout live here so they don't have to be repeated.

use owo_colors::OwoColorize;

/// One colored cell glyph for a heatmap.
pub enum CellColor {
    Green,
    Red,
    Yellow,
    Magenta,
    Blue,
}

impl CellColor {
    fn paint(&self, glyph: &str) -> String {
        match self {
            CellColor::Green => glyph.green().to_string(),
            CellColor::Red => glyph.red().to_string(),
            CellColor::Yellow => glyph.yellow().to_string(),
            CellColor::Magenta => glyph.magenta().to_string(),
            CellColor::Blue => glyph.blue().to_string(),
        }
    }
}

/// Print a 64-column heatmap covering `[start, start + n_cells * cell_bytes)`.
///
/// The caller's `color` closure receives the 0-based cell index and
/// returns the color for that cell. Cell index `i` corresponds to the
/// byte range `[start + i*cell_bytes, start + (i+1)*cell_bytes)`.
///
/// The function prints:
/// 1. A blank line then the heading.
/// 2. A column ruler every 8 columns.
/// 3. `n_rows = ceil(n_cells / 64)` rows of glyphs, each row prefixed
///    by the byte address of the first cell in the row.
pub fn render(
    title: &str,
    subtitle: &str,
    start: u32,
    cell_bytes: u32,
    n_cells: usize,
    color: impl Fn(usize) -> CellColor,
) {
    const COLS: usize = 64;
    let row_bytes = COLS as u32 * cell_bytes;
    let n_rows = n_cells.div_ceil(COLS);

    println!();
    println!("{} {}", bold(title), dim(subtitle));

    // Column ruler. The 12 leading spaces line up with the row-address
    // prefix `"0x20000000 │ "`.
    print!("            ");
    for col in 0..COLS {
        if col % 8 == 0 {
            print!("{}", dim(&format!("{col:<8}")));
        }
    }
    println!();

    for row in 0..n_rows {
        let row_addr = start + (row as u32) * row_bytes;
        print!(
            "{} {} ",
            dim(&format!("0x{row_addr:08X}")),
            bright_black("│")
        );
        for col in 0..COLS {
            let cell = row * COLS + col;
            if cell >= n_cells {
                print!(" ");
                continue;
            }
            print!("{}", color(cell).paint("█"));
        }
        println!();
    }
}

/// Print a single legend line. Each entry is `(color, label)`.
pub fn legend(entries: &[(CellColor, &str)]) {
    print!("  Legend: ");
    for (i, (c, label)) in entries.iter().enumerate() {
        if i > 0 {
            print!("    ");
        }
        print!("{} {}", c.paint("█"), label);
    }
    println!();
}

// Small wrappers that keep the call sites short and consistent.
fn bold(s: &str) -> String {
    s.bold().to_string()
}
fn dim(s: &str) -> String {
    s.dimmed().to_string()
}
fn bright_black(s: &str) -> String {
    s.bright_black().to_string()
}
