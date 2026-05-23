# Repository Guidelines

## Project Structure & Module Organization

This repository is a Cargo workspace with one member crate:

- `Cargo.toml` defines the workspace and lists `wfc-road-visualizer`.
- `Cargo.lock` pins dependency versions and should be committed for reproducible builds.
- `wfc-road-visualizer/Cargo.toml` defines the Rust 2021 GUI crate using `eframe` and `egui`.
- `wfc-road-visualizer/src/main.rs` contains the entry point, WFC model, random selection logic, and UI code.
- `target/` is Cargo build output and should not be edited or committed.

Add new Rust modules under `wfc-road-visualizer/src/` and declare them from `main.rs` or a future `lib.rs`.

## Build, Test, and Development Commands

- `cargo run -p wfc-road-visualizer` launches the desktop visualizer locally.
- `cargo check` type-checks the whole workspace quickly.
- `cargo build` compiles the workspace in debug mode.
- `cargo test` runs all unit and integration tests once tests are added.
- `cargo fmt` formats Rust code using rustfmt.
- `cargo clippy --all-targets --all-features` runs additional lint checks.

If an environment workaround is added, include a brief comment with the observed failure and rationale, such as avoiding cross-device rename or hardlink errors.

## Coding Style & Naming Conventions

Use standard Rust style: four-space indentation, `snake_case` for functions and variables, `PascalCase` for types, and `SCREAMING_SNAKE_CASE` for constants. Prefer small functions around distinct WFC, rendering, and input-handling responsibilities. Keep comments focused on non-obvious constraints.

Run `cargo fmt` before submitting changes. Treat `cargo clippy --all-targets --all-features` warnings as issues to fix unless there is a clear reason to allow one locally.

## Testing Guidelines

There are currently no tests in the checkout. Add unit tests near the code they exercise using `#[cfg(test)] mod tests`, especially for deterministic WFC behavior such as compatibility calculation, boundary constraints, propagation, and random selection edge cases. Use integration tests under `wfc-road-visualizer/tests/` when behavior spans public crate boundaries.

Run `cargo test` before opening a pull request. For UI-only changes, also run `cargo run -p wfc-road-visualizer` and manually verify startup and interaction.

## Commit & Pull Request Guidelines

No local Git history is available in this directory, so no repository-specific commit convention can be inferred. Use short, imperative commit subjects such as `Add boundary propagation tests` or `Refine tile rendering`.

Pull requests should describe the user-visible change, list validation commands run, and include screenshots or short recordings for visual UI changes. Link related issues when available and call out any known limitations or follow-up work.
