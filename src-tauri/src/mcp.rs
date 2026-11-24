//! Read-only JSON-RPC 2.0 Stdio MCP Server for SLClean (`SLClean --mcp`).
//!
//! Strictly adheres to SLClean's safety rule: **zero automatic deletion**.
//! Exposes 5 read-only disk/project/hygiene/ledger inspection tools so AI coding
//! agents (Claude Code, Cursor, Windsurf, Codex, Antigravity) can diagnose disk bloat,
//! stale `.git` build artifacts, empty dirs / broken symlinks, and cleanup history.

use crate::{artifacts, big_files, blob_dupes, catalog_specs, dead_links, disk_health, drives, env_reclaimer, history, ledger, project_roots, projects};
use serde_json::{json, Value};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const PROTOCOL_VERSION: &str = "2025-06-18";

fn rpc_result(id: &Value, result: Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
    .to_string()
}

fn rpc_error(id: &Value, code: i32, message: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        }
    })
    .to_string()
}

fn tool_outcome(id: &Value, payload: Value, is_error: bool) -> String {
    let text = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
    rpc_result(
        id,
        json!({
            "content": [{ "type": "text", "text": text }],
            "isError": is_error,
        }),
    )
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "slclean_list_drives",
            "description": "List mounted fixed drives with total bytes, free bytes, and usage percentage (read-only).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_scan_projects",
            "description": "Scan directories for rebuildable project build artifacts (target/, node_modules/, build/, .venv/) grouped by repository with .git staleness days (SLC009, read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roots": { "type": "array", "items": { "type": "string" } },
                    "stale_days": { "type": "integer", "minimum": 1, "default": 30 }
                },
                "required": ["roots"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_scan_hygiene",
            "description": "Scan directories for empty leaf folders and broken symlinks honoring SLClean's protected path guards (SLC010, read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roots": { "type": "array", "items": { "type": "string" } },
                    "max_depth": { "type": "integer", "minimum": 1, "maximum": 12, "default": 5 }
                },
                "required": ["roots"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_scan_dupes",
            "description": "Detect byte-identical duplicate AI model weights (.safetensors, .gguf, .bin) and large blobs across directories with zero-read CAS fastpath (SLC004, read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roots": { "type": "array", "items": { "type": "string" } },
                    "min_mb": { "type": "number", "minimum": 0.1, "default": 10.0 },
                    "trust_cas_names": { "type": "boolean", "default": true }
                },
                "required": ["roots"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_scan_big_files",
            "description": "Scan directories for large files above min_mb and top directory consumers (read-only, skips Windows system files).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roots": { "type": "array", "items": { "type": "string" } },
                    "min_mb": { "type": "number", "minimum": 0.001, "default": 100.0 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 50 }
                },
                "required": ["roots"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_ledger_summary",
            "description": "Inspect SLClean's persistent Clean Run Ledger summary (lifetime reclaimed bytes, total runs, Recycle Bin vs Permanent counts, recent runs).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_scan_envs",
            "description": "Scan directories for orphaned Python virtualenvs, manifest-less node_modules, stale Cargo/Gradle targets, and guarded WSL2/Docker .vhdx virtual disks (SLC006-SLC008, read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roots": { "type": "array", "items": { "type": "string" } },
                    "stale_days": { "type": "integer", "minimum": 1, "default": 30 }
                },
                "required": ["roots"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_history_diff",
            "description": "Inspect SLClean scan history snapshots and compute what caches/artifacts grew between the two most recent scans (read-only).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_doctor",
            "description": "Compute an executive A-F workstation storage health diagnosis across all fixed drives and Clean Run Ledger history (read-only).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_list_rules",
            "description": "List SLClean's built-in cache catalog specs and OASIS SARIF v2.1.0 rules (SLC001-SLC010).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true }
        }
    ])
}

/// Process a single JSON-RPC 2.0 request payload. Returns `None` for notifications.
pub fn handle_message(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let val: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(e) => return Some(rpc_error(&Value::Null, -32700, &format!("Malformed JSON-RPC: {}", e))),
    };
    let Some(obj) = val.as_object() else {
        return Some(rpc_error(&Value::Null, -32600, "JSON-RPC payload must be an object."));
    };
    let id = obj.get("id")?;
    let Some(method) = obj.get("method").and_then(|v| v.as_str()) else {
        return Some(rpc_error(id, -32600, "Missing JSON-RPC method."));
    };
    let params = obj.get("params").and_then(|v| v.as_object());

    match method {
        "initialize" => Some(rpc_result(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "serverInfo": {
                    "name": "slclean",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {
                    "tools": { "listChanged": false }
                },
                "instructions": "SLClean read-only disk, project artifact, and hygiene inspector. Zero automatic deletion."
            }),
        )),
        "ping" => Some(rpc_result(id, json!({}))),
        "tools/list" => Some(rpc_result(id, json!({ "tools": tool_definitions() }))),
        "tools/call" => Some(handle_tool_call(id, params)),
        other => Some(rpc_error(id, -32601, &format!("Method not found: {}", other))),
    }
}

