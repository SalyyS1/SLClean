//! Content-addressed duplicate AI model weight and build-artifact blob analyzer.
//!
//! Standard user-file duplicate scanners skip `.cache`, `AppData`, `.ollama`, and `.lmstudio`
//! because deleting arbitrary package-cache files breaks installations. However, multi-gigabyte
//! LLM/diffusion weights (`.gguf`, `.safetensors`, `.onnx`, `.pt`, `.ckpt`, and content-addressed
//! `blobs/` entries) are frequently downloaded twice across HuggingFace Hub, Ollama, LM Studio,
//! Jan, GPT4All, ComfyUI, and `Downloads`.
//!
//! This module finds byte-identical model and artifact blobs across ecosystems while:
//! 1. Skipping symlinks (such as HuggingFace `snapshots/<rev>/* -> ../../blobs/<sha256>`) so
//!    pointer trees are never miscounted as duplicate disk consumption.
//! 2. Deduplicating hardlinks sharing the same underlying physical file identity `(dev, ino)`
//!    or Windows file index so hardlinked caches (`uv`, `ln`, `mklink /H`) report `0` wasted bytes
//!    and instead credit `hardlinked_saved_bytes`.
//! 3. Using a three-phase `(exact size -> 4 KiB head + 4 KiB tail -> streaming SHA-256)` pipeline
//!    plus optional content-addressed filename digest extraction (`--trust-cas-names`) so unique
//!    multi-GB files require zero disk reads.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Default minimum blob size for duplicate model/artifact scanning: 4 MiB.
pub const DEFAULT_MIN_BLOB_BYTES: u64 = 4 * 1024 * 1024;

const EDGE_SAMPLE_BYTES: usize = 4096;
const STREAM_CHUNK_BYTES: usize = 256 * 1024;

