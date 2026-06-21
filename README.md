# coder

An **all-local, verifier-first** coding CLI. Your model, your machine, nothing sent to the cloud.

The difference vs aider/Cline: `coder` doesn't just ask the model — it **runs your tests to verify
every fix**. It generates candidates, keeps only one that *actually makes the tests pass*, and shows
you the diff. The model is never trusted; the test is.

```
coder fix calc.py --test "pytest -q"          # verified single-file fix (generate → verify → apply → diff)
coder do  "add input validation" --test "cargo test"   # multi-step agentic task
coder explain src/foo.rs                        # plain explanation
```

## How it works (one engine, many faces)
```
coder  →  localize (callsieve)  →  local model  →  best-of-N  →  run your tests  →  apply + diff
```
It talks to a local MLX model server (default `http://localhost:8080`, set `CODER_BASE` to change).
Pair it with the GLM-5.2-Demolition model + serve, or any OpenAI-compatible local endpoint.

## Install
```
git clone <this repo> && cd coder
ln -s "$PWD/coder.py" /usr/local/bin/coder   # or add to PATH
```
Requires Python 3.11+ and a running local model server. Cross-platform; Apple-silicon-native when paired
with the MLX serve.

## Status
Early but proven — `fix` verifies real bugs end-to-end. Roadmap: richer `do` agent loop, git `--commit`,
multi-file fixes, then a native SwiftUI desktop app over the same engine.
