//! merle — the all-local, verifier-first coding CLI. 🐕
//!
//! Named after Gayla, a blue merle Australian Shepherd: fast, brilliant, tireless — she herds your code.
//! The difference vs other agents: merle never trusts the model, it trusts the TEST. It generates
//! candidate fixes, keeps only one that makes your tests pass, and shows you the diff.
//!
//! One Rust binary. Talks to a local model server (default http://localhost:8080, set MERLE_BASE).
//! Part of a one-language stack: merle + callsieve (retrieval) + vecstore (memory), all Rust.

use clap::{Parser, Subcommand};
use std::fs;
use std::process::Command;
use std::time::Duration;

fn base() -> String {
    std::env::var("MERLE_BASE").unwrap_or_else(|_| "http://localhost:8080/v1".into()) + "/chat/completions"
}

#[derive(Parser)]
#[command(name = "merle", version, about = "all-local, verifier-first coding CLI 🐕 — verify, don't trust")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Verified single-file fix: generate candidates, keep one that makes the tests pass, show the diff.
    Fix {
        file: String,
        /// Test command that must pass (e.g. "pytest -q" or "cargo test")
        #[arg(long)]
        test: String,
        /// Number of candidates to try
        #[arg(long, default_value_t = 5)]
        n: usize,
        /// Repo / working dir (defaults to the file's directory)
        #[arg(long)]
        repo: Option<String>,
        /// Git-commit the verified fix once it passes
        #[arg(long)]
        commit: bool,
    },
    /// Explain a file in plain language.
    Explain { file: String },
    /// Agentic task — the model uses tools (read/list/grep/write/run) to do it, then verifies.
    Do {
        /// What to do, in plain language
        task: String,
        /// Repo / working dir
        #[arg(long, default_value = ".")]
        repo: String,
        /// Optional test command to verify the result at the end
        #[arg(long)]
        test: Option<String>,
        /// Max agent steps before giving up
        #[arg(long, default_value_t = 16)]
        max_steps: usize,
    },
    /// Show the code that's relevant to a task — embedded callsieve retrieval.
    Context {
        /// What you're trying to do
        task: String,
        /// Repo / working dir
        #[arg(long, default_value = ".")]
        repo: String,
    },
}

/// One blocking chat call to the local model server. No `model` field — the serve serves what's loaded.
fn ask(prompt: &str, temp: f64, max_tokens: u32) -> String {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(600)).build();
    let body = serde_json::json!({
        "messages": [{"role": "user", "content": prompt}],
        "temperature": temp,
        "max_tokens": max_tokens,
        "chat_template_kwargs": {"enable_thinking": false}
    });
    match agent.post(&base()).send_json(body) {
        Ok(resp) => resp
            .into_json::<serde_json::Value>()
            .ok()
            .and_then(|v| v["choices"][0]["message"]["content"].as_str().map(str::to_string))
            .unwrap_or_default(),
        Err(_) => String::new(),
    }
}

/// Pull the first fenced code block out of the model's reply (```lang ... ```), else the whole text.
fn extract_code(text: &str) -> String {
    if let Some(s) = text.find("```") {
        let after = &text[s + 3..];
        if let Some(nl) = after.find('\n') {
            let rest = &after[nl + 1..];
            if let Some(e) = rest.find("```") {
                return rest[..e].trim().to_string();
            }
        }
    }
    text.trim().to_string()
}

/// Run a shell command in `cwd`; return (exit code, stdout+stderr).
fn run(cmd: &str, cwd: &str) -> (i32, String) {
    match Command::new("sh").arg("-c").arg(cmd).current_dir(cwd).output() {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            (o.status.code().unwrap_or(-1), s)
        }
        Err(e) => (-1, e.to_string()),
    }
}

fn tail(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    chars[chars.len().saturating_sub(n)..].iter().collect()
}

