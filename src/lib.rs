//! `rambo` — RAM ROM-clobber surveyor.
//!
//! Maps the post-reset state of every RAM region declared by a
//! probe-rs target. Used to figure out which SRAM partitions the
//! boot ROM scrubs, which ones survive, and which ones are undriven.
//!
//! All probe-rs interaction lives in the [`probe`] and [`io`]
//! modules; everything else is pure logic that can be unit-tested
//! without hardware.

#![cfg_attr(test, allow(clippy::unwrap_used))]

pub mod classify;
pub mod cli;
pub mod dual_pattern;
pub mod fingerprint;
pub mod format;
pub mod heatmap;
pub mod io;
pub mod probe;
pub mod render;
pub mod stability;
pub mod survey;
pub mod write_readback;

use color_eyre::eyre::Result;
use owo_colors::OwoColorize;

pub use cli::Cli;

use crate::format::human_bytes;
use crate::probe::{RamRange, clip_to_block, discover_ram_regions, open_session};
use crate::render::{class_inline, info_kv, section};

impl Cli {
    /// Drive the full diagnostic for this invocation.
    ///
    /// 1. Validate flag combinations.
    /// 2. Open a probe-rs session against `--chip`.
    /// 3. Discover every RAM region from the chip's memory map.
    /// 4. For each region, run the requested operations in this order:
    ///    full survey → optional dual-pattern → optional fingerprint
    ///    → optional write-readback.
    pub fn run(&self) -> Result<()> {
        self.validate()?;

        let mut session = open_session(self)?;

        let regions = discover_ram_regions(&session);
        if regions.is_empty() {
            return Err(color_eyre::eyre::eyre!(
                "Chip '{}' declares no RAM regions in its memory map",
                self.chip
            ));
        }

        section("Discovered RAM regions");
        for r in &regions {
            info_kv(
                &r.name,
                format!(
                    "0x{:08X}..0x{:08X}  ({})",
                    r.start,
                    r.end,
                    human_bytes(r.len() as u64)
                ),
            );
        }

        for region in &regions {
            let Some(clipped) = clip_to_block(region, self.block) else {
                println!();
                println!(
                    "  {} skipping {} ({}): smaller than --block {}",
                    "WARN:".yellow().bold(),
                    region.name,
                    human_bytes(region.len() as u64),
                    human_bytes(self.block as u64),
                );
                continue;
            };
            self.run_region(&mut session, &clipped)?;
        }

        section("Done");
        print_class_legend();

        Ok(())
    }

    fn run_region(&self, session: &mut probe_rs::Session, region: &RamRange) -> Result<()> {
        section(&format!(
            "SRAM region {}  0x{:08X}–0x{:08X}  ({})",
            region.name,
            region.start,
            region.end,
            human_bytes(region.len() as u64)
        ));

        let readback_a = survey::full_sram_survey(
            session,
            region.start,
            region.end,
            self.block,
            self.reset_cycles,
            false,
        )?;

        let last_readback = if self.dual_pattern {
            section("Dual-pattern run: re-write with !addr, reset, classify");
            let readback_b =
                survey::full_sram_survey(session, region.start, region.end, self.block, 1, true)?;
            section("Dual-pattern verdict");
            dual_pattern::print_dual_pattern_verdict(
                region.start,
                region.end,
                self.block,
                &readback_a,
                &readback_b,
            );
            readback_b
        } else {
            readback_a
        };

        if self.fingerprint {
            section("Fingerprint of CHANGED blocks");
            fingerprint::fingerprint_changed(region.start, region.end, self.block, &last_readback);
        }

        if self.write_readback {
            section(&format!(
                "Write-readback (no reset) on {} 0x{:08X}–0x{:08X}",
                region.name, region.start, region.end
            ));
            write_readback::write_readback_test(session, region.start, region.end, self.block)?;
        }

        Ok(())
    }
}

fn print_class_legend() {
    use crate::classify::Class;
    println!("{}", "Class legend:".bold());
    println!(
        "  {}    every word still equals its address (untouched between write & re-read)",
        class_inline(Class::Safe)
    );
    println!(
        "  {}    block read back as all 0x00000000 (ROM scrub or undriven SRAM)",
        class_inline(Class::Zero)
    );
    println!(
        "  {}    block read back as all 0xFFFFFFFF (undriven SRAM)",
        class_inline(Class::Ones)
    );
    println!(
        "  {} block was modified in some other way",
        class_inline(Class::Changed)
    );
}
