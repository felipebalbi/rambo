//! Generic terminal output helpers: section headers, info lines,
//! pre-styled `comfy-table` tables, and the colored cells used by every
//! diagnostic.
//!
//! Keeping all styling in one module makes it easy to tweak the look of
//! the entire tool from a single place.

use comfy_table::presets::UTF8_FULL;
use comfy_table::{Attribute, Cell, Color as TableColor, ContentArrangement, Table};
use owo_colors::OwoColorize;

use crate::classify::Class;
use crate::heatmap::CellColor;

/// Map a [`Class`] to a `(table color, heatmap color, label)` triple.
///
/// This is the single source of truth for the per-class palette.
/// Both the runs/totals table and the survey heatmap consume it, so
/// the two views always agree on what each color means.
fn class_palette(class: Class) -> (TableColor, CellColor, &'static str) {
    match class {
        Class::Safe => (TableColor::Green, CellColor::Green, "SAFE"),
        Class::Zero => (TableColor::Blue, CellColor::Blue, "ZERO"),
        Class::Ones => (TableColor::Magenta, CellColor::Magenta, "ONES"),
        Class::Changed => (TableColor::Red, CellColor::Red, "CHANGED"),
    }
}

/// Heatmap color for a [`Class`].
pub fn class_color(class: Class) -> CellColor {
    class_palette(class).1
}

/// Print a bold, cyan section title underlined with box-drawing
/// dashes. Always emits a blank line first to separate sections.
pub fn section(title: &str) {
    println!();
    println!("{}", title.bold().cyan());
    println!("{}", "─".repeat(title.chars().count()).bright_black());
}

/// Print a single progress line prefixed with a dim `›`.
pub fn step(msg: &str) {
    println!("{} {}", "›".bright_black(), msg);
}

/// Print a `key: value` line where the key is dimmed.
pub fn info_kv(key: &str, value: impl std::fmt::Display) {
    println!("  {} {}", format!("{key}:").dimmed(), value);
}

/// Build an empty `comfy-table` with the standard preset everyone in
/// this crate uses (full UTF-8 borders, dynamic content arrangement
/// so widths follow the current terminal).
pub fn make_table() -> Table {
    let mut t = Table::new();
    t.load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic);
    t
}

/// Build a bold white cell suitable for use as a column header.
pub fn header_cell(text: &str) -> Cell {
    Cell::new(text)
        .add_attribute(Attribute::Bold)
        .fg(TableColor::White)
}

/// Build a colored, bold cell for one of the survey [`Class`] values.
pub fn class_cell(class: Class) -> Cell {
    let (color, _, text) = class_palette(class);
    Cell::new(text).fg(color).add_attribute(Attribute::Bold)
}

/// Render a [`Class`] as a colored, bold string for inline use in
/// prose (legends, summaries).
pub fn class_inline(class: Class) -> String {
    match class {
        Class::Safe => "SAFE".green().bold().to_string(),
        Class::Zero => "ZERO".blue().bold().to_string(),
        Class::Ones => "ONES".magenta().bold().to_string(),
        Class::Changed => "CHANGED".red().bold().to_string(),
    }
}
