# Installing jsknown

## Requirements

- Rust 1.96 or newer.
- Burp Suite Professional or Community with support for Montoya extensions.
- Java 17 or newer for building the Burp plugin.

## Install From Source

```bash
git clone https://github.com/davi-1337/jsknown.git
cd jsknown
cargo install --path crates/jsknown-cli
```

Start the local server:

```bash
jsknown serve --project default
```

The server listens on `127.0.0.1:3333` by default.

## Build The Burp Suite Plugin

```bash
cd burp-plugin
gradle build
```

The plugin JAR is created at:

```text
burp-plugin/build/libs/jsknown-burp.jar
```

## Load The Plugin In Burp Suite

1. Open Burp Suite.
2. Go to `Extensions`.
3. Add a Java extension.
4. Select `jsknown-burp.jar`.
5. Confirm the server URL is `http://127.0.0.1:3333`.
6. Browse the target application through Burp.

`jsknown` will capture HTML and JavaScript, then process chunks, source maps, reversed sources, and AST findings.

## Build Release Artifacts Locally

```bash
cargo build --release --workspace
cd burp-plugin
gradle build
```

The Rust binary is located at:

```text
target/release/jsknown
```

The Burp plugin is located at:

```text
burp-plugin/build/libs/jsknown-burp.jar
```

## Troubleshooting

- If no files are written, check that Burp is proxying traffic and the plugin is enabled.
- If the plugin reports connection failures, run `jsknown serve` and verify `GET /health`.
- If chunks are missing, lower rate limits or increase `--fetch-concurrency`.
- If reversed sources are sparse, the source map may not include `sourcesContent`.