fn show_diff(name: &str, before: &str, after: &str) {
    use similar::{ChangeTag, TextDiff};
    println!("--- a/{name}\n+++ b/{name}");
    for change in TextDiff::from_lines(before, after).iter_all_changes() {
        let (sign, color) = match change.tag() {
            ChangeTag::Delete => ("-", "\x1b[31m"),
            ChangeTag::Insert => ("+", "\x1b[32m"),
            ChangeTag::Equal => (" ", "\x1b[0m"),
        };
        print!("{color}{sign}{change}\x1b[0m");
    }
}

fn cmd_fix(file: &str, test: &str, n: usize, repo: Option<String>, commit: bool) -> i32 {
    let path = std::path::Path::new(file);
    let repo = repo.unwrap_or_else(|| match path.parent().and_then(|p| p.to_str()) {
        Some(p) if !p.is_empty() => p.to_string(),
        _ => ".".to_string(),
    });
    let name = path.file_name().and_then(|s| s.to_str()).unwrap_or(file);
    let original = match fs::read_to_string(file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("✗ can't read {file}: {e}");
            return 2;
        }
    };
    println!("\x1b[36m● running tests…\x1b[0m");
    if run(test, &repo).0 == 0 {
        println!("\x1b[32m✓ tests already pass — nothing to fix.\x1b[0m");
        return 0;
    }
    let failure = tail(&run(test, &repo).1, 1200);
    println!("\x1b[33m✗ failing. generating {n} candidates…\x1b[0m");
    let prompt = format!(
        "This file fails its tests. Output ONLY the corrected full file, nothing else.\n\n\
         === {name} ===\n{original}\n\n=== test failure ===\n{failure}\n"
    );
    for i in 0..n {
        let cand = extract_code(&ask(&prompt, 0.2 + 0.2 * i as f64, 1400));
        if cand.is_empty() || cand.trim() == original.trim() {
            println!("  candidate {}: no change", i + 1);
            continue;
        }
        let written = format!("{cand}\n");
        let _ = fs::write(file, &written);
        if run(test, &repo).0 == 0 {
            println!("\x1b[32m✓ candidate {} PASSES — verified fix applied:\x1b[0m", i + 1);
            show_diff(name, &original, &written);
            if commit {
                let (rc, _) = run(&format!("git add {file} && git commit -q -m 'merle: verified fix'"), &repo);
                println!(
                    "{}",
                    if rc == 0 {
                        "\x1b[32m  ✓ committed\x1b[0m"
                    } else {
                        "\x1b[33m  (commit skipped — not a git repo, or nothing to commit)\x1b[0m"
                    }
                );
            }
            return 0;
        }
        println!("  candidate {}: still failing", i + 1);
        let _ = fs::write(file, &original); // revert before next try
    }
    println!("\x1b[31m✗ no verified fix in {n} candidates (file unchanged). Try --n higher.\x1b[0m");
    1
}

fn cmd_explain(file: &str) -> i32 {
    match fs::read_to_string(file) {
        Ok(src) => {
            let src: String = src.chars().take(6000).collect();
            println!("{}", ask(&format!("Explain this code clearly and concisely:\n```\n{src}\n```"), 0.4, 1200));
            0
        }
        Err(e) => {
            eprintln!("✗ {e}");
            2
        }
    }
}

// ============ agentic loop: the model drives tools to read/edit/run, verifier-gated ============

