//! Low-level Cortex-M debug I/O: halt, reset, and bulk word transfers.
//!
//! Everything goes through `Session::core(0)`. We deliberately wrap
//! short scopes around the borrow because reading and writing the
//! same session multiple times in succession is the common pattern
//! and Rust's borrow checker rejects holding `Core` across resets.

use std::time::Duration;

use color_eyre::eyre::Result;
use probe_rs::{MemoryInterface, Session};

use crate::render::step;

/// Bulk transfer chunk size, in 32-bit words.
///
/// 0x4000 words == 64 KiB per SWD transaction. Large enough to amortize
/// the per-transaction overhead but small enough that we don't keep the
/// link saturated for a noticeably long time before pumping the next
/// chunk.
pub const CHUNK_WORDS: usize = 0x4000;

/// Halt core 0 if it isn't already halted. Cheap no-op when already halted.
pub fn ensure_halted(s: &mut Session) -> Result<()> {
    let mut core = s.core(0)?;
    if !core.core_halted()? {
        core.halt(Duration::from_secs(1))?;
    }
    Ok(())
}

/// Reset core 0 and stop immediately after the boot ROM runs but
/// before any user code executes — that is exactly when we want to
/// observe the post-ROM SRAM state.
pub fn reset_and_halt(s: &mut Session) -> Result<()> {
    let mut core = s.core(0)?;
    core.reset_and_halt(Duration::from_secs(2))?;
    Ok(())
}

/// Read `n_words` 32-bit words starting at byte address `start`.
///
/// Issues bulk reads in `CHUNK_WORDS`-sized batches. Prints a single
/// "reading back" step line so progress is visible on large surveys.
pub fn read_words(s: &mut Session, start: u32, n_words: usize) -> Result<Vec<u32>> {
    step("reading back");
    let mut buf = vec![0u32; n_words];
    let mut core = s.core(0)?;
    for (i, chunk) in buf.chunks_mut(CHUNK_WORDS).enumerate() {
        let addr = start as u64 + (i * CHUNK_WORDS * 4) as u64;
        core.read_32(addr, chunk)?;
    }
    Ok(buf)
}

/// Write `words` starting at byte address `start`, in `CHUNK_WORDS`
/// bulk transfers.
pub fn write_words(s: &mut Session, start: u32, words: &[u32]) -> Result<()> {
    ensure_halted(s)?;
    let mut core = s.core(0)?;
    for (i, chunk) in words.chunks(CHUNK_WORDS).enumerate() {
        let addr = start as u64 + (i * CHUNK_WORDS * 4) as u64;
        core.write_32(addr, chunk)?;
    }
    Ok(())
}
