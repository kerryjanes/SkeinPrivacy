# Contributing to Weft

Thanks for your interest in Weft. Contributions of all kinds are welcome — bug reports,
fixes, new features, docs, and tooling.

## Getting set up

```sh
git clone https://github.com/kerryjanes/WeftNetwork.git
cd Weft
pnpm install            # JS workspaces (services, sdk, desktop)
cargo build --release   # host crates
cargo test --workspace  # run the test suite
```

The on-chain programs use Anchor; build them with `anchor build` and test with `cargo test`
(the program test suites run on an in-process Solana VM).

## Workflow

1. Open an issue describing the change before large work, so we can agree on the approach.
2. Branch from `main`, keep changes focused, and add tests for new behavior.
3. Make sure everything is green before opening a PR:
   - `cargo test --workspace`
   - `cargo clippy --workspace -- -D warnings`
   - `cargo fmt --check`
   - `pnpm -r build` and `pnpm format:check`
4. Open a pull request against `main` with a clear description and rationale.

## Style

- Rust: `rustfmt` defaults; no Clippy warnings.
- TypeScript: Prettier (100 cols, single quotes); TypeScript `strict` mode.
- Commits: short, imperative, one line. Group logically-related changes.

## Reporting security issues

Do **not** open a public issue for security vulnerabilities. See [SECURITY.md](SECURITY.md).