fn tool_schemas() -> serde_json::Value {
    serde_json::json!([
        {"type":"function","function":{"name":"read_file","description":"Read a file's contents.","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}},
        {"type":"function","function":{"name":"list_dir","description":"List entries in a directory.","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}},
        {"type":"function","function":{"name":"grep","description":"Search the repo for a string/regex.","parameters":{"type":"object","properties":{"pattern":{"type":"string"}},"required":["pattern"]}}},
        {"type":"function","function":{"name":"write_file","description":"Create or overwrite a file.","parameters":{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}}},
        {"type":"function","function":{"name":"run","description":"Run a shell command (build, tests, git…).","parameters":{"type":"object","properties":{"cmd":{"type":"string"}},"required":["cmd"]}}},
        {"type":"function","function":{"name":"done","description":"The request is complete.","parameters":{"type":"object","properties":{"summary":{"type":"string"}},"required":["summary"]}}}
    ])
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}\n…(truncated)", s.chars().take(n).collect::<String>())
    } else {
        s.to_string()
    }
}

fn execute_tool(name: &str, args: &serde_json::Value, repo: &str) -> String {
    let s = |k: &str| args[k].as_str().unwrap_or("").to_string();
    let full = |rel: &str| std::path::Path::new(repo).join(rel);
    match name {
        "read_file" => fs::read_to_string(full(&s("path")))
            .map(|c| trunc(&c, 4000))
            .unwrap_or_else(|e| format!("error: {e}")),
        "list_dir" => match fs::read_dir(full(&s("path"))) {
            Ok(rd) => {
                let mut v: Vec<String> = rd.filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned()).collect();
                v.sort();
                v.join("\n")
            }
            Err(e) => format!("error: {e}"),
        },
        "grep" => trunc(&run(&format!("grep -rn {:?} . 2>/dev/null | head -40", s("pattern")), repo).1, 2000),
        "write_file" => fs::write(full(&s("path")), s("content"))
            .map(|_| format!("wrote {}", s("path")))
            .unwrap_or_else(|e| format!("error: {e}")),
        "run" => {
            let (rc, o) = run(&s("cmd"), repo);
            trunc(&format!("exit={rc}\n{o}"), 3000)
        }
        "done" => format!("done: {}", s("summary")),
        _ => format!("unknown tool: {name}"),
    }
}

fn chat_with_tools(messages: &[serde_json::Value], tools: &serde_json::Value) -> serde_json::Value {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(600)).build();
    let body = serde_json::json!({
        "messages": messages, "tools": tools, "temperature": 0.3, "max_tokens": 1024,
        "chat_template_kwargs": {"enable_thinking": false}
    });
    agent.post(&base()).send_json(body).ok()
        .and_then(|r| r.into_json::<serde_json::Value>().ok())
        .unwrap_or_else(|| serde_json::json!({"choices":[{"message":{"content":"(no response — is the model server running on :8080?)"}}]}))
}

/// Run the agent until it gives a final text answer / calls `done` / hits max_steps. Returns its summary.
fn run_agent_turn(
    messages: &mut Vec<serde_json::Value>,
    tools: &serde_json::Value,
    repo: &str,
    max_steps: usize,
    test: Option<&str>,
) -> String {
    let mut edited = false;
    for _ in 0..max_steps {
        let msg = chat_with_tools(messages, tools)["choices"][0]["message"].clone();
        messages.push(msg.clone());
        match msg["tool_calls"].as_array() {
            Some(calls) if !calls.is_empty() => {
                let mut finished = None;
                for call in calls {
                    let name = call["function"]["name"].as_str().unwrap_or("");
                    let args: serde_json::Value =
                        serde_json::from_str(call["function"]["arguments"].as_str().unwrap_or("{}"))
                            .unwrap_or_else(|_| serde_json::json!({}));
                    println!("\x1b[36m  ● {name} {}\x1b[0m", trunc(&args.to_string(), 100).replace('\n', " "));
                    let result = execute_tool(name, &args, repo);
                    messages.push(serde_json::json!({"role":"tool","tool_call_id":call["id"].clone(),"content":result}));
                    if name == "write_file" {
                        edited = true;
                    }
                    if name == "done" {
                        finished = Some(args["summary"].as_str().unwrap_or("done").to_string());
                    }
                }
                if let Some(f) = finished {
                    return f;
                }
                // Verifier-gated early termination: if the model has edited something and the tests now
                // pass, we're verifiably done — don't wait for the model to remember to call `done`.
                if edited {
                    if let Some(t) = test {
                        if run(t, repo).0 == 0 {
                            return "verified — tests pass".to_string();
                        }
                    }
                }
            }
            _ => return msg["content"].as_str().unwrap_or("").to_string(),
        }
    }
    "(reached max steps)".to_string()
}

