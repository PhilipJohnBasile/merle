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
    cmd: Cmd,
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
    },
    /// Explain a file in plain language.
    Explain { file: String },
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

fn cmd_fix(file: &str, test: &str, n: usize, repo: Option<String>) -> i32 {
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

fn main() {
    let code = match Cli::parse().cmd {
        Cmd::Fix { file, test, n, repo } => cmd_fix(&file, &test, n, repo),
        Cmd::Explain { file } => cmd_explain(&file),
    };
    std::process::exit(code);
}
