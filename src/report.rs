//! Wire-stable JSON report schema.
//!
//! # Stability contract
//!
//! The shape of [`Report`] (and everything it transitively contains)
//! is a **public artifact**: once a user wires
//! `rambo --json report.json` into CI, silently breaking the schema
//! costs them. The rules:
//!
//! - **Backward-compatible changes** (safe in any release):
//!   adding new optional fields (with `#[serde(default, skip_serializing_if = "Option::is_none")]`
//!   or sane defaults). Readers are expected to ignore unknown fields,
//!   so we do **not** put `deny_unknown_fields` on the output side.
//! - **Breaking changes**: removing fields, renaming, retyping,
//!   repurposing string-enum variants. These bump
//!   [`SCHEMA_VERSION`] *and* the crate's SemVer-significant digit
//!   (major if `>= 1.0`, minor for `0.x`).
//!
//! The `Class` enum (re-exported here via [`classify::Class`]) is
//! also wire-significant: adding a new variant is a breaking change
//! for downstream parsers that exhaustively match on it.
//!
//! Address fields use [`crate::serde_hex`] so they appear as
//! `"0x20030000"` in JSON — matches every other place in the tool
//! where addresses are printed.

use serde::Serialize;

use crate::classify::{BlockResult, Class};

/// Current schema version. Bump on any breaking change to [`Report`].
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Debug)]
pub struct Report {
    pub schema_version: u32,
    pub rambo_version: &'static str,
    pub chip: String,
    pub block: u32,
    pub generated_at: String,
    pub modes: Modes,
    pub regions: Vec<RegionReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expectations: Option<ExpectationsReport>,
}

#[derive(Serialize, Debug, Default)]
pub struct Modes {
    pub reset_cycles: u32,
    pub fingerprint: bool,
    pub dual_pattern: bool,
    pub write_readback: bool,
}

#[derive(Serialize, Debug)]
pub struct RegionReport {
    pub name: String,
    #[serde(with = "crate::serde_hex")]
    pub start: u32,
    #[serde(with = "crate::serde_hex")]
    pub end: u32,
    pub blocks: Vec<BlockReport>,
    pub totals: Totals,
    pub runs: Vec<Run>,
}

#[derive(Serialize, Debug, Clone)]
pub struct BlockReport {
    #[serde(with = "crate::serde_hex")]
    pub addr: u32,
    pub class: Class,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_diff: Option<FirstDiff>,
}

#[derive(Serialize, Debug, Clone)]
pub struct FirstDiff {
    #[serde(with = "crate::serde_hex")]
    pub addr: u32,
    #[serde(with = "crate::serde_hex")]
    pub expected: u32,
    #[serde(with = "crate::serde_hex")]
    pub got: u32,
}

#[derive(Serialize, Debug, Default)]
pub struct Totals {
    pub safe_bytes: u64,
    pub zero_bytes: u64,
    pub ones_bytes: u64,
    pub changed_bytes: u64,
}

#[derive(Serialize, Debug)]
pub struct Run {
    #[serde(with = "crate::serde_hex")]
    pub start: u32,
    #[serde(with = "crate::serde_hex")]
    pub end: u32,
    pub class: Class,
}

#[derive(Serialize, Debug)]
pub struct ExpectationsReport {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub results: Vec<ExpectationResult>,
}

#[derive(Serialize, Debug)]
pub struct ExpectationResult {
    pub name: String,
    #[serde(with = "crate::serde_hex")]
    pub range_start: u32,
    #[serde(with = "crate::serde_hex")]
    pub range_end: u32,
    pub clause: ClauseSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
    pub outcome: Outcome,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ClauseSummary {
    Expect(Class),
    ExpectAnyOf(Vec<Class>),
    ExpectNot(Class),
}

#[derive(Serialize, Debug)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum Outcome {
    Pass,
    Fail { offending_blocks: Vec<BlockReport> },
}

impl RegionReport {
    /// Build a per-region report from the already-classified blocks
    /// the survey produced.
    pub fn from_blocks(
        name: String,
        start: u32,
        end: u32,
        block: u32,
        blocks: &[BlockResult],
    ) -> Self {
        let block_reports: Vec<BlockReport> = blocks.iter().map(BlockReport::from).collect();

        let mut totals = Totals::default();
        let block_bytes = u64::from(block);
        for b in blocks {
            match b.class {
                Class::Safe => totals.safe_bytes += block_bytes,
                Class::Zero => totals.zero_bytes += block_bytes,
                Class::Ones => totals.ones_bytes += block_bytes,
                Class::Changed => totals.changed_bytes += block_bytes,
            }
        }

        let runs = collapse_runs(blocks, end);

        Self {
            name,
            start,
            end,
            blocks: block_reports,
            totals,
            runs,
        }
    }
}