fn agent_system(repo: &str) -> serde_json::Value {
    serde_json::json!({"role":"system","content": format!(
        "You are merle 🐕, an autonomous, verifier-first coding agent working in the repository at '{repo}'. \
         Use the tools (read_file, list_dir, grep, write_file, run) to inspect and edit the code. Make real \
         changes; prefer running tests/builds to verify. When the request is satisfied, summarize and call \
         `done`. Be concise and concrete.")})
}

fn cmd_do(task: &str, repo: &str, test: Option<String>, max_steps: usize) -> i32 {
    let tools = tool_schemas();
    let mut messages = vec![agent_system(repo), serde_json::json!({"role":"user","content":task})];
    let summary = run_agent_turn(&mut messages, &tools, repo, max_steps, test.as_deref());
    println!("\x1b[32m✓ {summary}\x1b[0m");
    if let Some(t) = test {
        let ok = run(&t, repo).0 == 0;
        println!("{}", if ok { "\x1b[32m✓ tests pass — verified\x1b[0m" } else { "\x1b[31m✗ tests fail\x1b[0m" });
        return i32::from(!ok);
    }
    0
}

/// `merle` with no subcommand: an interactive session — talk to the local model, it acts in this dir.
fn repl(repo: &str) -> i32 {
    use std::io::Write;
    println!("\x1b[35mmerle 🐕\x1b[0m — local coding agent in {repo}  (model: GLM-5.2-Demolition). /exit to quit.");
    let tools = tool_schemas();
    let mut messages = vec![agent_system(repo)];
    loop {
        print!("\x1b[35mmerle>\x1b[0m ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).unwrap_or(0) == 0 {
            println!();
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if line == "/exit" || line == "/quit" {
            break;
        }
        messages.push(serde_json::json!({"role":"user","content":line}));
        let answer = run_agent_turn(&mut messages, &tools, repo, 16, None);
        if !answer.is_empty() {
            println!("\x1b[37m{answer}\x1b[0m");
        }
    }
    println!("bye 🐕");
    0
}

// ---- embedded callsieve: relevant-code retrieval, compiled into the merle binary --------------

fn callsieve_context(repo: &str, task: &str) -> Result<Vec<String>, String> {
    let root = std::path::Path::new(repo);
    let index = callsieve::indexer::build_index(root).map_err(|e| e.to_string())?;
    let ctx = callsieve::query::build_context(root, &index, task, 6, 2, true).map_err(|e| e.to_string())?;
    Ok(callsieve::query::context_read_first_files(&ctx))
}

fn cmd_context(task: &str, repo: &str) -> i32 {
    match callsieve_context(repo, task) {
        Ok(files) if !files.is_empty() => {
            println!("\x1b[36m● callsieve: {} relevant file(s) for \"{task}\"\x1b[0m", files.len());
            for f in &files {
                println!("  {f}");
            }
            0
        }
        Ok(_) => {
            println!("(callsieve found nothing relevant — try a more specific task)");
            0
        }
        Err(e) => {
            eprintln!("✗ callsieve: {e}");
            1
        }
    }
}

fn main() {
    let cwd = std::env::current_dir().ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_else(|| ".".into());
    let code = match Cli::parse().cmd {
        None => repl(&cwd),
        Some(Cmd::Fix { file, test, n, repo, commit }) => cmd_fix(&file, &test, n, repo, commit),
        Some(Cmd::Explain { file }) => cmd_explain(&file),
        Some(Cmd::Do { task, repo, test, max_steps }) => cmd_do(&task, &repo, test, max_steps),
        Some(Cmd::Context { task, repo }) => cmd_context(&task, &repo),
    };
    std::process::exit(code);
}
