//! Probe attach and RAM region discovery.
//!
//! All probe-rs session setup lives here so the rest of the crate
//! never has to touch `probe-rs::probe::list::Lister` or the chip
//! configuration types directly.

use color_eyre::eyre::{Result, eyre};
use owo_colors::OwoColorize;
use probe_rs::config::MemoryRegion;
use probe_rs::probe::list::Lister;
use probe_rs::{Permissions, Session};

use crate::cli::Cli;

/// A single RAM region of interest. Address fields are `u32` because
/// every Cortex-M target we care about sits comfortably in the 4 GiB
/// address space.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RamRange {
    pub name: String,
    pub start: u32,
    pub end: u32,
}

impl RamRange {
    pub fn len(&self) -> u32 {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.end == self.start
    }
}

/// Open a probe-rs session against the chip named in `cli`.
///
/// If `--probe` is set, the first probe whose identifier contains the
/// selector (or whose serial number matches exactly) is used.
/// Otherwise the first probe found is used.
pub fn open_session(cli: &Cli) -> Result<Session> {
    let lister = Lister::new();
    let probes = lister.list_all();
    if probes.is_empty() {
        return Err(eyre!("No probes found"));
    }
    let probe_info = match &cli.probe {
        Some(sel) => probes
            .iter()
            .find(|p| p.identifier.contains(sel) || p.serial_number.as_deref() == Some(sel))
            .ok_or_else(|| eyre!("Probe '{sel}' not found"))?,
        None => &probes[0],
    };
    println!("Using probe: {}", probe_info.identifier.bold());
    let probe = probe_info.open()?;
    let session = probe.attach(cli.chip.clone(), Permissions::default())?;
    Ok(session)
}

/// Walk the chip's declared memory map and return every RAM region.
///
/// The information comes straight from the YAML target description
/// shipped with probe-rs (or any custom target added via
/// `Lister::with_targets`), so as long as the chip is supported there
/// is no need for the user to pass `--ram-start`/`--ram-end`.
pub fn discover_ram_regions(session: &Session) -> Vec<RamRange> {
    session
        .target()
        .memory_map
        .iter()
        .filter_map(|region| match region {
            MemoryRegion::Ram(r) => Some(RamRange {
                name: r.name.clone().unwrap_or_else(|| "RAM".to_string()),
                start: r.range.start as u32,
                end: r.range.end as u32,
            }),
            _ => None,
        })
        .collect()
}

/// Clip a region down to the largest sub-range whose length is a
/// multiple of `block` bytes, starting at the original `start`.
///
/// Returns `None` if the region cannot accommodate even one full
/// block. Trailing bytes that wouldn't form a full block are dropped
/// silently.
pub fn clip_to_block(region: &RamRange, block: u32) -> Option<RamRange> {
    assert!(block > 0, "block must be > 0");
    let len = region.len();
    let kept = (len / block) * block;
    if kept == 0 {
        return None;
    }
    Some(RamRange {
        name: region.name.clone(),
        start: region.start,
        end: region.start + kept,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rr(start: u32, end: u32) -> RamRange {
        RamRange {
            name: "T".into(),
            start,
            end,
        }
    }

    #[test]
    fn clip_exact_multiple_unchanged() {
        let r = rr(0x2000_0000, 0x2000_4000); // 16 KiB
        let c = clip_to_block(&r, 0x1000).unwrap();
        assert_eq!(c, rr(0x2000_0000, 0x2000_4000));
    }

    #[test]
    fn clip_drops_trailing_partial_block() {
        let r = rr(0x2000_0000, 0x2000_4500); // 16 KiB + 0x500
        let c = clip_to_block(&r, 0x1000).unwrap();
        assert_eq!(c.end, 0x2000_4000);
    }

    #[test]
    fn clip_too_small_returns_none() {
        let r = rr(0x2000_0000, 0x2000_0800); // 2 KiB
        assert!(clip_to_block(&r, 0x1000).is_none());
    }
}
