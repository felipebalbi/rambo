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
pub mod expectations;
pub mod fingerprint;
pub mod format;
pub mod heatmap;
pub mod io;
pub mod probe;
pub mod render;
pub mod report;
pub mod serde_hex;
pub mod stability;
pub mod survey;
pub mod write_readback;

use std::path::Path;

use color_eyre::eyre::Result;
use owo_colors::OwoColorize;

pub use cli::Cli;

use crate::expectations::{Loaded as LoadedExpectations, evaluate as evaluate_expectations};
use crate::format::human_bytes;
use crate::probe::{RamRange, clip_to_block, discover_ram_regions, open_session};
use crate::render::{class_inline, info_kv, is_quiet, section, set_quiet};
use crate::report::{BlockReport, ExpectationsReport, Modes, RegionReport, Report, SCHEMA_VERSION};

/// Process exit code returned by [`Cli::run`]. `0` on success, `1`
/// when at least one expectation failed.
#[must_use]
pub enum ExitStatus {
    Ok,
    ExpectationsFailed,
}

impl ExitStatus {
    #[must_use]
    pub fn code(&self) -> i32 {
        match self {
            ExitStatus::Ok => 0,
            ExitStatus::ExpectationsFailed => 1,
        }
    }
}

impl Cli {
    /// Drive the full diagnostic for this invocation. See module-level
    /// docs for the high-level flow.
    pub fn run(&self) -> Result<ExitStatus> {
        self.validate()?;

        // Quiet mode: JSON destined for stdout suppresses every other
        // stdout write so the JSON payload is parseable on its own.
        let json_to_stdout = matches!(self.json.as_deref(), Some(p) if p == Path::new("-"));
        if json_to_stdout {
            set_quiet(true);
        }

        let mut session = open_session(self)?;

        let regions = discover_ram_regions(&session);
        if regions.is_empty() {
            return Err(color_eyre::eyre::eyre!(
                "Chip '{}' declares no RAM regions in its memory map",
                self.chip
            ));
        }

        // Pre-flight: load + validate expectations against discovered
        // regions BEFORE any survey I/O happens, so typos fail fast.
        let loaded_expectations = self
            .expectations
            .as_deref()
            .map(|p| expectations::load_and_validate(p, &regions, self.block))
            .transpose()?;

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

        let mut region_reports: Vec<RegionReport> = Vec::new();

        for region in &regions {
            let Some(clipped) = clip_to_block(region, self.block) else {
                if !is_quiet() {
                    println!();
                    println!(
                        "  {} skipping {} ({}): smaller than --block {}",
                        "WARN:".yellow().bold(),
                        region.name,
                        human_bytes(region.len() as u64),
                        human_bytes(self.block as u64),
                    );
                }
                continue;
            };
            let result = self.run_region(&mut session, &clipped)?;
            region_reports.push(RegionReport::from_blocks(
                clipped.name.clone(),
                clipped.start,
                clipped.end,
                self.block,
                &result,
            ));
        }

        // Build the JSON report (only used when --json is set, but
        // assembling it is cheap and lets us evaluate expectations
        // against the same data structure either way).
        let all_blocks: Vec<BlockReport> = region_reports
            .iter()
            .flat_map(|r| r.blocks.iter().cloned())
            .collect();

        let expectations_report = loaded_expectations
            .as_ref()
            .map(|loaded| evaluate_expectations(loaded, &all_blocks));

        let exit_status = match &expectations_report {
            Some(r) if r.failed > 0 => ExitStatus::ExpectationsFailed,
            _ => ExitStatus::Ok,
        };

        if let Some(rep) = &expectations_report {
            print_expectations_results(rep, loaded_expectations.as_ref());
        }

        if let Some(path) = self.json.as_deref() {
            let report = Report {
                schema_version: SCHEMA_VERSION,
                rambo_version: env!("CARGO_PKG_VERSION"),
                chip: self.chip.clone(),
                block: self.block,
                generated_at: rfc3339_now(),
                modes: Modes {
                    reset_cycles: self.reset_cycles,
                    fingerprint: self.fingerprint,
                    dual_pattern: self.dual_pattern,
                    write_readback: self.write_readback,
                },
                regions: region_reports,
                expectations: expectations_report,
            };
            write_json_report(&report, path)?;
        }

        section("Done");
        print_class_legend();

        Ok(exit_status)
    }

