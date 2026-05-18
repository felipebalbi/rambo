//! "RAM contract" expectations: a JSON file the user writes to
//! declare what each address range should classify as after the
//! survey.
//!
//! Use case: "I store crash dumps at 0x20030000..0x20031000 and need
//! them to survive a watchdog reset. Fail my CI if a new chip rev
//! starts clobbering that range."
//!
//! # File format
//!
//! ```json
//! {
//!   "schema_version": 1,
//!   "expectations": [
//!     {
//!       "name": "crash_dump_area",
//!       "range": { "start": "0x20030000", "end": "0x20031000" },
//!       "expect": "safe",
//!       "rationale": "Reserved for crash-dump recovery"
//!     }
//!   ]
//! }
//! ```
//!
//! Exactly one of `expect`, `expect_any_of`, or `expect_not` must
//! appear per expectation.
//!
//! # Validation
//!
//! Expectations are validated against the discovered RAM regions
//! **before** any probe I/O happens. We reject:
//! - misaligned ranges (must align to `--block`),
//! - empty/inverted ranges,
//! - ranges that don't fit fully within exactly one declared region,
//! - duplicate expectation names,
//! - unknown JSON fields (`deny_unknown_fields`),
//! - schema versions we don't recognize.

use std::collections::BTreeSet;
use std::path::Path;

use color_eyre::eyre::{Result, eyre};
use serde::Deserialize;

use crate::classify::Class;
use crate::probe::RamRange;
use crate::report::{BlockReport, ClauseSummary, ExpectationResult, ExpectationsReport, Outcome};

