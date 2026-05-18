# AGENTS.md

Guidance for human contributors *and* AI coding agents (Copilot, Claude,
Cursor, Aider, etc.) working on `rambo`.

This file is intentionally prescriptive. It encodes the conventions
that keep CI green, releases reproducible, and reviews short. Read it
before opening a PR.

---

## 1. What `rambo` is

A small Rust CLI + library that surveys the post-reset state of an
MCU's SRAM via [probe-rs]. It is **hardware-adjacent** but the bulk
of the code is pure logic that *must* be unit-testable without a
probe.

- Edition: **2024**.
- MSRV: see `rust-version` in `Cargo.toml` (currently **1.95**). Do not bump it without justification.
- Binary + library crate (`src/main.rs` + `src/lib.rs`).
- All hardware I/O is confined to `src/probe.rs` and `src/io.rs`. Everything else is pure.

## 2. Golden rules (the "do not break these" list)

1. **Never `unwrap`/`expect` in non-test code.** `clippy::unwrap_used` and `clippy::expect_used` are `deny`. Use `?` with `color_eyre::Result` and `.map_err(...)`/`.ok_or_else(...)` for context.
2. **Never `eprintln!` / `dbg!` / `todo!` / `unimplemented!`.** All denied by clippy. Use `println!` for user-facing output, `tracing` is not currently wired in.
3. **`unsafe` is forbidden** (`#![forbid(unsafe_code)]` at the crate level via `[lints.rust]`).
4. **Do not add new dependencies without a clear justification.** Prefer `std`. If you must add one, explain why in the PR description and prefer crates already in our transitive tree (check `Cargo.lock`).
5. **Keep `src/probe.rs` the only file that talks to probe-rs.** Anything testable without a chip belongs elsewhere.
6. **All public functions returning errors must use `color_eyre::eyre::Result`.** Do not introduce custom error types unless the API surface justifies it.
7. **The JSON wire format is stable.** See §7.
8. **The CLI surface is stable.** See §6.

## 3. Code style

- `cargo fmt` is the source of truth — never hand-format.
- Comments only where the *why* isn't obvious from the code. Doc comments (`///`) on every `pub` item.
- Prefer iterators over index loops; prefer pattern matching over chained `if`s.
- Use `owo_colors` (`OwoColorize`) for terminal colors, never raw ANSI escapes.
- Use `comfy_table` for tabular output.
- New numeric CLI args go through `parse_int` so users can write `0x1000` / `0b1010` / `1_024`.
- **Quiet mode**: any new print site in the JSON-output path must check `crate::render::is_quiet()` and early-return, so `--json -` stays parseable.

## 4. Testing

- `cargo test --all-features` must stay green. We currently have 59+ tests and growing.
- **Every new pure function gets a unit test.** No exceptions for "trivial" logic.
- Tests may use `unwrap` (allowed via `#![cfg_attr(test, allow(clippy::unwrap_used))]` in `lib.rs`).
- For file-based tests use `tempfile::NamedTempFile`; never write into the repo or `/tmp` directly.
- No mocking of `probe-rs`. Hardware code is tested manually; everything else must be reachable from a unit test by construction.
- Property tests are welcome via `proptest` if it earns its weight, but ask first.

## 5. Commit conventions

We use [Conventional Commits] and `release-plz` reads them to bump
versions and write `CHANGELOG.md`.

**Bump rules on `0.x` (where we are now):**

| Commit                                  | Bump  |
|-----------------------------------------|-------|
| `fix:` / `perf:` / `feat:`              | patch |
| `feat!:` or `BREAKING CHANGE:` footer   | minor |
| `refactor:` / `chore:` / `docs:` / `ci:`/ `test:` / `style:` | none |
| non-conventional                        | patch |

On `0.x`, the **minor** digit is the breaking axis (per Cargo's
semver rules), so `feat:` deliberately stays on the patch line.

**Critical guidance for marking breaking changes:**

- Use `!` (e.g. `feat!:`, `refactor!:`) **only** when the change breaks a behavior that has been released. Renaming a CLI flag that was added after the last `vX.Y.Z` tag and never shipped is **not** breaking — there are no users to break.
- Check first: `git log vLAST..HEAD -- <files>`. If the symbol/flag/API only exists in unreleased commits, drop the `!`.

**Trailers:**