    fn run_region(
        &self,
        session: &mut probe_rs::Session,
        region: &RamRange,
    ) -> Result<Vec<classify::BlockResult>> {
        section(&format!(
            "SRAM region {}  0x{:08X}–0x{:08X}  ({})",
            region.name,
            region.start,
            region.end,
            human_bytes(region.len() as u64)
        ));

        let survey_a = survey::full_sram_survey(
            session,
            region.start,
            region.end,
            self.block,
            self.reset_cycles,
            false,
        )?;

        // Track the readback that downstream consumers (fingerprint)
        // should use: dual-pattern wants the inverted-pattern (B)
        // readback; the plain case wants survey_a's readback.
        let (final_blocks, last_readback) = if self.dual_pattern {
            section("Dual-pattern run: re-write with !addr, reset, classify");
            let survey_b =
                survey::full_sram_survey(session, region.start, region.end, self.block, 1, true)?;
            section("Dual-pattern verdict");
            dual_pattern::print_dual_pattern_verdict(
                region.start,
                region.end,
                self.block,
                &survey_a.readback,
                &survey_b.readback,
            );
            (survey_b.blocks, survey_b.readback)
        } else {
            (survey_a.blocks, survey_a.readback)
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

        Ok(final_blocks)
    }
}

fn write_json_report(report: &Report, path: &Path) -> Result<()> {
    if path == Path::new("-") {
        serde_json::to_writer_pretty(std::io::stdout().lock(), report)?;
        println!();
    } else {
        let file = std::fs::File::create(path)?;
        serde_json::to_writer_pretty(std::io::BufWriter::new(file), report)?;
    }
    Ok(())
}

fn rfc3339_now() -> String {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| String::from("1970-01-01T00:00:00Z"))
}

fn print_expectations_results(report: &ExpectationsReport, _loaded: Option<&LoadedExpectations>) {
    use crate::report::{ClauseSummary, Outcome};
    if is_quiet() {
        return;
    }

    section("Expectations");

    for r in &report.results {
        let (sym, name_color) = match &r.outcome {
            Outcome::Pass => ("✓".green().to_string(), r.name.green().bold().to_string()),
            Outcome::Fail { .. } => ("✗".red().to_string(), r.name.red().bold().to_string()),
        };
        let clause_str = match &r.clause {
            ClauseSummary::Expect(c) => format!("expect {}", class_inline(*c)),
            ClauseSummary::ExpectAnyOf(cs) => format!(
                "expect any of [{}]",
                cs.iter()
                    .map(|c| class_inline(*c))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            ClauseSummary::ExpectNot(c) => format!("expect not {}", class_inline(*c)),
        };
        println!(
            "  {sym} {name_color}  0x{:08X}..0x{:08X}  {clause_str}",
            r.range_start, r.range_end
        );
        if let Some(rationale) = &r.rationale {
            println!("      {} {}", "rationale:".dimmed(), rationale.dimmed());
        }
        if let Outcome::Fail { offending_blocks } = &r.outcome {
            let limit = 5usize.min(offending_blocks.len());
            for b in &offending_blocks[..limit] {
                let diff = b
                    .first_diff
                    .as_ref()
                    .map(|d| {
                        format!(
                            " (first diff @0x{:08X}: 0x{:08X}→0x{:08X})",
                            d.addr, d.expected, d.got
                        )
                    })
                    .unwrap_or_default();
                println!(
                    "      {} block @0x{:08X} is {}{}",
                    "→".red(),
                    b.addr,
                    class_inline(b.class),
                    diff
                );
            }
            if offending_blocks.len() > limit {
                println!(
                    "      {} ({} more)",
                    "…".dimmed(),
                    offending_blocks.len() - limit
                );
            }
        }
    }

    println!();
    let totals = format!(
        "{} passed, {} failed (out of {})",
        report.passed, report.failed, report.total
    );
    if report.failed == 0 {
        println!("  {} {}", "✓".green().bold(), totals.green().bold());
    } else {
        println!("  {} {}", "✗".red().bold(), totals.red().bold());
    }
}

fn print_class_legend() {
    use crate::classify::Class;
    use crate::render::class_inline as ci;
    if is_quiet() {
        return;
    }
    println!("{}", "Class legend:".bold());
    println!(
        "  {}    every word still equals its address (untouched between write & re-read)",
        ci(Class::Safe)
    );
    println!(
        "  {}    block read back as all 0x00000000 (ROM scrub or undriven SRAM)",
        ci(Class::Zero)
    );
    println!(
        "  {}    block read back as all 0xFFFFFFFF (undriven SRAM)",
        ci(Class::Ones)
    );
    println!(
        "  {} block was modified in some other way",
        ci(Class::Changed)
    );
}
