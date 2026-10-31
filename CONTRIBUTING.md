# Contributing to Bonk

Bonk erases disks: a mistake here costs someone their data. Issues and pull requests may be written in English or
French.

## Before you start

- Open an issue first for anything beyond a small fix: new feature, behavior change, new dependency.
- One logical change per pull request. A fix does not carry a refactoring.
- Read [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Code specific to an operating system stays in `bonk-hal`.

## Rules

- **Never test on a real disk you care about.** Use `bonk write --simulate`, image files or a virtual machine.
- **Safety is not negotiable.** A system disk is never offered or written; a change must not weaken the filter or the
  identity check before writing.
- **A test first.** A fixed bug keeps a test that failed before the fix. No test depends on the clock, the real network
  or a real disk.
- **Say what you did not verify:** system, version, hardware. A simulated run is not a real one.
- **Stable contracts:** error codes, exit codes, JSON schemas and the service protocol do not change without a version
  bump.
- **New dependency:** explain the need, the alternatives and the license (MIT compatible; `cargo deny check`).

## Checks

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test -p bonk --no-default-features --locked
cargo deny check
```

Commits follow Conventional Commits, in lower case, with a scope: `fix(parser): reject truncated headers`.

Contributions are accepted under the [MIT license](LICENSE). Security issues: [SECURITY.md](SECURITY.md).
