# Contributing

Thanks for improving `jsknown`.

## Development

```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Build the Burp plugin:

```bash
cd burp-plugin
gradle build
```

## Pull Requests

- Keep changes focused.
- Add tests for new parsing, chunk discovery, source map, and AST behavior.
- Update `README.md` or `INSTALL.md` when user-facing behavior changes.
- Do not commit captured third-party application code.
