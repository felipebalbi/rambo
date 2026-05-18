# rambo

[![CI](https://github.com/felipebalbi/rambo/actions/workflows/ci.yml/badge.svg)](https://github.com/felipebalbi/rambo/actions/workflows/ci.yml)
[![Release](https://github.com/felipebalbi/rambo/actions/workflows/release.yml/badge.svg)](https://github.com/felipebalbi/rambo/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust 2024](https://img.shields.io/badge/rust-2024-orange.svg)](https://blog.rust-lang.org/)

> **R**OM **A**rea **M**emory **B**ehavior **O**bserver — a CLI for mapping the
> SRAM collateral damage a Cortex-M boot ROM leaves behind.

`rambo` connects to a target MCU over SWD using [probe-rs], primes its on-chip
SRAM with a known pattern, issues a reset, halts the core *before* it executes
any user firmware, and then classifies every block of RAM to show you exactly
what the boot ROM clobbered, what it left alone, and what looks suspiciously
like leftover data structures.

It is intentionally **not** a TUI — output is plain stdout with ANSI colors,
designed to be readable in a terminal, piped to a file, or pasted into a bug
report.

## Why?

When you write low-level firmware (bootloaders, secure-boot stages, RTOS ports),
it matters which parts of RAM are safe to use at startup. Vendor reference
manuals may not document which scratch areas the on-chip ROM uses, and the
layout often differs between silicon revisions. `rambo` answers the question
empirically: *"if I want my firmware's first instruction to find untouched
memory, which addresses can I trust?"*

## How it works

For each RAM region reported by probe-rs's chip database:

1. **Write** a deterministic pattern (`addr-as-data`: each 32-bit word stores
   its own address) across the entire region.
2. **Reset** the target and **halt** it immediately on the vector catch, before
   any user code runs.
3. **Read back** the region and **classify** each block:
   - **SAFE** — pattern survived intact (ROM did not touch this block).
   - **ZERO** — block was zeroed.
   - **ONES** — block was filled with `0xFF`.
   - **CHANGED** — block was rewritten with something else (likely ROM scratch /
     stack / data structures).
4. **Render** a heatmap, a run-length summary, and per-region totals.

Optional modes layer extra analyses on top of this core loop:

- **Fingerprint** changed blocks (`--fingerprint`) — detect constants, dominant
  values, ascending counters, repeating motifs, address-with-offset patterns, or
  noise.
- **Dual-pattern** sweep (`--dual-pattern`) — write `addr` then `!addr` in two
  separate reset cycles to distinguish *undriven* RAM from *actively modified*
  RAM.
- **Write-readback** (`--write-readback`) — no reset between write and read;
  detects aliasing and unmapped windows in reserved address ranges.
- **Stability** (`--reset-cycles N`) — re-read after each of N resets to see
  whether post-ROM state is deterministic or drifts.

## Installation

### From source

```sh
cargo install --path .
```

### Pre-built binaries

Pre-built binaries for Linux, macOS (Intel + Apple Silicon), and Windows are
attached to each tagged release on the [Releases page].

### System dependencies

`probe-rs` needs `libudev` on Linux:

```sh
sudo apt-get install -y libudev-dev pkg-config   # Debian/Ubuntu
sudo dnf install -y systemd-devel pkgconf-pkg-config  # Fedora
```

## Quick start

Survey all RAM regions of an MCXA276 with default settings (4 KiB blocks, single
reset cycle):

```sh
rambo --chip MCXA276
```

Use a smaller block size and fingerprint anything ROM modified:

```sh
rambo --chip MCXA276 --block 0x200 --fingerprint
```

Decide whether a region is driven or undriven via the dual-pattern sweep:

```sh
rambo --chip MCXA276 --dual-pattern
```

Detect aliasing or unmapped windows in a reserved RAM range (no reset between
write and read):

```sh
rambo --chip MCXA276 --write-readback
```

Check determinism across five resets:

```sh
rambo --chip MCXA276 --reset-cycles 5
```

Select a specific debug probe by VID:PID or serial:

```sh
rambo --chip MCXA276 --probe 0483:374b
```

## CLI reference

| Flag                 | Default  | Description                                                                                                  |
|----------------------|----------|--------------------------------------------------------------------------------------------------------------|
| `--chip <NAME>`      | —        | Required. probe-rs target identifier (e.g. `MCXA276`).                                                       |
| `--block <BYTES>`    | `0x1000` | Classification block size. Decimal / `0x` / `0o` / `0b` / `_` accepted. Must be a non-zero multiple of 4.    |
| `--reset-cycles <N>` | `1`      | Number of read-after-reset iterations. `N>1` enables stability analysis. Incompatible with `--dual-pattern`. |
| `--fingerprint`      | off      | Fingerprint each CHANGED block (bit density, top values, pattern matches).                                   |
| `--dual-pattern`     | off      | Two-pass `addr` / `!addr` sweep to distinguish undriven from modified RAM.                                   |
| `--write-readback`   | off      | Write then immediately read back (no reset). Reveals aliasing / unmapped windows.                            |
| `--probe <SEL>`      | first    | Probe selector, `VID:PID` or serial.                                                                         |
| `--json <PATH>`      | off      | Write a machine-readable JSON report to `<PATH>` (use `-` for stdout, which suppresses normal text output).  |
| `--expectations <PATH>` | off   | Load a "RAM contract" JSON file and evaluate each expectation against the survey. Non-zero exit on failure.  |

Run `rambo --help` for the full clap-generated help text.

## Classification legend

| Class   | Glyph     | Meaning                                                     |
|---------|-----------|-------------------------------------------------------------|
| SAFE    | █ green   | Pattern survived. ROM did not touch this block.             |
| ZERO    | █ blue    | Block was cleared to `0x00000000`.                          |
| ONES    | █ magenta | Block was filled with `0xFFFFFFFF`.                         |
| CHANGED | █ red     | Block was rewritten with something else (ROM working data). |

The heatmap is laid out as **64 cells per row**, where each cell aggregates one
block (1 KiB for the survey heatmap, `--block` bytes elsewhere). Below the
heatmap, `rambo` prints a run-length-encoded list of contiguous regions sharing
a class, followed by totals.

## Output anatomy

A typical survey output for a single region looks like:

```
═══ RAM @ 0x20000000 .. 0x2003c000 (240 KiB) ═══

[heatmap, 64 cells per row]

Runs
┌────────────────────────┬─────────┬──────────┐
│ Range                  │ Size    │ Class    │
├────────────────────────┼─────────┼──────────┤
│ 0x20000000..0x20001000 │   4 KiB │ CHANGED  │
│ 0x20001000..0x2003c000 │ 236 KiB │ SAFE     │
└────────────────────────┴─────────┴──────────┘

Totals
  SAFE:    236 KiB
  CHANGED:   4 KiB
```

## CI gate: JSON output and RAM contracts

`rambo` can emit a stable JSON report (`--json`) and evaluate a JSON
**RAM contract** (`--expectations`) against the survey. Together they
turn `rambo` from a one-off diagnostic into a regression gate that
catches when a new chip rev, silicon lot, or ROM patch clobbers memory
your firmware relies on.

```sh
rambo --chip MCXA276 \
      --json report.json \
      --expectations contract.json
```

Exit code is `0` if every expectation passes, `1` if any fails. Both
flags are independent — use `--json` alone to archive a CI artifact,
or `--expectations` alone for a pass/fail check.

A contract file declares per-range expectations; see
[`examples/expectations.json`](examples/expectations.json) for a
worked example. Each expectation specifies exactly one clause:

| Clause            | Meaning                                                  |
|-------------------|----------------------------------------------------------|
| `expect: "safe"`  | Every block in the range must classify as that class.    |
| `expect_any_of: ["safe", "zero"]` | Every block must classify as one of these. |
| `expect_not: "changed"` | No block in the range may classify as that class.  |

Ranges must be block-aligned (`--block`) and fit entirely inside one
of the chip's RAM regions; misalignment or out-of-region addresses
are rejected *before* any probe I/O happens, so a typo can never
brick a run.

JSON output is wire-stable: `schema_version: 1` will keep its current
shape, and breaking changes will bump the schema version alongside a
release.

## Hardware notes

`rambo` has been smoke-tested against an **NXP MCXA276** development board with
the on-board MCU-Link probe. Any Cortex-M chip supported by [probe-rs] should
work, but only chips whose target description in probe-rs exposes RAM regions in
its `memory_map` will be useful (which is essentially all of them).

> ⚠️ The CMSIS-Pack chip descriptions that probe-rs consumes occasionally list
> code-bus *aliases* of the system-bus RAM as separate regions. `rambo` treats
> every region in the map as independent — if you see the same physical RAM
> surveyed twice under different addresses, that is why.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

CI runs the same three checks on every push and PR (Ubuntu/macOS/Windows for
tests, Ubuntu for lints). The CI badges at the top of this README reflect the
current status of `main`.

### Commit convention

Commits to `main` MUST follow [Conventional Commits]. `release-plz` parses
them to compute the next version bump and to generate `CHANGELOG.md`.
Non-conforming commits default to a patch bump and won't appear in the
changelog cleanly.

Bump rules differ between `0.x` and `1.x` (per the [`next_version`] crate
that release-plz uses):

| Commit                                  | `0.x` bump | `≥1.0` bump |
|-----------------------------------------|------------|-------------|
| `fix:` / `perf:` / `feat:`              | patch      | patch / patch / minor |
| `feat!:` or `BREAKING CHANGE:` footer   | minor      | major       |
| non-conventional                        | patch      | patch       |

On `0.x` the minor digit *is* the breaking axis (per Cargo's semver
rules), so `feat:` deliberately stays on the patch line until you bump
to `1.0.0` manually.

### Releases

Releases are automated via [release-plz]:

1. Merging conventional commits to `main` causes release-plz to open (or
   update) a **"chore: release"** PR that bumps `Cargo.toml` and updates
   `CHANGELOG.md`.
2. Merging that PR tags `vX.Y.Z`, creates a GitHub Release, and publishes
   to crates.io.
3. The tag also triggers `release.yml`, which builds per-OS binary
   archives and uploads them onto the same GitHub Release.

## License

[MIT](LICENSE) © Felipe Balbi

[probe-rs]: https://probe.rs
[Releases page]: https://github.com/felipebalbi/rambo/releases
[Conventional Commits]: https://www.conventionalcommits.org/
[release-plz]: https://release-plz.dev
[`next_version`]: https://docs.rs/next_version