fn handle_tool_call(id: &Value, params: Option<&serde_json::Map<String, Value>>) -> String {
    let Some(name) = params.and_then(|p| p.get("name")).and_then(|v| v.as_str()) else {
        return rpc_error(id, -32602, "Missing string 'name' in tools/call.");
    };
    let args = params.and_then(|p| p.get("arguments")).and_then(|v| v.as_object());

    match name {
        "slclean_list_drives" => {
            let list = drives::list_drives();
            tool_outcome(id, json!({ "drive_count": list.len(), "drives": list }), false)
        }
        "slclean_scan_projects" => {
            let roots: Vec<PathBuf> = args
                .and_then(|a| a.get("roots"))
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(PathBuf::from)).collect())
                .unwrap_or_default();
            if roots.is_empty() {
                return tool_outcome(id, json!({ "error": "'roots' must be a non-empty array of directories." }), true);
            }
            let stale_days = args
                .and_then(|a| a.get("stale_days"))
                .and_then(|v| v.as_u64())
                .unwrap_or(projects::DEFAULT_STALE_PROJECT_DAYS);
            let cancel = Arc::new(AtomicBool::new(false));
            let plan = project_roots::ScanPlan { roots, excluded: Vec::new() };
            let found = artifacts::find_artifacts(&plan, &cancel, |_| {});
            let report = projects::group_artifacts_by_project(&found, stale_days, None);
            tool_outcome(id, serde_json::to_value(&report).unwrap_or_else(|_| json!({})), false)
        }
        "slclean_scan_hygiene" => {
            let roots: Vec<PathBuf> = args
                .and_then(|a| a.get("roots"))
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(PathBuf::from)).collect())
                .unwrap_or_default();
            if roots.is_empty() {
                return tool_outcome(id, json!({ "error": "'roots' must be a non-empty array of directories." }), true);
            }
            let max_depth = args
                .and_then(|a| a.get("max_depth"))
                .and_then(|v| v.as_u64())
                .unwrap_or(5)
                .clamp(1, 12) as usize;
            let report = dead_links::scan(&roots, max_depth);
            tool_outcome(id, serde_json::to_value(&report).unwrap_or_else(|_| json!({})), false)
        }
        "slclean_scan_dupes" => {
            let roots: Vec<PathBuf> = args
                .and_then(|a| a.get("roots"))
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(PathBuf::from)).collect())
                .unwrap_or_default();
            if roots.is_empty() {
                return tool_outcome(id, json!({ "error": "'roots' must be a non-empty array of directories." }), true);
            }
            let min_mb = args
                .and_then(|a| a.get("min_mb"))
                .and_then(|v| v.as_f64())
                .unwrap_or(10.0)
                .max(0.001);
            let min_bytes = (min_mb * 1024.0 * 1024.0).round() as u64;
            let trust_cas = args
                .and_then(|a| a.get("trust_cas_names"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let cancel = AtomicBool::new(false);
            let report = blob_dupes::scan_blobs(&roots, min_bytes, trust_cas, &cancel);
            tool_outcome(id, serde_json::to_value(&report).unwrap_or_else(|_| json!({})), false)
        }
        "slclean_scan_big_files" => {
            let roots: Vec<PathBuf> = args
                .and_then(|a| a.get("roots"))
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(PathBuf::from)).collect())
                .unwrap_or_default();
            if roots.is_empty() {
                return tool_outcome(id, json!({ "error": "'roots' must be a non-empty array of directories." }), true);
            }
            let min_mb = args
                .and_then(|a| a.get("min_mb"))
                .and_then(|v| v.as_f64())
                .unwrap_or(100.0)
                .max(0.001);
            let limit = args
                .and_then(|a| a.get("limit"))
                .and_then(|v| v.as_u64())
                .unwrap_or(50)
                .clamp(1, 200) as usize;
            let threshold = (min_mb * 1024.0 * 1024.0).round() as u64;
            let cancel = AtomicBool::new(false);
            let mut scan = big_files::scan(&roots, &[], threshold, &cancel, |_| {}, |_| {});
            let total_matches = scan.files.len();
            scan.files.truncate(limit);
            tool_outcome(
                id,
                json!({
                    "threshold_bytes": threshold,
                    "total_matches": total_matches,
                    "returned_files": scan.files.len(),
                    "scanned_files": scan.scanned_files,
                    "scanned_bytes": scan.scanned_bytes,
                    "files": scan.files,
                    "dir_map": scan.map,
                }),
                false,
            )
        }
        "slclean_ledger_summary" => {
            let l = ledger::load_default();
            let summary = l.summary();
            let recent: Vec<_> = l.runs.iter().rev().take(10).cloned().collect();
            tool_outcome(
                id,
                json!({
                    "summary": summary,
                    "recent_runs": recent,
                }),
                false,
            )
        }
        "slclean_scan_envs" => {
            let roots: Vec<PathBuf> = args
                .and_then(|a| a.get("roots"))
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(PathBuf::from)).collect())
                .unwrap_or_default();
            if roots.is_empty() {
                return tool_outcome(id, json!({ "error": "'roots' must be a non-empty array of directories." }), true);
            }
            let stale_days = args
                .and_then(|a| a.get("stale_days"))
                .and_then(|v| v.as_u64())
                .unwrap_or(env_reclaimer::DEFAULT_STALE_DAYS);
            let cancel = AtomicBool::new(false);
            let report = env_reclaimer::scan_environments(&roots, stale_days, &cancel);
            tool_outcome(id, serde_json::to_value(&report).unwrap_or_else(|_| json!({})), false)
        }
        "slclean_history_diff" => {
            let hist = history::load_default();
            let count = hist.snapshots.len();
            let latest_diff = if count >= 2 {
                Some(history::diff(&hist.snapshots[count - 2], &hist.snapshots[count - 1]))
            } else {
                None
            };
            tool_outcome(
                id,
                json!({
                    "snapshot_count": count,
                    "snapshots": history::list(&hist),
                    "latest_diff": latest_diff,
                }),
                false,
            )
        }
        "slclean_doctor" => {
            let rep = disk_health::run_default();
            tool_outcome(id, serde_json::to_value(&rep).unwrap_or_else(|_| json!({})), false)
        }
        "slclean_list_rules" => {
            let specs = catalog_specs::spec_metadata();
            tool_outcome(
                id,
                json!({
                    "catalog_spec_count": specs.len(),
                    "sarif_rules": ["SLC001","SLC002","SLC003","SLC004","SLC005","SLC006","SLC007","SLC008","SLC009","SLC010"],
                    "specs": specs,
                }),
                false,
            )
        }
        unknown => rpc_error(id, -32602, &format!("Unknown tool: {}", unknown)),
    }
}

