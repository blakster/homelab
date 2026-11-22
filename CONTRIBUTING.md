# Contributing

- `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` must pass. CI runs the same.
- UI changes: run `cargo test`, review snapshot diffs with `cargo insta review`, and include a screenshot (`render --cells | scripts/screenshot.py`) in the PR.
- Keep `render` pure. Collectors talk to the UI only through `Snapshot`.
- New states belong in `model.rs` as enum variants, not booleans.
- Follow `.interface-design/system.md`: one accent colour, status colours only for status, truncate with `…`, no animation on data.
- Never commit real tailnet data. Fixtures are redacted.
