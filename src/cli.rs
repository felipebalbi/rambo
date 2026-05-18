//! Command-line interface definition.
//!
//! Holds the `Cli` struct and the `--dual-pattern` / `--reset-cycles`
//! validation rule. Orchestration of what to actually do with these
//! flags lives in [`crate::run`] in `lib.rs`.

use clap::Parser;
use color_eyre::eyre::{Result, eyre};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(author, version, about = "A tool to map ROM collateral damage")]
pub struct Cli {
    /// Target chip name (probe-rs target identifier), e.g. `MCXA266`.
    #[arg(long, required = true)]
    pub chip: String,

    /// Block size for the survey classification (bytes).
    ///
    /// Accepts decimal, `0x`/`0o`/`0b` prefixes and `_` separators.
    #[arg(long, value_parser = parse_int::parse::<u32>, default_value = "0x1000")]
    pub block: u32,

    /// Number of reset+read cycles. With N>1, RAM is written only
    /// once and then re-read after each of N resets so you can see
    /// whether post-ROM state is deterministic or drifts between
    /// resets. Incompatible with `--dual-pattern`.
    #[arg(long, default_value = "1")]
    pub reset_cycles: u32,

    /// Fingerprint each CHANGED block: bit density, top values,
    /// pattern matches (constant, ascending counter, repeating motif,
    /// address-as-data with offset, etc).
    #[arg(long)]
    pub fingerprint: bool,

    /// Attempt to determine if a partition is undriven or modified
    /// by ROM by initializing SRAM once with addr-as-data and once
    /// with !addr.
    #[arg(long)]
    pub dual_pattern: bool,

    /// Write addr-as-data to the SRAM range and read it back
    /// immediately, with NO reset between. Detects whether real RAM
    /// is smaller than the reserved address window (unmapped regions
    /// reading as 0/FF, aliasing to another partition).
    #[arg(long)]
    pub write_readback: bool,

    /// Probe selector (e.g. "0483:374b" or a serial). Defaults to the
    /// first probe found.
    #[arg(long)]
    pub probe: Option<String>,

    /// Path to a chip description YAML file (CMSIS-Pack style) to
    /// load instead of the built-in probe-rs target database.
    #[arg(long, value_name = "PATH")]
    pub chip_description_path: Option<PathBuf>,

    /// Write a machine-readable JSON report of the survey results to
    /// PATH. Use `-` to write to stdout (which suppresses all normal
    /// text output so the JSON is parseable on its own).
    #[arg(long, value_name = "PATH")]
    pub json: Option<PathBuf>,

    /// Path to a JSON expectations ("RAM contract") file. Each
    /// expectation declares a class that a block-aligned range must
    /// (or must not) classify as after the survey. Any failure causes
    /// rambo to exit with a non-zero status.
    #[arg(long, value_name = "PATH")]
    pub expectations: Option<PathBuf>,
}

impl Cli {
    /// Reject impossible flag combinations before any I/O happens.
    pub fn validate(&self) -> Result<()> {
        if self.dual_pattern && self.reset_cycles > 1 {
            return Err(eyre!(
                "--dual-pattern is incompatible with --reset-cycles>1 \
                 (dual-pattern always does exactly two write+reset cycles)"
            ));
        }
        if self.block == 0 || !self.block.is_multiple_of(4) {
            return Err(eyre!("--block must be a non-zero multiple of 4"));
        }
        if self.reset_cycles == 0 {
            return Err(eyre!("--reset-cycles must be >= 1"));
        }
        if self.expectations.is_some() {
            if self.dual_pattern {
                return Err(eyre!(
                    "--expectations is incompatible with --dual-pattern \
                     (expectations are evaluated against the post-reset \
                     survey readback only)"
                ));
            }
            if self.write_readback {
                return Err(eyre!(
                    "--expectations is incompatible with --write-readback"
                ));
            }
            if self.reset_cycles > 1 {
                return Err(eyre!(
                    "--expectations is incompatible with --reset-cycles>1 \
                     (expectations are evaluated against a single survey \
                     readback)"
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(unused_imports)]
    use clap::Parser as _;

    fn parse(args: &[&str]) -> Cli {
        let mut full = vec!["rambo"];
        full.extend_from_slice(args);
        Cli::parse_from(full)
    }

    #[test]
    fn rejects_dual_pattern_with_multi_reset() {
        let cli = parse(&["--chip", "X", "--dual-pattern", "--reset-cycles", "3"]);
        assert!(cli.validate().is_err());
    }

    #[test]
    fn rejects_non_multiple_of_4_block() {
        let cli = parse(&["--chip", "X", "--block", "7"]);
        assert!(cli.validate().is_err());
    }

    #[test]
    fn rejects_zero_reset_cycles() {
        let cli = parse(&["--chip", "X", "--reset-cycles", "0"]);
        assert!(cli.validate().is_err());
    }

    #[test]
    fn rejects_expectations_with_dual_pattern() {
        let cli = parse(&[
            "--chip",
            "X",
            "--dual-pattern",
            "--expectations",
            "exp.json",
        ]);
        assert!(cli.validate().is_err());
    }

    #[test]
    fn rejects_expectations_with_write_readback() {
        let cli = parse(&[
            "--chip",
            "X",
            "--write-readback",
            "--expectations",
            "exp.json",
        ]);
        assert!(cli.validate().is_err());
    }

    #[test]
    fn rejects_expectations_with_multi_reset() {
        let cli = parse(&[
            "--chip",
            "X",
            "--reset-cycles",
            "3",
            "--expectations",
            "exp.json",
        ]);
        assert!(cli.validate().is_err());
    }

    #[test]
    fn accepts_json_and_expectations_with_defaults() {
        let cli = parse(&[
            "--chip",
            "X",
            "--json",
            "report.json",
            "--expectations",
            "exp.json",
        ]);
        assert!(cli.validate().is_ok());
        assert_eq!(cli.json.as_ref().unwrap().to_str().unwrap(), "report.json");
    }

    #[test]
    fn accepts_defaults() {
        let cli = parse(&["--chip", "X"]);
        assert!(cli.validate().is_ok());
        assert_eq!(cli.block, 0x1000);
        assert_eq!(cli.reset_cycles, 1);
    }

    #[test]
    fn block_accepts_multiple_radixes() {
        assert_eq!(parse(&["--chip", "X", "--block", "4096"]).block, 4096);
        assert_eq!(parse(&["--chip", "X", "--block", "0x1000"]).block, 4096);
        assert_eq!(parse(&["--chip", "X", "--block", "0o10000"]).block, 4096);
    }
}
