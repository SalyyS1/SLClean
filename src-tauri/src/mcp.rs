//! Read-only JSON-RPC 2.0 Stdio MCP Server for SLClean (`SLClean --mcp`).
//!
//! Strictly adheres to SLClean's safety rule: **zero automatic deletion**.
//! Exposes 5 read-only disk/project/hygiene/ledger inspection tools so AI coding
//! agents (Claude Code, Cursor, Windsurf, Codex, Antigravity) can diagnose disk bloat,
//! stale `.git` build artifacts, empty dirs / broken symlinks, and cleanup history.

use crate::{artifacts, big_files, blob_dupes, catalog_specs, dead_links, disk_health, drives, env_reclaimer, history, ledger, project_roots, projects, settings, startup_audit};
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
            "name": "slclean_audit_startup_entries",
            "description": "Classify Windows startup entries (Run keys, Startup folders, Scheduled Tasks) by boot impact (high/medium/low/broken/protected), compute login latency overhead in ms, and return reversible recommendations (read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entries": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "command": { "type": "string" },
                                "enabled": { "type": "boolean", "default": true },
                                "broken": { "type": "boolean", "default": false },
                                "system": { "type": "boolean", "default": false },
                                "stale": { "type": "boolean", "default": false }
                            },
                            "required": ["name", "command"]
                        }
                    },
                    "verify_local_paths": { "type": "boolean", "default": false }
                },
                "required": ["entries"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_extension_breakdown",
            "description": "Scan directories for large files above min_mb and aggregate storage consumption by file extension & category (ai-model, vm-disk, video, archive, installer, logs-dump; read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roots": { "type": "array", "items": { "type": "string" } },
                    "min_mb": { "type": "number", "minimum": 0.001, "default": 50.0 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 15 }
                },
                "required": ["roots"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_digest_html",
            "description": "Generate a self-contained dark-mode executive HTML workstation storage digest combining Drive Pressure (A-F), Clean Run Ledger summary, and non-destructive recommendations (read-only).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_inspect_settings",
            "description": "Inspect active SLClean workstation settings (language, Recycle Bin default, Simple Mode, free-space budget) and validate extra/excluded project paths for missing or redundant entries (read-only).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_plan_safe_cleanup",
            "description": "Generate a strictly non-destructive, safety-ordered workstation cleanup roadmap (Safe caches -> Rebuildable .git artifacts -> Duplicate AI blobs -> Large file review) with exact dry-run CLI commands (read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target_free_gb": { "type": "number", "minimum": 1.0, "default": 20.0 }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_doctor_sarif",
            "description": "Export Disk Health Doctor findings (SLC011 critical <5% free, SLC012 warning <15% free, SLC013 low grade) as OASIS SARIF v2.1.0 JSON for GitHub Code Scanning (read-only).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_startup_sarif",
            "description": "Audit startup entries and export heavy/stale (SLC014) and broken executable (SLC015) boot-impact findings as OASIS SARIF v2.1.0 JSON (read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entries": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "command": { "type": "string" },
                                "enabled": { "type": "boolean", "default": true },
                                "broken": { "type": "boolean", "default": false },
                                "system": { "type": "boolean", "default": false },
                                "stale": { "type": "boolean", "default": false }
                            },
                            "required": ["name", "command"]
                        }
                    },
                    "verify_local_paths": { "type": "boolean", "default": false }
                },
                "required": ["entries"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_startup_html",
            "description": "Audit startup entries and render a self-contained dark-mode HTML Startup Boot-Impact Report (read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "entries": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "command": { "type": "string" },
                                "enabled": { "type": "boolean", "default": true },
                                "broken": { "type": "boolean", "default": false },
                                "system": { "type": "boolean", "default": false },
                                "stale": { "type": "boolean", "default": false }
                            },
                            "required": ["name", "command"]
                        }
                    },
                    "verify_local_paths": { "type": "boolean", "default": false }
                },
                "required": ["entries"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_extensions_html",
            "description": "Scan directories for large files above min_mb and render a self-contained dark-mode HTML Storage Breakdown by File Extension Report (read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roots": { "type": "array", "items": { "type": "string" } },
                    "min_mb": { "type": "number", "minimum": 0.001, "default": 50.0 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 15 }
                },
                "required": ["roots"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_doctor_badge",
            "description": "Compute workstation Disk Health Doctor grade (A-F) and emit a shields.io Endpoint Badge JSON payload (read-only).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_cleanup_plan_html",
            "description": "Generate a self-contained dark-mode HTML Safe Cleanup Roadmap with strictly non-destructive dry-run CLI commands (read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target_free_gb": { "type": "number", "minimum": 1.0, "default": 20.0 }
                },
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_list_rules",
            "description": "List SLClean's built-in cache catalog specs and OASIS SARIF v2.1.0 rules (SLC001-SLC015).",
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
        "slclean_audit_startup_entries" => {
            let raw_entries = args.and_then(|a| a.get("entries")).cloned().unwrap_or(Value::Null);
            let parsed: Result<Vec<startup_audit::StartupAuditInput>, _> = serde_json::from_value(raw_entries);
            match parsed {
                Ok(entries) => {
                    let verify_paths = args
                        .and_then(|a| a.get("verify_local_paths"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let rep = startup_audit::audit_entries(&entries, verify_paths);
                    tool_outcome(id, serde_json::to_value(&rep).unwrap_or_else(|_| json!({})), false)
                }
                Err(err) => tool_outcome(id, json!({ "error": format!("Invalid 'entries' array: {}", err) }), true),
            }
        }
        "slclean_extension_breakdown" => {
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
                .unwrap_or(50.0)
                .max(0.001);
            let limit = args
                .and_then(|a| a.get("limit"))
                .and_then(|v| v.as_u64())
                .unwrap_or(15)
                .clamp(1, 50) as usize;
            let threshold = (min_mb * 1024.0 * 1024.0).round() as u64;
            let cancel = AtomicBool::new(false);
            let scan = big_files::scan(&roots, &[], threshold, &cancel, |_| {}, |_| {});
            let buckets = big_files::summarize_extensions(&scan.files, limit);
            let matched_bytes: u64 = scan.files.iter().map(|f| f.bytes).sum();
            tool_outcome(
                id,
                json!({
                    "threshold_bytes": threshold,
                    "matched_files": scan.files.len(),
                    "matched_bytes": matched_bytes,
                    "extension_count": buckets.len(),
                    "extensions": buckets,
                }),
                false,
            )
        }
        "slclean_export_digest_html" => {
            let rep = disk_health::run_default();
            let html = disk_health::render_digest_html(&rep);
            tool_outcome(
                id,
                json!({
                    "grade": rep.grade,
                    "score": rep.score,
                    "html_bytes": html.len(),
                    "html": html,
                }),
                false,
            )
        }
        "slclean_inspect_settings" => {
            let s = settings::load_offline();
            let audit = settings::audit_settings(&s);
            tool_outcome(id, serde_json::to_value(&audit).unwrap_or_else(|_| json!({})), false)
        }
        "slclean_plan_safe_cleanup" => {
            let target_gb = args
                .and_then(|a| a.get("target_free_gb"))
                .and_then(|v| v.as_f64())
                .unwrap_or(20.0)
                .max(1.0);
            let doc = disk_health::run_default();
            let specs_count = catalog_specs::spec_metadata().len();
            let steps = vec![
                json!({
                    "phase": 1,
                    "safety": "safe",
                    "title": "Preview Safe App & Browser Caches (Dry-Run)",
                    "command": "SLClean --clean-safe --dry-run",
                    "deletes_automatically": false
                }),
                json!({
                    "phase": 2,
                    "safety": "rebuildable",
                    "title": "Scan Stale .git Project Build Targets (>30 days untouched, SLC009)",
                    "command": "SLClean --scan-projects --stale-days 30 --report-html slclean-projects.html",
                    "deletes_automatically": false
                }),
                json!({
                    "phase": 3,
                    "safety": "review",
                    "title": "Audit Byte-Identical Duplicate AI Weights (.safetensors/.gguf, SLC004)",
                    "command": "SLClean --scan-dupes --trust-cas-names --sarif slclean-dupes.sarif",
                    "deletes_automatically": false
                }),
                json!({
                    "phase": 4,
                    "safety": "review",
                    "title": "Export Executive Workstation Storage Digest (HTML)",
                    "command": "SLClean --doctor --report-html slclean-digest.html",
                    "deletes_automatically": false
                }),
            ];
            tool_outcome(
                id,
                json!({
                    "target_free_gb": target_gb,
                    "current_grade": doc.grade,
                    "current_score": doc.score,
                    "lowest_free_pct": doc.lowest_free_pct,
                    "catalog_spec_count": specs_count,
                    "zero_auto_delete_enforced": true,
                    "steps": steps,
                }),
                false,
            )
        }
        "slclean_export_doctor_sarif" => {
            let doc = disk_health::run_default();
            let sarif = disk_health::render_doctor_sarif(&doc);
            tool_outcome(
                id,
                json!({
                    "grade": doc.grade,
                    "score": doc.score,
                    "sarif_bytes": sarif.len(),
                    "sarif": sarif,
                }),
                false,
            )
        }
        "slclean_export_startup_sarif" => {
            let Some(arr) = args.and_then(|a| a.get("entries")).and_then(|v| v.as_array()) else {
                return tool_outcome(id, json!({ "error": "'entries' must be an array of startup entry objects." }), true);
            };
            let verify_paths = args
                .and_then(|a| a.get("verify_local_paths"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut inputs = Vec::with_capacity(arr.len());
            for item in arr {
                if let Ok(parsed) = serde_json::from_value::<startup_audit::StartupAuditInput>(item.clone()) {
                    inputs.push(parsed);
                }
            }
            let rep = startup_audit::audit_entries(&inputs, verify_paths);
            let sarif = startup_audit::render_startup_sarif(&rep, env!("CARGO_PKG_VERSION"));
            tool_outcome(
                id,
                json!({
                    "grade": rep.grade,
                    "boot_health_score": rep.boot_health_score,
                    "high_impact_count": rep.high_impact_count,
                    "broken_count": rep.broken_count,
                    "sarif_bytes": sarif.len(),
                    "sarif": sarif,
                }),
                false,
            )
        }
        "slclean_export_startup_html" => {
            let Some(arr) = args.and_then(|a| a.get("entries")).and_then(|v| v.as_array()) else {
                return tool_outcome(id, json!({ "error": "'entries' must be an array of startup entry objects." }), true);
            };
            let verify_paths = args
                .and_then(|a| a.get("verify_local_paths"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut inputs = Vec::with_capacity(arr.len());
            for item in arr {
                if let Ok(parsed) = serde_json::from_value::<startup_audit::StartupAuditInput>(item.clone()) {
                    inputs.push(parsed);
                }
            }
            let rep = startup_audit::audit_entries(&inputs, verify_paths);
            let html = startup_audit::render_startup_html(&rep);
            tool_outcome(
                id,
                json!({
                    "grade": rep.grade,
                    "boot_health_score": rep.boot_health_score,
                    "html_bytes": html.len(),
                    "html": html,
                }),
                false,
            )
        }
        "slclean_export_extensions_html" => {
            let roots: Vec<PathBuf> = args
                .and_then(|a| a.get("roots"))
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(PathBuf::from)).collect())
                .unwrap_or_default();
            if roots.is_empty() {
                return tool_outcome(id, json!({ "error": "'roots' must be a non-empty array of directories." }), true);
            }
            let min_mb = args.and_then(|a| a.get("min_mb")).and_then(|v| v.as_f64()).unwrap_or(50.0).max(0.001);
            let limit = args.and_then(|a| a.get("limit")).and_then(|v| v.as_u64()).unwrap_or(15).clamp(1, 50) as usize;
            let min_bytes = (min_mb * 1024.0 * 1024.0).round() as u64;
            let cancel = AtomicBool::new(false);
            let scan = big_files::scan(&roots, &[], min_bytes, &cancel, |_| {}, |_| {});
            let buckets = big_files::summarize_extensions(&scan.files, limit);
            let html = big_files::render_extensions_html(&buckets, scan.files.len(), min_bytes);
            tool_outcome(
                id,
                json!({
                    "matched_files": scan.files.len(),
                    "extension_buckets": buckets.len(),
                    "html_bytes": html.len(),
                    "html": html,
                }),
                false,
            )
        }
        "slclean_export_doctor_badge" => {
            let doc = disk_health::run_default();
            let badge_raw = disk_health::render_doctor_badge_json(&doc);
            let badge_val: Value = serde_json::from_str(&badge_raw).unwrap_or_else(|_| json!({}));
            tool_outcome(
                id,
                json!({
                    "grade": doc.grade,
                    "score": doc.score,
                    "badge": badge_val,
                }),
                false,
            )
        }
        "slclean_export_cleanup_plan_html" => {
            let target_gb = args.and_then(|a| a.get("target_free_gb")).and_then(|v| v.as_f64()).unwrap_or(20.0).max(1.0);
            let doc = disk_health::run_default();
            let html = format!(
                "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>SLClean v{} Safe Cleanup Roadmap</title>\
<style>body{{background:#0f1411;color:#e6efe9;font-family:system-ui,sans-serif;padding:28px}}\
.wrap{{max-width:860px;margin:0 auto}}.card{{background:#161d19;border:1px solid #2a3730;border-radius:10px;padding:18px;margin-bottom:14px}}\
code{{font-family:ui-monospace,monospace;background:#1c2520;padding:3px 6px;border-radius:4px;color:#4ade80}}</style></head>\
<body><div class=\"wrap\"><div class=\"card\"><h1>SLClean Safe Cleanup Roadmap (Read-Only)</h1>\
<p>Current Disk Grade: <b>{}</b> ({}/100) &middot; Lowest Free: <b>{:.1}%</b> &middot; Target Free: <b>{:.1} GiB</b></p></div>\
<div class=\"card\"><ol>\
<li><b>[SAFE]</b> <code>SLClean --clean-safe --dry-run</code></li>\
<li><b>[REBUILDABLE]</b> <code>SLClean --scan-projects --stale-days 30 --report-html slclean-projects.html</code></li>\
<li><b>[REVIEW]</b> <code>SLClean --scan-dupes --trust-cas-names --sarif slclean-dupes.sarif</code></li>\
<li><b>[VERIFY]</b> <code>SLClean --doctor --report-html slclean-digest.html --sarif slclean-health.sarif</code></li>\
</ol></div></div></body></html>",
                env!("CARGO_PKG_VERSION"), doc.grade, doc.score, doc.lowest_free_pct, target_gb
            );
            tool_outcome(
                id,
                json!({
                    "grade": doc.grade,
                    "score": doc.score,
                    "target_free_gb": target_gb,
                    "html_bytes": html.len(),
                    "html": html,
                }),
                false,
            )
        }
        "slclean_list_rules" => {
            let specs = catalog_specs::spec_metadata();
            tool_outcome(
                id,
                json!({
                    "catalog_spec_count": specs.len(),
                    "sarif_rules": ["SLC001","SLC002","SLC003","SLC004","SLC005","SLC006","SLC007","SLC008","SLC009","SLC010","SLC011","SLC012","SLC013","SLC014","SLC015"],
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
        assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 21);

        let startup_sarif_call = json!({
            "jsonrpc": "2.0",
            "id": 23,
            "method": "tools/call",
            "params": {
                "name": "slclean_export_startup_sarif",
                "arguments": {
                    "entries": [
                        { "name": "Discord", "command": "Discord.exe", "enabled": true, "stale": true },
                        { "name": "Ghost", "command": "C:\\Missing\\ghost.exe", "enabled": true, "broken": true }
                    ]
                }
            }
        });
        let startup_sarif_resp: Value = serde_json::from_str(&handle_message(&startup_sarif_call.to_string()).unwrap()).unwrap();
        assert_eq!(startup_sarif_resp["result"]["isError"], false);
        let startup_sarif_txt = startup_sarif_resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(startup_sarif_txt.contains("SLC014"));
        assert!(startup_sarif_txt.contains("SLC015"));

        let doc_sarif_call = json!({
            "jsonrpc": "2.0",
            "id": 22,
            "method": "tools/call",
            "params": { "name": "slclean_export_doctor_sarif", "arguments": {} }
        });
        let doc_sarif_resp: Value = serde_json::from_str(&handle_message(&doc_sarif_call.to_string()).unwrap()).unwrap();
        assert_eq!(doc_sarif_resp["result"]["isError"], false);
        assert!(doc_sarif_resp["result"]["content"][0]["text"].as_str().unwrap().contains("2.1.0"));

        let startup_call = json!({
            "jsonrpc": "2.0",
            "id": 21,
            "method": "tools/call",
            "params": {
                "name": "slclean_audit_startup_entries",
                "arguments": {
                    "entries": [
                        { "name": "Discord", "command": "Discord.exe", "enabled": true, "stale": true },
                        { "name": "SecurityHealth", "command": "C:\\Windows\\System32\\SecurityHealthSystray.exe", "system": true }
                    ]
                }
            }
        });
        let startup_resp: Value = serde_json::from_str(&handle_message(&startup_call.to_string()).unwrap()).unwrap();
        assert_eq!(startup_resp["result"]["isError"], false);
        let startup_payload: Value = serde_json::from_str(startup_resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(startup_payload["high_impact_count"], 1);

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