/// Run the JSON-RPC 2.0 stdio MCP loop (supports newline-delimited & Content-Length framing).
pub fn serve_stdio() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = stdout.lock();

    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (body, framed) = if trimmed.to_ascii_lowercase().starts_with("content-length:") {
            let len: usize = trimmed
                .split_once(':')
                .and_then(|(_, v)| v.trim().parse().ok())
                .unwrap_or(0);
            loop {
                let mut hdr = String::new();
                if reader.read_line(&mut hdr)? == 0 || hdr.trim().is_empty() {
                    break;
                }
            }
            let mut buf = vec![0u8; len];
            reader.read_exact(&mut buf)?;
            (String::from_utf8_lossy(&buf).into_owned(), true)
        } else {
            (trimmed.to_string(), false)
        };

        if let Some(resp) = handle_message(&body) {
            if framed {
                write!(writer, "Content-Length: {}\r\n\r\n{}", resp.len(), resp)?;
            } else {
                writeln!(writer, "{}", resp)?;
            }
            writer.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_server_lists_five_read_only_tools_and_scans_hygiene() {
        let init: Value = serde_json::from_str(
            &handle_message(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#).unwrap(),
        )
        .unwrap();
        assert_eq!(init["result"]["serverInfo"]["name"], "slclean");

        let tools: Value = serde_json::from_str(
            &handle_message(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#).unwrap(),
        )
        .unwrap();
        assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 10);

        let dir = std::env::temp_dir().join("slclean_mcp_hygiene_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("empty_leaf")).unwrap();
        std::fs::write(dir.join("heavy.bin"), vec![1u8; 4096]).unwrap();
        let call = json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "slclean_scan_hygiene",
                "arguments": { "roots": [dir.to_string_lossy()] }
            }
        });
        let resp: Value = serde_json::from_str(&handle_message(&call.to_string()).unwrap()).unwrap();
        assert_eq!(resp["result"]["isError"], false);
        let payload: Value = serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(payload["empty_dir_count"], 1);

        let big_call = json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "slclean_scan_big_files",
                "arguments": { "roots": [dir.to_string_lossy()], "min_mb": 0.002 }
            }
        });
        let big_resp: Value = serde_json::from_str(&handle_message(&big_call.to_string()).unwrap()).unwrap();
        assert_eq!(big_resp["result"]["isError"], false);
        let big_payload: Value = serde_json::from_str(big_resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(big_payload["returned_files"], 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