/// File extensions treated as AI model weights, checkpoints, or binary build archives.
const TARGET_EXTENSIONS: &[&str] = &[
    "gguf",
    "safetensors",
    "onnx",
    "pt",
    "pth",
    "ckpt",
    "bin",
    "ggml",
    "tflite",
    "ov",
    "engine",
    "plan",
    "whl",
    "jar",
    "nupkg",
    "tgz",
    "vsix",
    "conda",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BlobLocation {
    pub path: PathBuf,
    pub ecosystem: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_hint: Option<String>,
    pub modified: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct BlobDupGroup {
    pub sha256: String,
    pub bytes: u64,
    pub wasted_bytes: u64,
    pub cross_ecosystem: bool,
    pub ecosystems: Vec<String>,
    pub locations: Vec<BlobLocation>,
    pub remediation: String,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct BlobDupReport {
    pub roots_scanned: Vec<PathBuf>,
    pub min_blob_bytes: u64,
    pub trust_cas_names: bool,
    pub scanned_files: u64,
    pub candidate_blobs: u64,
    pub edge_sampled_files: u64,
    pub full_hashed_files: u64,
    pub cas_fastpath_files: u64,
    pub symlinks_skipped: u64,
    pub hardlinks_skipped: u64,
    pub hardlinked_saved_bytes: u64,
    pub total_wasted_bytes: u64,
    pub cross_ecosystem_wasted_bytes: u64,
    pub groups: Vec<BlobDupGroup>,
}

fn dup_html_esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render a self-contained dark-mode HTML Duplicate AI Model & Build Blob Report (`--scan-dupes --report-html`).
pub fn render_dupes_html(rep: &BlobDupReport) -> String {
    let mut rows = String::new();
    for g in &rep.groups {
        let mb = g.wasted_bytes as f64 / (1024.0 * 1024.0);
        let paths = g
            .locations
            .iter()
            .map(|l| format!("<code>{}</code>", dup_html_esc(&l.path.to_string_lossy())))
            .collect::<Vec<_>>()
            .join("<br>");
        rows.push_str(&format!(
            "<tr><td><code>{}</code></td><td>{}</td><td>{:.2} MiB</td><td>{}</td><td>{}</td></tr>\n",
            dup_html_esc(&g.sha256.chars().take(12).collect::<String>()),
            g.locations.len(),
            mb,
            dup_html_esc(&g.ecosystems.join(", ")),
            paths
        ));
    }
    if rows.is_empty() {
        rows.push_str("<tr><td colspan=\"5\" style=\"color:#4ade80\"><b>PASS &mdash; Zero byte-identical duplicate blobs found.</b></td></tr>");
    }
    let total_mb = rep.total_wasted_bytes as f64 / (1024.0 * 1024.0);
    format!(
        r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>SLClean Duplicate Blob Report</title>
<style>
body{{margin:0;padding:28px;background:#0f1411;color:#e6efe9;font-family:system-ui,sans-serif}}
.wrap{{max-width:1060px;margin:0 auto}}.card{{background:#161d19;border:1px solid #2a3730;border-radius:10px;padding:18px;margin-bottom:16px}}
table{{width:100%;border-collapse:collapse;font-size:12.5px}}th,td{{padding:8px 10px;border-bottom:1px solid #24302a;text-align:left;vertical-align:top}}
th{{color:#93a89b}}code{{font-family:ui-monospace,monospace;color:#4ade80}}
</style></head><body><div class="wrap">
<div class="card"><h1>SLClean Content-Addressed Duplicate Blob Report</h1>
<p>Groups: <b>{}</b> &middot; Wasted Space: <b>{:.2} MiB</b> &middot; Scanned Files: <b>{}</b> &middot; Hardlinks Preserved: <b>{}</b></p></div>
<div class="card"><table><thead><tr><th>SHA-256</th><th>Copies</th><th>Wasted</th><th>Ecosystems</th><th>Locations</th></tr></thead>
<tbody>{}</tbody></table></div></div></body></html>"##,
        rep.groups.len(),
        total_mb,
        rep.scanned_files,
        rep.hardlinks_skipped,
        rows
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct PhysicalFileId {
    dev: u64,
    ino: u64,
}

#[cfg(unix)]
fn physical_file_id(_path: &Path, meta: &Metadata) -> Option<PhysicalFileId> {
    use std::os::unix::fs::MetadataExt;
    Some(PhysicalFileId { dev: meta.dev(), ino: meta.ino() })
}

#[cfg(windows)]
fn physical_file_id(path: &Path, _meta: &Metadata) -> Option<PhysicalFileId> {
    use std::os::windows::ffi::OsStrExt;
    #[repr(C)]
    #[allow(non_snake_case)]
    struct ByHandleFileInformation {
        dwFileAttributes: u32,
        ftCreationTime: [u32; 2],
        ftLastAccessTime: [u32; 2],
        ftLastWriteTime: [u32; 2],
        dwVolumeSerialNumber: u32,
        nFileSizeHigh: u32,
        nFileSizeLow: u32,
        nNumberOfLinks: u32,
        nFileIndexHigh: u32,
        nFileIndexLow: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateFileW(
            lpFileName: *const u16,
            dwDesiredAccess: u32,
            dwShareMode: u32,
            lpSecurityAttributes: *const core::ffi::c_void,
            dwCreationDisposition: u32,
            dwFlagsAndAttributes: u32,
            hTemplateFile: isize,
        ) -> isize;
        fn GetFileInformationByHandle(hFile: isize, lpFileInformation: *mut ByHandleFileInformation) -> i32;
        fn CloseHandle(hObject: isize) -> i32;
    }
    const INVALID_HANDLE_VALUE: isize = -1;
    const FILE_READ_ATTRIBUTES: u32 = 0x0080;
    const FILE_SHARE_ALL: u32 = 0x0000_0007;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    unsafe {
        let handle = CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_ALL,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            0,
        );
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut info: ByHandleFileInformation = std::mem::zeroed();
        let ok = GetFileInformationByHandle(handle, &mut info);
        let _ = CloseHandle(handle);
        if ok == 0 {
            return None;
        }
        let ino = ((info.nFileIndexHigh as u64) << 32) | (info.nFileIndexLow as u64);
        if ino == 0 {
            return None;
        }
        Some(PhysicalFileId { dev: info.dwVolumeSerialNumber as u64, ino })
    }
}

#[cfg(not(any(unix, windows)))]
fn physical_file_id(_path: &Path, _meta: &Metadata) -> Option<PhysicalFileId> {
    None
}

/// Discover default AI model cache and artifact directories that exist on this machine.
pub fn default_ai_and_artifact_roots() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Ok(hf_home) = std::env::var("HF_HOME") {
        candidates.push(PathBuf::from(hf_home));
    }
    if let Ok(ollama_models) = std::env::var("OLLAMA_MODELS") {
        candidates.push(PathBuf::from(ollama_models));
    }

    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".cache/huggingface/hub"));
        candidates.push(home.join(".ollama/models"));
        candidates.push(home.join(".lmstudio/models"));
        candidates.push(home.join(".cache/lm-studio/models"));
        candidates.push(home.join(".cache/torch/hub/checkpoints"));
        candidates.push(home.join("jan/models"));
        candidates.push(home.join("Downloads"));
    }

    if let Some(local) = dirs::cache_dir() {
        candidates.push(local.join("huggingface/hub"));
        candidates.push(local.join("nomic.ai/GPT4All"));
        candidates.push(local.join("pip/http-v2"));
        candidates.push(local.join("uv/archive-v0"));
    }

    let mut out: Vec<PathBuf> = Vec::new();
    for c in candidates {
        if !c.is_dir() {
            continue;
        }
        let norm = norm_lower(&c);
        if out.iter().any(|existing| {
            let ex = norm_lower(existing);
            norm == ex || norm.starts_with(&format!("{ex}/"))
        }) {
            continue;
        }
        out.push(c);
    }
    out
}

fn norm_lower(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_ascii_lowercase()
}

fn modified_epoch(m: &Metadata) -> u64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Extract a canonical lowercase 64-character SHA-256 hex digest from content-addressed blob paths:
/// - Ollama: `.../blobs/sha256-<64hex>` or `.../blobs/sha256:<64hex>`
/// - HuggingFace / OCI / uv: `.../blobs/<64hex>`
pub fn extract_cas_sha256(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_str()?;
    if let Some(rest) = file_name.strip_prefix("sha256-").or_else(|| file_name.strip_prefix("sha256:")) {
        if is_hex64(rest) {
            return Some(rest.to_ascii_lowercase());
        }
    }
    if is_hex64(file_name) {
        let parent_name = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if parent_name.eq_ignore_ascii_case("blobs") {
            return Some(file_name.to_ascii_lowercase());
        }
    }
    None
}

pub fn is_target_blob(path: &Path) -> bool {
    if extract_cas_sha256(path).is_some() {
        return true;
    }
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if name.ends_with(".tar.gz") || name.ends_with(".tar.zst") {
        return true;
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    TARGET_EXTENSIONS.contains(&ext.as_str())
}

pub fn classify_ecosystem(path: &Path) -> (&'static str, Option<String>) {
    let norm = path.to_string_lossy().replace('\\', "/");
    let lower = norm.to_ascii_lowercase();

    if lower.contains("/huggingface/") || lower.contains("/models--") {
        let hint = norm.split('/').find_map(|seg| {
            seg.strip_prefix("models--")
                .or_else(|| seg.strip_prefix("datasets--"))
                .map(|rest| rest.replace("--", "/"))
        });
        return ("huggingface", hint);
    }
    if lower.contains("/.ollama/") || lower.contains("/ollama/models") {
        return ("ollama", None);
    }
    if lower.contains("/.lmstudio/") || lower.contains("/lm-studio/") {
        let parts: Vec<&str> = norm.split('/').collect();
        let hint = parts.iter().position(|s| s.eq_ignore_ascii_case("models")).and_then(|idx| {
            if idx + 2 < parts.len() {
                Some(format!("{}/{}", parts[idx + 1], parts[idx + 2]))
            } else {
                None
            }
        });
        return ("lmstudio", hint);
    }
    if lower.contains("/gpt4all/") {
        return ("gpt4all", path.file_name().map(|n| n.to_string_lossy().into_owned()));
    }
    if lower.contains("/jan/models") {
        return ("jan", None);
    }
    if lower.contains("/comfyui/") || lower.contains("/stable-diffusion-webui/") {
        return ("comfyui", path.file_name().map(|n| n.to_string_lossy().into_owned()));
    }
    if lower.contains("/torch/hub/") || lower.contains("/pytorch/") {
        return ("pytorch", path.file_name().map(|n| n.to_string_lossy().into_owned()));
    }
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if matches!(ext.as_str(), "whl" | "jar" | "nupkg" | "tgz" | "vsix" | "conda")
        || lower.ends_with(".tar.gz")
        || lower.ends_with(".tar.zst")
    {
        return ("artifact", path.file_name().map(|n| n.to_string_lossy().into_owned()));
    }
    if lower.contains("/downloads/") {
        return ("downloads", path.file_name().map(|n| n.to_string_lossy().into_owned()));
    }
    ("user-file", path.file_name().map(|n| n.to_string_lossy().into_owned()))
}

/// Phase-2 fast edge fingerprint: hashes file length + first 4 KiB + last 4 KiB.
/// Sampling both head and tail separates quantized GGUF / SafeTensors checkpoints that share
/// identical header metadata in the first 4 KiB without streaming multi-GB bodies.
fn edge_fingerprint(path: &Path, size: u64) -> Option<[u8; 32]> {
    let mut f = File::open(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(size.to_le_bytes());

    let head_len = (size as usize).min(EDGE_SAMPLE_BYTES);
    let mut buf = vec![0u8; head_len];
    let mut read_total = 0usize;
    while read_total < head_len {
        match f.read(&mut buf[read_total..]) {
            Ok(0) => break,
            Ok(n) => read_total += n,
            Err(_) => return None,
        }
    }
    hasher.update(&buf[..read_total]);

    if size > EDGE_SAMPLE_BYTES as u64 {
        let tail_offset = size.saturating_sub(EDGE_SAMPLE_BYTES as u64);
        f.seek(SeekFrom::Start(tail_offset)).ok()?;
        let tail_len = (size - tail_offset) as usize;
        let mut tail_buf = vec![0u8; tail_len];
        let mut tail_read = 0usize;
        while tail_read < tail_len {
            match f.read(&mut tail_buf[tail_read..]) {
                Ok(0) => break,
                Ok(n) => tail_read += n,
                Err(_) => return None,
            }
        }
        hasher.update(&tail_buf[..tail_read]);
    }

    Some(hasher.finalize().into())
}

fn stream_sha256_hex(path: &Path, cancel: &AtomicBool) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; STREAM_CHUNK_BYTES];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(_) => return None,
        }
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for b in digest {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{b:02x}");
    }
    Some(hex)
}

fn skip_walk_dir(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    // Skip HuggingFace `snapshots` & `refs` pointer trees (`snapshots` contains symlinks or
    // redundant pointers to `blobs/`), VCS metadata, and OS system folders.
    matches!(
        n.as_str(),
        ".git" | "snapshots" | "refs" | "$recycle.bin" | "system volume information" | "winsxs"
    )
}

fn build_remediation(ecosystems: &[String], cross_ecosystem: bool) -> String {
    if cross_ecosystem {
        let joined = ecosystems.join(" + ");
        format!(
            "Identical weight/artifact blob stored independently across {joined}. Remove stale copies via tool CLI (e.g. `ollama rm` or `hf cache delete`) or replace secondary copies with hardlinks/symlinks to a single shared model root."
        )
    } else if ecosystems.first().map(|s| s.as_str()) == Some("huggingface") {
        "Duplicate blob across HuggingFace cache revisions or repos; prune unreferenced revisions with `huggingface-cli delete-cache`.".into()
    } else if ecosystems.first().map(|s| s.as_str()) == Some("artifact") {
        "Identical build archive duplicated across directories; keep the canonical release artifact and clean build staging copies.".into()
    } else {
        "Byte-identical model/artifact copies in the same ecosystem; keep the primary copy and remove or hardlink redundant duplicates.".into()
    }
}

#[derive(Clone)]
struct CandidateFile {
    path: PathBuf,
    bytes: u64,
    modified: u64,
}

/// Scan `roots` for duplicate AI model weights and binary build-artifact blobs.
pub fn scan_blobs(
    roots: &[PathBuf],
    min_blob_bytes: u64,
    trust_cas_names: bool,
    cancel: &AtomicBool,
) -> BlobDupReport {
    let effective_min = min_blob_bytes.max(1);
    let mut report = BlobDupReport {
        roots_scanned: roots.to_vec(),
        min_blob_bytes: effective_min,
        trust_cas_names,
        ..BlobDupReport::default()
    };

    let mut seen_physical: HashSet<PhysicalFileId> = HashSet::new();
    let mut by_size: HashMap<u64, Vec<CandidateFile>> = HashMap::new();

    let mut stack: Vec<PathBuf> = roots.iter().filter(|r| r.is_dir()).cloned().collect();
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return report;
        }
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        let mut sorted_entries: Vec<_> = entries.flatten().collect();
        sorted_entries.sort_by_key(|e| e.file_name());

        for entry in sorted_entries {
            if cancel.load(Ordering::Relaxed) {
                return report;
            }
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_symlink() {
                report.symlinks_skipped += 1;
                continue;
            }
            let path = entry.path();
            if ft.is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !skip_walk_dir(&name) {
                    stack.push(path);
                }
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            report.scanned_files += 1;
            if !is_target_blob(&path) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let bytes = meta.len();
            if bytes < effective_min {
                continue;
            }
            if let Some(fid) = physical_file_id(&path, &meta) {
                if !seen_physical.insert(fid) {
                    report.hardlinks_skipped += 1;
                    report.hardlinked_saved_bytes += bytes;
                    continue;
                }
            }
            report.candidate_blobs += 1;
            by_size.entry(bytes).or_default().push(CandidateFile {
                path,
                bytes,
                modified: modified_epoch(&meta),
            });
        }
    }

    // Phase 1: only exact-size buckets with >= 2 physical files can contain duplicates.
    for (bytes, candidates) in by_size {
        if candidates.len() < 2 {
            continue;
        }
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        // Partition files that can use verified CAS filename SHA-256 vs files needing disk reads.
        let mut digest_buckets: HashMap<String, Vec<CandidateFile>> = HashMap::new();
        let mut needs_io: Vec<CandidateFile> = Vec::new();

        if trust_cas_names {
            for c in candidates {
                if let Some(cas_hex) = extract_cas_sha256(&c.path) {
                    report.cas_fastpath_files += 1;
                    digest_buckets.entry(cas_hex).or_default().push(c);
                } else {
                    needs_io.push(c);
                }
            }
        } else {
            needs_io = candidates;
        }

        if !needs_io.is_empty() {
            // If we already have some CAS digests for this exact byte size, any non-CAS file of the
            // exact same size might match a CAS blob, so we must hash `needs_io` if either:
            // - an edge-fingerprint bucket has >= 2 files, OR
            // - `digest_buckets` is non-empty (cross-matching e.g. Downloads/model.gguf against Ollama sha256-<hex>).
            let has_cas_peers = !digest_buckets.is_empty();
            let mut edge_buckets: HashMap<[u8; 32], Vec<CandidateFile>> = HashMap::new();
            for c in needs_io {
                report.edge_sampled_files += 1;
                if let Some(fp) = edge_fingerprint(&c.path, c.bytes) {
                    edge_buckets.entry(fp).or_default().push(c);
                }
            }

            for (_fp, bucket) in edge_buckets {
                if bucket.len() < 2 && !has_cas_peers {
                    continue;
                }
                for c in bucket {
                    if cancel.load(Ordering::Relaxed) {
                        break;
                    }
                    report.full_hashed_files += 1;
                    if let Some(hex) = stream_sha256_hex(&c.path, cancel) {
                        digest_buckets.entry(hex).or_default().push(c);
                    }
                }
            }
        }

        for (sha256, mut matched) in digest_buckets {
            if matched.len() < 2 {
                continue;
            }
            matched.sort_by(|a, b| a.modified.cmp(&b.modified).then_with(|| a.path.cmp(&b.path)));

            let mut ecosystems_set: Vec<String> = Vec::new();
            let locations: Vec<BlobLocation> = matched
                .into_iter()
                .map(|c| {
                    let (eco, hint) = classify_ecosystem(&c.path);
                    let eco_str = eco.to_string();
                    if !ecosystems_set.contains(&eco_str) {
                        ecosystems_set.push(eco_str.clone());
                    }
                    BlobLocation {
                        path: c.path,
                        ecosystem: eco_str,
                        model_hint: hint,
                        modified: c.modified,
                    }
                })
                .collect();

            let cross_ecosystem = ecosystems_set.len() > 1;
            let wasted_bytes = bytes * (locations.len() as u64 - 1);
            let remediation = build_remediation(&ecosystems_set, cross_ecosystem);

            report.total_wasted_bytes += wasted_bytes;
            if cross_ecosystem {
                report.cross_ecosystem_wasted_bytes += wasted_bytes;
            }
            report.groups.push(BlobDupGroup {
                sha256,
                bytes,
                wasted_bytes,
                cross_ecosystem,
                ecosystems: ecosystems_set,
                locations,
                remediation,
            });
        }
    }

    report.groups.sort_by(|a, b| {
        b.wasted_bytes
            .cmp(&a.wasted_bytes)
            .then_with(|| a.sha256.cmp(&b.sha256))
    });
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_blob(path: &Path, data: &[u8]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }

    fn sha256_of(data: &[u8]) -> String {
        let digest = Sha256::digest(data);
        let mut s = String::with_capacity(64);
        for b in digest {
            use std::fmt::Write;
            let _ = write!(&mut s, "{b:02x}");
        }
        s
    }

    #[test]
    fn detects_cross_ecosystem_hf_ollama_and_downloads_duplicates() {
        let root = std::env::temp_dir().join(format!("slclean-blob-eco-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        let weights = vec![0x42u8; 32 * 1024];
        let hex = sha256_of(&weights);

        let hf_blob = root
            .join(".cache/huggingface/hub/models--Qwen--Qwen2.5-7B-Instruct-GGUF/blobs")
            .join(&hex);
        let ollama_blob = root
            .join(".ollama/models/blobs")
            .join(format!("sha256-{hex}"));
        let dl_gguf = root.join("Downloads/qwen2.5-7b-q4_k_m.gguf");

        write_blob(&hf_blob, &weights);
        write_blob(&ollama_blob, &weights);
        write_blob(&dl_gguf, &weights);

        // Same byte length and identical first 4 KiB header, but different tail bytes: must NOT group!
        let mut impostor = weights.clone();
        *impostor.last_mut().unwrap() = 0x99;
        write_blob(&root.join("Downloads/impostor.gguf"), &impostor);

        // Unique size: should read 0 bytes in phase 2/3.
        write_blob(&root.join("Downloads/unique.safetensors"), &[0x11u8; 40 * 1024]);

        let report = scan_blobs(&[root.clone()], 1024, false, &AtomicBool::new(false));
        assert_eq!(report.groups.len(), 1, "{report:?}");
        let g = &report.groups[0];
        assert_eq!(g.sha256, hex);
        assert_eq!(g.locations.len(), 3);
        assert_eq!(g.wasted_bytes, 2 * 32 * 1024);
        assert!(g.cross_ecosystem);
        assert!(g.ecosystems.contains(&"huggingface".to_string()));
        assert!(g.ecosystems.contains(&"ollama".to_string()));
        assert!(g.ecosystems.contains(&"downloads".to_string()));
        let hf_loc = g.locations.iter().find(|l| l.ecosystem == "huggingface").unwrap();
        assert_eq!(hf_loc.model_hint.as_deref(), Some("Qwen/Qwen2.5-7B-Instruct-GGUF"));

        // Verify CAS fast-path produces the exact same match while using CAS digests for the 2 CAS blobs.
        let fast = scan_blobs(&[root.clone()], 1024, true, &AtomicBool::new(false));
        assert_eq!(fast.groups.len(), 1);
        assert_eq!(fast.cas_fastpath_files, 2);
        assert_eq!(fast.groups[0].locations.len(), 3);
        assert_eq!(fast.total_wasted_bytes, 2 * 32 * 1024);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn skips_hf_snapshots_and_deduplicates_physical_hardlinks() {
        let root = std::env::temp_dir().join(format!("slclean-blob-links-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        let weights = vec![0x7Au8; 16 * 1024];
        let hex = sha256_of(&weights);

        let blob = root
            .join("huggingface/hub/models--meta-llama--Llama-3.2-3B/blobs")
            .join(&hex);
        let snapshot_copy = root
            .join("huggingface/hub/models--meta-llama--Llama-3.2-3B/snapshots/main/model.safetensors");
        write_blob(&blob, &weights);
        // Even if snapshots/ contains a materialized file or pointer, snapshots/ is skipped as HF view layer.
        write_blob(&snapshot_copy, &weights);

        // Hardlink to the exact same physical inode in LM Studio: consumes 0 extra disk blocks!
        let hardlinked = root.join(".lmstudio/models/meta-llama/Llama-3.2-3B/model.safetensors");
        fs::create_dir_all(hardlinked.parent().unwrap()).unwrap();
        fs::hard_link(&blob, &hardlinked).unwrap();

        let report = scan_blobs(&[root.clone()], 1024, false, &AtomicBool::new(false));
        assert!(report.groups.is_empty(), "hardlink + snapshots view should produce 0 duplicate waste: {report:?}");
        assert_eq!(report.hardlinks_skipped, 1);
        assert_eq!(report.hardlinked_saved_bytes, 16 * 1024);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn cas_digest_extraction_validates_strict_hex64() {
        let ollama = Path::new("/home/u/.ollama/models/blobs/sha256-e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(
            extract_cas_sha256(ollama).as_deref(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        let hf = Path::new("/home/u/.cache/huggingface/hub/models--x--y/blobs/E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855");
        assert_eq!(
            extract_cas_sha256(hf).as_deref(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        // Not inside blobs/ and no sha256- prefix -> None
        let non_cas = Path::new("/home/u/Downloads/e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert!(extract_cas_sha256(non_cas).is_none());
        // Short hex -> None
        let short = Path::new("/home/u/.ollama/models/blobs/sha256-deadbeef");
        assert!(extract_cas_sha256(short).is_none());
    }
}
