# merle

> *Named after **Gayla**, a blue merle Australian Shepherd — fast, brilliant, tireless, and she herds your code.* 🐕


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


## Surfaces — one engine, three faces
merle is the *engine* (localize → model → best-of-N → verify → repair). You reach it however you like:
- **CLI** — `merle fix / do / explain` — *available now*
- **Desktop** — a native SwiftUI macOS app — *planned*
- **VS Code** — an extension — *planned*

All three are thin, native clients that talk to the same local engine; the model runs on your machine.

## Install
```
git clone <this repo> && cd merle
ln -s "$PWD/merle.py" /usr/local/bin/merle   # or add to PATH
```
Requires Python 3.11+ and a running local model server. Cross-platform; Apple-silicon-native when paired
with the MLX serve.

## Status
Early but proven — `fix` verifies real bugs end-to-end. Roadmap: richer `do` agent loop, git `--commit`,
multi-file fixes, then a native SwiftUI desktop app over the same engine.