const SUPPORTED_SCHEMA: u32 = 1;

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct ExpectationsFile {
    pub schema_version: u32,
    pub expectations: Vec<Expectation>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Expectation {
    pub name: String,
    pub range: AddrRange,
    /// Block must classify as exactly this class.
    #[serde(default)]
    pub expect: Option<Class>,
    /// Block must classify as one of these classes.
    #[serde(default)]
    pub expect_any_of: Option<Vec<Class>>,
    /// Block must NOT classify as this class.
    #[serde(default)]
    pub expect_not: Option<Class>,
    #[serde(default)]
    pub rationale: Option<String>,
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct AddrRange {
    #[serde(with = "crate::serde_hex")]
    pub start: u32,
    #[serde(with = "crate::serde_hex")]
    pub end: u32,
}

/// The validated form of an expectation's clause. Produced by
/// [`Expectation::clause`] after we've confirmed exactly one of
/// `expect` / `expect_any_of` / `expect_not` was provided.
#[derive(Debug, Clone)]
pub enum Clause {
    Single(Class),
    AnyOf(Vec<Class>),
    Not(Class),
}

impl Expectation {
    /// Returns the validated clause, erroring if zero or more than
    /// one of `expect` / `expect_any_of` / `expect_not` was set.
    pub fn clause(&self) -> Result<Clause> {
        match (self.expect, self.expect_any_of.as_ref(), self.expect_not) {
            (Some(c), None, None) => Ok(Clause::Single(c)),
            (None, Some(list), None) => Ok(Clause::AnyOf(list.clone())),
            (None, None, Some(c)) => Ok(Clause::Not(c)),
            (None, None, None) => Err(eyre!(
                "expectation {:?}: must specify exactly one of \
                 `expect`, `expect_any_of`, or `expect_not`",
                self.name
            )),
            _ => Err(eyre!(
                "expectation {:?}: only one of `expect`, \
                 `expect_any_of`, or `expect_not` may be set",
                self.name
            )),
        }
    }
}

/// Owned, validated, ready-to-evaluate set of expectations. Storing
/// the resolved [`Clause`] alongside the original [`Expectation`]
/// means [`evaluate`] never has to revalidate or panic.
#[derive(Debug)]
pub struct Loaded {
    pub items: Vec<LoadedExpectation>,
}

#[derive(Debug)]
pub struct LoadedExpectation {
    pub name: String,
    pub range: AddrRange,
    pub clause: Clause,
    pub rationale: Option<String>,
}

/// Load `path`, parse with serde, and validate every expectation
/// against the supplied region map. Performs **all** validation
/// before returning, so a single error message surfaces all problems
/// only when there's exactly one — multiple problems still surface
/// one at a time, but always before any hardware I/O.
pub fn load_and_validate(path: &Path, regions: &[RamRange], block: u32) -> Result<Loaded> {
    let file = std::fs::File::open(path)
        .map_err(|e| eyre!("failed to open expectations file {}: {e}", path.display()))?;
    let parsed: ExpectationsFile = serde_json::from_reader(std::io::BufReader::new(file))
        .map_err(|e| eyre!("failed to parse expectations file {}: {e}", path.display()))?;

    if parsed.schema_version != SUPPORTED_SCHEMA {
        return Err(eyre!(
            "expectations schema_version {} not supported (this rambo understands {})",
            parsed.schema_version,
            SUPPORTED_SCHEMA
        ));
    }

    // Duplicate names.
    let mut seen = BTreeSet::new();
    for exp in &parsed.expectations {
        if !seen.insert(exp.name.clone()) {
            return Err(eyre!("duplicate expectation name: {:?}", exp.name));
        }
    }

    for exp in &parsed.expectations {
        validate_one(exp, regions, block)?;
    }

    let items = parsed
        .expectations
        .into_iter()
        .map(|exp| {
            let clause = exp.clause()?;
            Ok(LoadedExpectation {
                name: exp.name,
                range: exp.range,
                clause,
                rationale: exp.rationale,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(Loaded { items })
}

fn validate_one(exp: &Expectation, regions: &[RamRange], block: u32) -> Result<()> {
    let (start, end) = (exp.range.start, exp.range.end);

    if end <= start {
        return Err(eyre!(
            "expectation {:?}: empty or inverted range 0x{:08X}..0x{:08X}",
            exp.name,
            start,
            end
        ));
    }

    if !start.is_multiple_of(block) || !end.is_multiple_of(block) {
        return Err(eyre!(
            "expectation {:?}: range 0x{:08X}..0x{:08X} is not aligned to --block 0x{:X}",
            exp.name,
            start,
            end,
            block
        ));
    }

    regions
        .iter()
        .find(|r| r.start <= start && end <= r.end)
        .ok_or_else(|| {
            eyre!(
                "expectation {:?}: range 0x{:08X}..0x{:08X} does not fit within any declared RAM region",
                exp.name,
                start,
                end
            )
        })?;

    // Force the clause to validate (exactly-one-of), and also reject
    // empty expect_any_of lists.
    match exp.clause()? {
        Clause::AnyOf(list) if list.is_empty() => {
            return Err(eyre!(
                "expectation {:?}: expect_any_of must not be empty",
                exp.name
            ));
        }
        _ => {}
    }

    Ok(())
}

/// Evaluate every loaded expectation against the post-survey block
/// classifications and produce a serializable report.
///
/// `all_blocks` is the concatenation of every region's
/// `Vec<BlockReport>` — typically built by flattening
/// `report.regions.iter().flat_map(|r| r.blocks.iter().cloned())`.
pub fn evaluate(loaded: &Loaded, all_blocks: &[BlockReport]) -> ExpectationsReport {
    let mut results = Vec::with_capacity(loaded.items.len());
    let mut passed = 0;
    let mut failed = 0;

    for exp in &loaded.items {
        let in_range: Vec<BlockReport> = all_blocks
            .iter()
            .filter(|b| b.addr >= exp.range.start && b.addr < exp.range.end)
            .cloned()
            .collect();

        let (clause_summary, offending) = match &exp.clause {
            Clause::Single(expect) => (
                ClauseSummary::Expect(*expect),
                in_range
                    .iter()
                    .filter(|b| b.class != *expect)
                    .cloned()
                    .collect::<Vec<_>>(),
            ),
            Clause::AnyOf(expect_any_of) => (
                ClauseSummary::ExpectAnyOf(expect_any_of.clone()),
                in_range
                    .iter()
                    .filter(|b| !expect_any_of.contains(&b.class))
                    .cloned()
                    .collect::<Vec<_>>(),
            ),
            Clause::Not(expect_not) => (
                ClauseSummary::ExpectNot(*expect_not),
                in_range
                    .iter()
                    .filter(|b| b.class == *expect_not)
                    .cloned()
                    .collect::<Vec<_>>(),
            ),
        };

        let outcome = if offending.is_empty() {
            passed += 1;
            Outcome::Pass
        } else {
            failed += 1;
            Outcome::Fail {
                offending_blocks: offending,
            }
        };

        results.push(ExpectationResult {
            name: exp.name.clone(),
            range_start: exp.range.start,
            range_end: exp.range.end,
            clause: clause_summary,
            rationale: exp.rationale.clone(),
            outcome,
        });
    }

    ExpectationsReport {
        total: loaded.items.len(),
        passed,
        failed,
        results,
    }
}

// BlockReport needs to be Clone for the evaluator's filter() collect.
// Add the derive in report.rs.

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(contents: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        f
    }

    fn one_region() -> Vec<RamRange> {
        vec![RamRange {
            name: "SRAM".into(),
            start: 0x2000_0000,
            end: 0x2004_0000,
        }]
    }

    #[test]
    fn rejects_bad_schema_version() {
        let f = write_tmp(r#"{"schema_version":99,"expectations":[]}"#);
        let err = load_and_validate(f.path(), &one_region(), 0x1000).unwrap_err();
        assert!(err.to_string().contains("schema_version"));
    }

    #[test]
    fn rejects_unknown_top_level_fields() {
        let f = write_tmp(r#"{"schema_version":1,"expectations":[],"extra_field":1}"#);
        let err = load_and_validate(f.path(), &one_region(), 0x1000).unwrap_err();
        assert!(err.to_string().contains("extra_field") || err.to_string().contains("unknown"));
    }

    #[test]
    fn rejects_unknown_expectation_fields() {
        let f = write_tmp(
            r#"{
                "schema_version":1,
                "expectations":[{
                    "name":"x",
                    "range":{"start":"0x20000000","end":"0x20001000"},
                    "expect":"safe",
                    "typo_field":1
                }]
            }"#,
        );
        let err = load_and_validate(f.path(), &one_region(), 0x1000).unwrap_err();
        assert!(err.to_string().contains("typo_field") || err.to_string().contains("unknown"));
    }

    #[test]
    fn rejects_misaligned_range() {
        let f = write_tmp(
            r#"{
                "schema_version":1,
                "expectations":[{
                    "name":"x",
                    "range":{"start":"0x20000004","end":"0x20001000"},
                    "expect":"safe"
                }]
            }"#,
        );
        let err = load_and_validate(f.path(), &one_region(), 0x1000).unwrap_err();
        assert!(err.to_string().contains("aligned"));
    }

    #[test]
    fn rejects_range_outside_any_region() {
        let f = write_tmp(
            r#"{
                "schema_version":1,
                "expectations":[{
                    "name":"x",
                    "range":{"start":"0x10000000","end":"0x10001000"},
                    "expect":"safe"
                }]
            }"#,
        );
        let err = load_and_validate(f.path(), &one_region(), 0x1000).unwrap_err();
        assert!(err.to_string().contains("does not fit"));
    }

    #[test]
    fn rejects_duplicate_names() {
        let f = write_tmp(
            r#"{
                "schema_version":1,
                "expectations":[
                  {"name":"dup","range":{"start":"0x20000000","end":"0x20001000"},"expect":"safe"},
                  {"name":"dup","range":{"start":"0x20001000","end":"0x20002000"},"expect":"safe"}
                ]
            }"#,
        );
        let err = load_and_validate(f.path(), &one_region(), 0x1000).unwrap_err();
        assert!(err.to_string().contains("duplicate"));
    }

    #[test]
    fn rejects_empty_any_of() {
        let f = write_tmp(
            r#"{
                "schema_version":1,
                "expectations":[{
                    "name":"x",
                    "range":{"start":"0x20000000","end":"0x20001000"},
                    "expect_any_of":[]
                }]
            }"#,
        );
        let err = load_and_validate(f.path(), &one_region(), 0x1000).unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn rejects_missing_clause() {
        let f = write_tmp(
            r#"{
                "schema_version":1,
                "expectations":[{
                    "name":"x",
                    "range":{"start":"0x20000000","end":"0x20001000"}
                }]
            }"#,
        );
        let err = load_and_validate(f.path(), &one_region(), 0x1000).unwrap_err();
        assert!(err.to_string().contains("exactly one"));
    }

    #[test]
    fn rejects_multiple_clauses() {
        let f = write_tmp(
            r#"{
                "schema_version":1,
                "expectations":[{
                    "name":"x",
                    "range":{"start":"0x20000000","end":"0x20001000"},
                    "expect":"safe",
                    "expect_not":"changed"
                }]
            }"#,
        );
        let err = load_and_validate(f.path(), &one_region(), 0x1000).unwrap_err();
        assert!(err.to_string().contains("only one"));
    }

    #[test]
    fn accepts_valid_file() {
        let f = write_tmp(
            r#"{
                "schema_version":1,
                "expectations":[
                  {"name":"a","range":{"start":"0x20000000","end":"0x20001000"},"expect":"safe"},
                  {"name":"b","range":{"start":"0x20001000","end":"0x20002000"},"expect_any_of":["safe","zero"]},
                  {"name":"c","range":{"start":"0x20002000","end":"0x20003000"},"expect_not":"changed","rationale":"why"}
                ]
            }"#,
        );
        let loaded = load_and_validate(f.path(), &one_region(), 0x1000).unwrap();
        assert_eq!(loaded.items.len(), 3);
    }

    fn blocks_in(start: u32, count: u32, block: u32, class: Class) -> Vec<BlockReport> {
        (0..count)
            .map(|i| BlockReport {
                addr: start + i * block,
                class,
                first_diff: None,
            })
            .collect()
    }

    fn mk_loaded(name: &str, start: u32, end: u32, clause: Clause) -> Loaded {
        Loaded {
            items: vec![LoadedExpectation {
                name: name.into(),
                range: AddrRange { start, end },
                clause,
                rationale: None,
            }],
        }
    }

    #[test]
    fn evaluator_pass_single_class() {
        let loaded = mk_loaded("ok", 0x2000_0000, 0x2000_2000, Clause::Single(Class::Safe));
        let blocks = blocks_in(0x2000_0000, 2, 0x1000, Class::Safe);
        let r = evaluate(&loaded, &blocks);
        assert_eq!(r.passed, 1);
        assert_eq!(r.failed, 0);
    }

    #[test]
    fn evaluator_fail_single_class_lists_offending() {
        let loaded = mk_loaded(
            "fail",
            0x2000_0000,
            0x2000_2000,
            Clause::Single(Class::Safe),
        );
        let mut blocks = blocks_in(0x2000_0000, 2, 0x1000, Class::Safe);
        blocks[1].class = Class::Changed;
        let r = evaluate(&loaded, &blocks);
        assert_eq!(r.passed, 0);
        assert_eq!(r.failed, 1);
        match &r.results[0].outcome {
            Outcome::Fail { offending_blocks } => {
                assert_eq!(offending_blocks.len(), 1);
                assert_eq!(offending_blocks[0].addr, 0x2000_1000);
                assert_eq!(offending_blocks[0].class, Class::Changed);
            }
            Outcome::Pass => panic!("expected fail"),
        }
    }

    #[test]
    fn evaluator_any_of() {
        let loaded = mk_loaded(
            "ok",
            0x2000_0000,
            0x2000_2000,
            Clause::AnyOf(vec![Class::Safe, Class::Zero]),
        );
        let mut blocks = blocks_in(0x2000_0000, 2, 0x1000, Class::Safe);
        blocks[1].class = Class::Zero;
        let r = evaluate(&loaded, &blocks);
        assert_eq!(r.passed, 1);
        assert_eq!(r.failed, 0);
    }

    #[test]
    fn evaluator_not() {
        let loaded = mk_loaded(
            "no-changed",
            0x2000_0000,
            0x2000_2000,
            Clause::Not(Class::Changed),
        );
        let blocks = blocks_in(0x2000_0000, 2, 0x1000, Class::Safe);
        let r = evaluate(&loaded, &blocks);
        assert_eq!(r.passed, 1);

        let mut blocks_bad = blocks.clone();
        blocks_bad[0].class = Class::Changed;
        let r2 = evaluate(&loaded, &blocks_bad);
        assert_eq!(r2.failed, 1);
    }
}
