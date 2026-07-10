//! Fix-history memory: after `merle fix` verifies a working candidate, remember (bug -> diff) so a
//! later fix on a similar bug in the same repo starts from precedent instead of cold. Opt-in only
//! (see `--memory` in main.rs) — first use downloads callsieve's fastembed ONNX model, a real
//! network/disk cost callers shouldn't pay by default.

use callsieve::query::embed::{FastembedEmbedder, LocalEmbedder};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use vecstore::{Collection, Metadata, Query, VecDatabase};

const COLLECTION: &str = "fixes";
const MIN_SIMILARITY: f32 = 0.5;

// Not sha256 (no crypto-hash dependency in this binary): a 64-bit hash of the canonicalized repo
// path is plenty collision-resistant for a local cache directory name. The only failure mode of a
// non-cryptographic hash here — a rustc upgrade changing the algorithm — just starts a fresh, empty
// cache for that repo, not a correctness bug.
fn memory_path(repo: &str) -> PathBuf {
    let canon = std::fs::canonicalize(repo)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| repo.to_string());
    let mut hasher = DefaultHasher::new();
    canon.hash(&mut hasher);
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".merle").join("memory").join(format!("{:016x}", hasher.finish()))
}

/// Opens (creating on disk only if `create`) this repo's memory collection. `Ok(None)` means "no
/// memory yet for this repo" — callers reading history should treat that as zero hits, not an error.
fn open_collection(repo: &str, create: bool) -> Result<Option<Collection>, String> {
    let path = memory_path(repo);
    if !create && !path.join(COLLECTION).exists() {
        return Ok(None);
    }
    std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    let mut db = VecDatabase::open(&path).map_err(|e| e.to_string())?;
    match db.get_collection(COLLECTION).map_err(|e| e.to_string())? {
        Some(c) => Ok(Some(c)),
        None => Ok(Some(db.create_collection(COLLECTION).map_err(|e| e.to_string())?)),
    }
}

/// Store a verified (bug -> diff) pair for this repo. Call only after a candidate's tests pass.
pub fn record_fix(repo: &str, bug_desc: &str, diff: &str) -> Result<(), String> {
    let embedder = FastembedEmbedder::new_code().map_err(|e| e.to_string())?;
    let vector = embedder
        .embed(&[bug_desc])
        .map_err(|e| e.to_string())?
        .into_iter()
        .next()
        .ok_or_else(|| "embedder returned no vector".to_string())?;
    let mut col = open_collection(repo, true)?.expect("open_collection(create=true) always returns Some");
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos().to_string())
        .unwrap_or_else(|_| "0".to_string());
    let mut fields = HashMap::new();
    fields.insert("bug_desc".to_string(), serde_json::json!(bug_desc));
    fields.insert("diff".to_string(), serde_json::json!(diff));
    col.upsert(id, vector, Metadata { fields }).map_err(|e| e.to_string())
}

/// Up to `k` past verified fixes in this repo whose bug description is similar to `bug_desc`, each
/// as (diff, similarity in 0..1, higher = closer). Returns empty — not the nearest-available junk —
/// when the repo has no memory yet or nothing clears `MIN_SIMILARITY`.
pub fn similar_fixes(repo: &str, bug_desc: &str, k: usize) -> Result<Vec<(String, f32)>, String> {
    let col = match open_collection(repo, false)? {
        Some(c) => c,
        None => return Ok(Vec::new()),
    };
    let embedder = FastembedEmbedder::new_code().map_err(|e| e.to_string())?;
    let vector = embedder
        .embed(&[bug_desc])
        .map_err(|e| e.to_string())?
        .into_iter()
        .next()
        .ok_or_else(|| "embedder returned no vector".to_string())?;
    let hits = col.query(Query::new(vector).with_limit(k)).map_err(|e| e.to_string())?;
    Ok(hits
        .into_iter()
        // vecstore's Neighbor.score is a raw cosine DISTANCE for the default Cosine metric — lower
        // means more similar, not a similarity score (confirmed by reading hnsw_backend.rs's
        // search(), which passes the HNSW distance through unconverted for Distance::Cosine).
        // Flip to a conventional 0..1 similarity (1 - distance) so callers, prompts, and the
        // MIN_SIMILARITY floor all read "higher is closer," like everywhere else in this codebase.
        .filter_map(|n| {
            let sim = 1.0 - n.score;
            n.metadata.fields.get("diff").and_then(|v| v.as_str()).map(|d| (d.to_string(), sim))
        })
        .filter(|(_, sim)| *sim >= MIN_SIMILARITY)
        .collect())
}