- AI-generated or AI-assisted commits MUST include:
  ```
  Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>
  ```
- Use `Signed-off-by:` if you DCO-sign; not required.

**One logical change per commit.** Refactors and feature additions go in separate commits even if you push them together.

## 6. CLI stability

The flags listed in the README's "CLI reference" table are part of
the public contract. Adding new flags is fine; renaming or removing
one needs `feat!:` / `refactor!:` *after* it has shipped in a tagged
release.

Argument value semantics (defaults, accepted radixes, alignment
rules) are also contractual. Document any change in the commit body
and the README in the same commit.

## 7. JSON wire format stability

`src/report.rs` defines a wire-stable schema (`SCHEMA_VERSION`,
currently `1`). Rules:

- **Never** rename or remove a field at the current schema version.
- Additive changes (new optional fields, new enum variants in a `#[serde(other)]`-safe spot) are allowed but should be called out in the PR.
- **Breaking** changes (rename, remove, retype, semantics shift) require bumping `SCHEMA_VERSION` and updating every test that pins the previous shape.
- Address fields are serialized as uppercase 8-digit hex strings (`"0x20000000"`) via `src/serde_hex.rs`. Do not add bare-`u32` address fields.
- `Class` is serialized lowercase. Do not change that without bumping the schema.

The same rules apply to the expectations file format
(`src/expectations.rs`): `schema_version: 1` is stable.

## 8. Hardware-touching code

If your change touches `src/probe.rs`, `src/io.rs`, or anything that
makes a probe-rs call:

- State in the PR which board(s) you smoke-tested on (chip + probe).
- Never lock the user into a specific probe; honor `--probe`.
- Never assume more than one core; `rambo` is Cortex-M-single-core today.
- Reset behavior: always halt on vector catch before running any user code.

If you cannot smoke-test, say so explicitly and tag a reviewer who can.

## 9. Release process (don't run this yourself)

Releases are fully automated by `release-plz` (see
`.github/workflows/release-plz.yml` and `release-plz.toml`):

1. Conventional commits merged to `main` cause release-plz to open or update a `chore: release` PR.
2. Merging that PR tags `vX.Y.Z`, creates a GitHub Release, and publishes to crates.io.
3. The tag triggers `release.yml`, which builds per-OS binaries.

**You should not** edit `Cargo.toml`'s `version`, write `CHANGELOG.md`
entries by hand, or push tags manually. Let release-plz do it.

## 10. Pre-flight checklist (run before every push)

```sh
cargo fmt --all
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

CI runs the same three on Linux/macOS/Windows. There is no `--fix it
in CI` step — get it green locally first.

For changes that touch the public API surface (anything `pub` in
`lib.rs` or its modules), also run:

```sh
cargo install cargo-semver-checks   # one-time
cargo semver-checks check-release
```

CI runs `cargo-semver-checks` on every PR; a failure there means you
need a `feat!:`/`refactor!:` commit (only if released — see §5).

## 11. Pull request etiquette

- Title = the commit subject (Conventional Commits).
- Description should state **why**, then **what**, then **how to verify**.
- Keep PRs small. <300 LoC diff is ideal. If yours is bigger, justify it.
- Update `README.md` in the **same PR** as user-visible behavior changes — never in a separate "docs" follow-up.
- Squash-merge is the default; the squash title becomes the commit subject, so make sure your PR title is conventional-commits-compliant.

## 12. Agent-specific notes

If you are an AI agent reading this:

- **Read this whole file before editing.** Skim §2 and §5 at minimum.
- Always run the pre-flight checklist (§10) before reporting completion. Do not claim "all tests pass" without having run them.
- Do not invent new dependencies. If you need one, ask first.
- Do not delete tests to make CI green. If a test fails, the code is wrong (or the test was). Fix the right one.
- Do not bypass clippy with `#[allow(...)]` unless you have a specific, documented reason. Prefer fixing the lint.
- Prefer batching tool calls (parallel reads, parallel edits to different files) over sequential ones.
- When you finish a logical unit of work, commit it with the `Co-authored-by: Copilot` trailer (§5) so reviewers can see what was AI-assisted.
- If you cannot complete the task safely (missing context, ambiguous requirement, would require hardware), stop and ask the human.

---

[probe-rs]: https://probe.rs
[Conventional Commits]: https://www.conventionalcommits.org/
