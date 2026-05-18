<!--
Thanks for contributing to rambo!

PR title MUST be a Conventional Commit (feat:, fix:, refactor:, docs:, ci:, chore:, test:, style:, perf:).
Squash-merge uses your PR title as the commit subject, which release-plz reads.

See AGENTS.md for the full set of conventions.
-->

## Why

<!-- What's the user-visible problem or motivation? One short paragraph. -->

## What

<!-- High-level summary of the change. Bullet points are fine. -->

## How to verify

<!--
Reviewer should be able to run these commands locally.
Include any `rambo` invocations that exercise the change.
-->

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

## Checklist

- [ ] PR title is a Conventional Commit (`feat:`, `fix:`, `refactor:`, ...).
- [ ] `cargo fmt`, `cargo clippy -D warnings`, `cargo test` all pass locally.
- [ ] Added or updated tests for every new pure function / branch.
- [ ] Updated `README.md` if user-visible behavior changed (in the same PR, not a follow-up).
- [ ] No new `unwrap`/`expect`/`eprintln!`/`dbg!`/`todo!`/`unimplemented!` in non-test code.
- [ ] No new dependencies — or if there are, justified in "Why" above.
- [ ] If this changes a CLI flag or JSON field that has already shipped in a tagged release, it is marked `feat!:`/`refactor!:` and the impact is described below.
- [ ] If this changes `src/report.rs` or `src/expectations.rs`, the wire-format implications are stated below (additive vs. schema bump).
- [ ] AI-assisted commits include the `Co-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>` trailer.

## Wire / CLI stability impact

<!--
Delete this section if your PR doesn't touch the CLI surface or the JSON schema.
Otherwise:
- Which flag/field changed?
- Is the change additive or breaking?
- Has the changed surface already shipped in a tagged release?
- If breaking + released: did you bump SCHEMA_VERSION / use a `!` commit?
-->

## Hardware testing

<!--
Delete if your PR doesn't touch src/probe.rs or src/io.rs.
Otherwise list the chip(s) + probe(s) you smoke-tested on, or say "no hardware available — needs a reviewer with $CHIP".
-->

## Related issues

<!-- Closes #N, refs #M -->
