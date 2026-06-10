# jsknown

`jsknown` is an open source JavaScript asset recovery tool for security research, source map reversal, and AI-agent-ready code review workflows.

It runs a local ingestion server, receives browser/proxy traffic from the included Burp Suite extension, mirrors JavaScript and HTML assets to disk, beautifies minified code, discovers lazy-loaded chunks, fetches source maps, and reconstructs original source trees when source maps contain `sourcesContent`.

## Features

- URL-mirrored directory structure for captured HTML and JavaScript.
- Automatic JS and HTML beautification without overwriting originals.
- Webpack, Vite, and Next.js chunk discovery.
- Recursive lazy-loaded chunk fetching with configurable rate limiting.
- Automatic source map discovery from comments, headers, inline data URLs, and sibling `.map` URLs.
- Source map reversal into original source layout with safe path sanitization.
- JXScout-inspired AST findings for security-oriented JavaScript review.
- Agent-ready JSONL metadata for assets, relationships, and findings.
- Burp Suite extension that streams matching responses into the local server.
- GitHub Actions for tests, multi-platform binaries, plugin JAR builds, and releases.

## Quick Start

```bash
cargo install --path crates/jsknown-cli
jsknown serve --project default
```

Then load the Burp Suite plugin JAR and keep the default server URL:

```text
http://127.0.0.1:3333
```

Captured projects are written to:

```text
~/jsknown/<project>
```

See [INSTALL.md](INSTALL.md) for full installation and Burp setup instructions.

## Project Layout

```text
original/                 Raw captured HTML and JavaScript
beautified/               Formatted versions for review
sourcemaps/raw/           Downloaded or extracted source maps
sourcemaps/reversed/      Recovered original sources
analysis/                 Per-file AST analysis JSON
metadata/assets.jsonl     Asset index for tools and AI agents
metadata/relationships.jsonl
metadata/findings.jsonl
```

## CLI

```bash
jsknown serve \
  --host 127.0.0.1 \
  --port 3333 \
  --project default \
  --rate-per-second 2 \
  --fetch-concurrency 5
```

Useful flags:

- `--scope <pattern>` filters captured URLs. Can be repeated.
- `--output <path>` changes the output root.
- `--rate-per-second <n>` controls chunk and source map fetches.
- `--rate-per-minute <n>` applies a minute-level cap.
- `--fetch-concurrency <n>` controls concurrent follow-up downloads.
- `--max-body-bytes <n>` rejects oversized captured responses.

## Security Notes

`jsknown` stores application code and metadata locally. Only run it for systems you are authorized to test. The chunk discovery engine does not execute arbitrary JavaScript; it uses static parsing and constrained string evaluation for known bundler runtime shapes.

## License

MIT. See [LICENSE](LICENSE).
