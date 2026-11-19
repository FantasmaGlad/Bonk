# Contributing to Bonk

Bonk erases disks: a mistake here costs someone their data. Issues and pull requests may be written in English or French.

Open an issue first for anything beyond a small fix, such as a new feature, a behavior change or a new dependency. A pull request carries one logical change, and a fix does not carry a refactoring. Code specific to an operating system stays in `bonk-hal`.

Never test on a real disk you care about. Use `bonk write --simulate`, image files or a virtual machine.

Safety is not negotiable. A system disk is never offered or written, and a change must not weaken the filter or the identity check made before writing.

A fixed bug keeps a test that failed before the fix. No test depends on the clock, the real network or a real disk. Say what you did not verify, such as a system, a version or a piece of hardware: a simulated run is not a real one.

Error codes, exit codes, JSON schemas and the service protocol are stable contracts: they do not change without a version bump. A new dependency explains its need, the alternatives and its license, which must be compatible with MIT (`cargo deny check`).

The checks a pull request must pass:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test -p bonk --no-default-features --locked
cargo deny check
```

The BIOS boot code is edited in `boot/*.s`, then `boot/assemble.sh` regenerates `crates/bonk-core/src/boot/*.bin` and `boot/test-*.bin`, which are committed with it. Only the Rust toolchain is used.

After a change to `crates/bonk/assets/bonk-logo.png`, `python3 .github/scripts/icones.py` (which needs Pillow) regenerates the icons, which are committed with it.

Commits follow Conventional Commits, in lower case, with a scope: `fix(parser): reject truncated headers`.

Contributions are accepted under the [MIT license](LICENSE). Security issues follow [SECURITY.md](SECURITY.md).
