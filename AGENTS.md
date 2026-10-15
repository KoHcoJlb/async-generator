# Repository guidance

## Verification

Run from the repository root:

- `cargo test --workspace --locked` covers both crates, including runtime regressions for single and concurrent sends inside `FuturesUnordered` and yields during generator completion.
- `cargo check --locked --lib --no-default-features` checks the runtime without proc macros. Keep `--lib`: `examples/test1.rs` uses macros and is not feature-gated.
- `cargo run --locked --example test1` is the existing manual smoke check: prints `Some(Ok(123))`, `Some(Ok(45))`, then repeated `None`, with a three-second delay. It has no assertions and currently emits unused-code/import warnings.
- `cargo fmt --all -- --check` checks formatting. The import grouping/granularity settings in `rustfmt.toml` require nightly rustfmt; stable warns and ignores them.
- `cargo test --locked --doc --no-default-features` checks the README examples with macros disabled. The README is included as crate-level documentation.
- Rust 1.88 is the minimum supported version. Verify with `cargo +1.88.0 test --workspace --locked` and `cargo +1.88.0 check --locked --lib --no-default-features` when changing compiler requirements.

## Publishing

- Both crates inherit the license, repository URL, and minimum Rust version from `[workspace.package]`. Keep the runtime's version requirement for `async-generator-macros` compatible with the macro package version; retain its local `path` for development.
- Use a current Cargo for `cargo publish --workspace --locked --dry-run --allow-dirty`. It verifies both packages together, including the unpublished macro dependency. Inspect archive contents with `cargo package --workspace --locked --list --allow-dirty`.
- In the isolated agent environment, the masked `.idea` directory can make packaging fail even with `include` configured. Verify from a temporary copy of the manifests, lockfile, source directories, and READMEs instead.
- After committing the release changes, `cargo publish --workspace --locked` publishes both crates in dependency order. A dry run does not reserve names or publish anything.

## Runtime and macro boundary

- The root `async-generator` crate and the `macros/` proc-macro crate form one workspace. Use `--workspace` for checks intended to cover both packages.
- `src/lib.rs` exports functions and macros with the same names (`generator`, `try_generator`). The default `macros` feature controls only the macro re-exports/dependency; the closure-based runtime API remains available without it.
- `macros/src/lib.rs` rewrites `yield expr` to `yield_tx.send(expr).await` and wraps the body in an async closure passed to the runtime. Expansion hard-codes `async_generator::`; dependency renaming is not handled. Bare `yield;` is unsupported because the visitor unwraps its expression.
- `try_generator` bodies must return `Result<(), E>`; yielded values become `Ok(value)`, and a terminal error becomes one final `Err(error)` item. A value yielded during the completion poll is returned first, with the terminal result saved for the next poll. Completion is latched so the future is never polled again; after the terminal result is consumed, subsequent polls return `None`.

## Yield handoff invariants

- `src/yielder.rs` is a thread-local, single-slot handoff, not a channel. `GeneratorImpl::poll_next` installs a pointer to its stack-local output slot only while polling the generator future; the scope guard restores the previous thread-local state for nesting/unwinding.
- Preserve the sender/receiver ID check before the unsafe pointer cast. Polling a send future outside its matching generator scope panics; do not move sends into independently polled tasks.
- `send(value)` must be awaited and currently requires `T: Unpin`. Its future wakes the polling task whenever it returns `Pending`, so wake-driven combinators can resume it. If the slot is occupied, it retains the value for a later poll.