impl From<&BlockResult> for BlockReport {
    fn from(b: &BlockResult) -> Self {
        Self {
            addr: b.addr,
            class: b.class,
            first_diff: b.first_diff.map(|(addr, expected, got)| FirstDiff {
                addr,
                expected,
                got,
            }),
        }
    }
}

fn collapse_runs(blocks: &[BlockResult], end: u32) -> Vec<Run> {
    if blocks.is_empty() {
        return Vec::new();
    }
    let mut runs = Vec::new();
    let mut run_start = blocks[0].addr;
    let mut run_class = blocks[0].class;
    for b in &blocks[1..] {
        if b.class != run_class {
            runs.push(Run {
                start: run_start,
                end: b.addr,
                class: run_class,
            });
            run_start = b.addr;
            run_class = b.class;
        }
    }
    runs.push(Run {
        start: run_start,
        end,
        class: run_class,
    });
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_block(addr: u32, class: Class) -> BlockResult {
        BlockResult {
            addr,
            class,
            first_diff: None,
        }
    }

    #[test]
    fn class_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Class::Safe).unwrap(), r#""safe""#);
        assert_eq!(serde_json::to_string(&Class::Zero).unwrap(), r#""zero""#);
        assert_eq!(serde_json::to_string(&Class::Ones).unwrap(), r#""ones""#);
        assert_eq!(
            serde_json::to_string(&Class::Changed).unwrap(),
            r#""changed""#
        );
    }

    #[test]
    fn collapses_runs_correctly() {
        // 0x..00 SAFE, 0x..10 SAFE, 0x..20 CHANGED, 0x..30 ZERO, 0x..40 ZERO
        let blocks = vec![
            mk_block(0x2000_0000, Class::Safe),
            mk_block(0x2000_0010, Class::Safe),
            mk_block(0x2000_0020, Class::Changed),
            mk_block(0x2000_0030, Class::Zero),
            mk_block(0x2000_0040, Class::Zero),
        ];
        let runs = collapse_runs(&blocks, 0x2000_0050);
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].start, 0x2000_0000);
        assert_eq!(runs[0].end, 0x2000_0020);
        assert_eq!(runs[0].class, Class::Safe);
        assert_eq!(runs[1].start, 0x2000_0020);
        assert_eq!(runs[1].end, 0x2000_0030);
        assert_eq!(runs[1].class, Class::Changed);
        assert_eq!(runs[2].start, 0x2000_0030);
        assert_eq!(runs[2].end, 0x2000_0050);
        assert_eq!(runs[2].class, Class::Zero);
    }

    #[test]
    fn region_report_totals_match_block_counts() {
        let block = 0x10;
        let blocks = vec![
            mk_block(0x2000_0000, Class::Safe),
            mk_block(0x2000_0010, Class::Safe),
            mk_block(0x2000_0020, Class::Changed),
            mk_block(0x2000_0030, Class::Zero),
        ];
        let r = RegionReport::from_blocks("SRAM".into(), 0x2000_0000, 0x2000_0040, block, &blocks);
        assert_eq!(r.totals.safe_bytes, 0x20);
        assert_eq!(r.totals.changed_bytes, 0x10);
        assert_eq!(r.totals.zero_bytes, 0x10);
        assert_eq!(r.totals.ones_bytes, 0);
    }

    #[test]
    fn report_round_trips_via_json() {
        // We only need the *output* to be stable, so this is a smoke
        // test that the structure serializes at all, plus a couple of
        // checks on key field names.
        let report = Report {
            schema_version: SCHEMA_VERSION,
            rambo_version: env!("CARGO_PKG_VERSION"),
            chip: "MCXA276".into(),
            block: 0x1000,
            generated_at: "2025-01-01T00:00:00Z".into(),
            modes: Modes::default(),
            regions: vec![RegionReport::from_blocks(
                "SRAM0".into(),
                0x2000_0000,
                0x2000_0010,
                0x10,
                &[mk_block(0x2000_0000, Class::Safe)],
            )],
            expectations: None,
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains(r#""schema_version":1"#));
        assert!(json.contains(r#""chip":"MCXA276""#));
        assert!(json.contains(r#""start":"0x20000000""#));
        assert!(json.contains(r#""class":"safe""#));
        assert!(!json.contains("expectations"));
    }
}
