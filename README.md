# merle

> *Named after **Gayla**, a blue merle Australian Shepherd — fast, brilliant, tireless, and she herds your code.* 🐶


An **all-local, verifier-first** coding CLI. Your model, your machine, nothing sent to the cloud.

The difference vs aider/Cline: `merle` doesn't just ask the model — it **runs your tests to verify
every fix**. It generates candidates, keeps only one that *actually makes the tests pass*, and shows
you the diff. The model is never trusted; the test is.

```
merle fix calc.py --test "pytest -q"          # verified single-file fix (generate → verify → apply → diff)
merle do  "add input validation" --test "cargo test"   # multi-step agentic task
merle explain src/foo.rs                        # plain explanation
```

## How it works (one engine, many faces)
```
merle  →  localize (callsieve)  →  local model  →  best-of-N  →  run your tests  →  apply + diff
```
It talks to a local MLX model server (default `http://localhost:8080`, set `MERLE_BASE` to change).
Pair it with the GLM-5.2-Demolition model + serve, or any OpenAI-compatible local endpoint.

### Choosing a backend

`mlx_lm.server` (Apple's reference implementation) works but is explicitly not built for production use:
no continuous batching, no cross-call prompt caching, HTTP/1.0 only. For anything beyond quick one-off
fixes, point `MERLE_BASE` at a backend that actually implements batching and prefix caching — that's
where the real speed lives for merle's workload (many small verify/repair calls to the same model, same
repo context, back-to-back), not in the model itself. [oMLX](https://omlx.ai/) is a strong current option:
continuous batching, a two-tier RAM+SSD KV cache that survives restarts, and it's built directly on
`mlx-lm` so most models that load in `mlx_lm.server` should load there too.


## Surfaces — one engine, three faces
merle is the *engine* (localize → model → best-of-N → verify → repair). You reach it however you like:
- **CLI** — `merle fix / do / explain` — *available now*
- **Desktop** — a native SwiftUI macOS app — *planned*
- **VS Code** — an extension — *planned*

All three are thin, native clients that talk to the same local engine; the model runs on your machine.


## Batteries included — one binary
`merle` bundles **callsieve** (tree-sitter-based code retrieval / bug localization) as an embedded
library, so `cmd_do` gets task-relevant repo context without a separate service. `cargo build --release`
compiles it *into* the single `merle` binary — no separate downloads, no Python runtime. Only the MLX
model server runs as a separate local service.

## Install
```
git clone <this repo> && cd merle
cargo install --path .   # or: cargo build --release && ln -s "$PWD/target/release/merle" /usr/local/bin/merle
```
Requires a Rust toolchain to build, and a running local model server to use. Cross-platform;
Apple-silicon-native when paired with an MLX serve.

## Status
Early but proven — `fix` verifies real bugs end-to-end. Roadmap: richer `do` agent loop, git `--commit`,
multi-file fixes, then a native SwiftUI desktop app over the same engine.
