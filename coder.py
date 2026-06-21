#!/usr/bin/env python3
"""coder — the all-local, verifier-first coding CLI. End-to-end ours: M5 + MLX model, our retrieval, our
execution-verifiers, no cloud. The differentiator vs aider/Cline: it doesn't just ask the model — it
LOCALIZES (callsieve) → generates N candidates → VERIFIES each by running your tests → keeps only a fix
that actually passes → repairs on failure. The model is never trusted; the test is.

  coder fix calc.py --test "pytest -q"          # verified-fix loop (the core)
  coder do  "add input validation" --repo . --test "cargo test"   # the agentic loop (ReAct)
  coder explain src/query/tokens.rs             # plain explanation

Points at our local serve via the gateway (:8090 — sampling defaults + native tools). Set CODER_BASE to override.
"""
import argparse
import json
import os
import subprocess
import sys
import urllib.request

# Talk to OUR serve directly (:8080) — we control the request format. The gateway (:8090) is for
# third-party tools (aider/Cline). Do NOT send a `model` field: mlx_lm.server treats it as a load
# instruction and 404s/errors trying to fetch it. The serve serves whatever's loaded.
BASE = os.environ.get("CODER_BASE", "http://localhost:8080/v1")
HERE = os.path.dirname(os.path.abspath(__file__))


def C(s, c):  # tiny color helper
    return f"\033[{c}m{s}\033[0m" if sys.stdout.isatty() else s


def ask(prompt, temp=0.3, max_tokens=1400):
    body = json.dumps({"messages": [{"role": "user", "content": prompt}],
                       "temperature": temp, "max_tokens": max_tokens,
                       "chat_template_kwargs": {"enable_thinking": False}}).encode()
    req = urllib.request.Request(BASE + "/chat/completions", body, {"content-type": "application/json"})
    return json.loads(urllib.request.urlopen(req, timeout=600).read())["choices"][0]["message"]["content"]


def extract_code(text):
    import re
    m = re.search(r"```(?:[\w+]*)\n(.*?)```", text, re.S)
    return (m.group(1) if m else text).strip()


def run(cmd, cwd):
    r = subprocess.run(cmd, shell=True, cwd=cwd, capture_output=True, text=True, timeout=600)
    return r.returncode, (r.stdout + r.stderr)


def localize(repo, failure):
    """Use callsieve to pinpoint the relevant code, if available; else return ''."""
    cs = subprocess.run("command -v callsieve", shell=True, capture_output=True, text=True).stdout.strip()
    if not cs:
        return ""
    rc, out = run(f"callsieve {repo} --query {json.dumps(failure[:200])} --top 3 2>/dev/null", repo)
    return out[:1500] if rc == 0 else ""


def cmd_fix(a):
    path = os.path.abspath(a.file)
    repo = a.repo or os.path.dirname(path) or "."
    original = open(path).read()
    print(C("● running tests…", "36"))
    rc, out = run(a.test, repo)
    if rc == 0:
        print(C("✓ tests already pass — nothing to fix.", "32"))
        return 0
    failure = out[-1200:]
    print(C(f"✗ failing. localizing + generating {a.n} candidates…", "33"))
    ctx = localize(repo, failure)
    prompt = (f"This file fails its tests. Output ONLY the corrected full file, nothing else.\n\n"
              f"=== {os.path.basename(path)} ===\n{original}\n\n=== test failure ===\n{failure}\n"
              + (f"\n=== related code ===\n{ctx}\n" if ctx else ""))
    for i in range(a.n):
        cand = extract_code(ask(prompt, temp=0.2 + 0.2 * i))
        if not cand or cand.strip() == original.strip():
            print(f"  candidate {i+1}: no change"); continue
        open(path, "w").write(cand + ("\n" if not cand.endswith("\n") else ""))
        rc, _ = run(a.test, repo)
        if rc == 0:
            import difflib
            diff = "".join(difflib.unified_diff(original.splitlines(True), cand.splitlines(True),
                                                 f"a/{a.file}", f"b/{a.file}"))
            print(C(f"✓ candidate {i+1} PASSES — verified fix applied:", "32"))
            for ln in diff.splitlines():
                print(C(ln, "32" if ln.startswith("+") else "31" if ln.startswith("-") else "0"))
            return 0
        print(f"  candidate {i+1}: still failing")
        open(path, "w").write(original)            # revert before next try
    print(C(f"✗ no verified fix in {a.n} candidates (file unchanged). Try --n higher or `coder do`.", "31"))
    return 1


def cmd_do(a):
    print(C("● delegating to the agentic loop (57_tool_agent)…", "36"))
    agent = os.path.join(HERE, "57_tool_agent.py")
    if not os.path.exists(agent):
        print(C("agent script not found; use `coder fix` for single-file fixes.", "31")); return 1
    cmd = [sys.executable, agent, "--repo", a.repo or ".", "--task", a.task]
    if a.test:
        cmd += ["--test", a.test]
    if a.apply:
        cmd += ["--apply"]
    return subprocess.call(cmd)


def cmd_explain(a):
    src = open(a.file).read()
    print(ask(f"Explain this code clearly and concisely:\n```\n{src[:6000]}\n```", temp=0.4))
    return 0


def main():
    p = argparse.ArgumentParser(prog="coder", description="all-local verifier-first coding CLI")
    sub = p.add_subparsers(dest="cmd", required=True)
    f = sub.add_parser("fix", help="verified single-file fix")
    f.add_argument("file"); f.add_argument("--test", required=True, help="test command that must pass")
    f.add_argument("--repo", default=None); f.add_argument("--n", type=int, default=5)
    f.set_defaults(fn=cmd_fix)
    d = sub.add_parser("do", help="agentic multi-step task")
    d.add_argument("task"); d.add_argument("--repo", default="."); d.add_argument("--test", default=None)
    d.add_argument("--apply", action="store_true"); d.set_defaults(fn=cmd_do)
    e = sub.add_parser("explain", help="explain a file"); e.add_argument("file"); e.set_defaults(fn=cmd_explain)
    a = p.parse_args()
    try:
        sys.exit(a.fn(a))
    except (urllib.error.URLError, ConnectionError, OSError) as e:
        print(C(f"✗ can't reach the local model ({type(e).__name__}). Start it: "
                f"serve_supervisor.py --model models/GLM-5.2-q3a4-v2 --adapter-path heal/adapters-recover --port 8080", "31"))
        sys.exit(2)


if __name__ == "__main__":
    main()
