//! Read-only JSON-RPC 2.0 Stdio MCP Server for SLClean (`SLClean --mcp`).
//!
//! Strictly adheres to SLClean's safety rule: **zero automatic deletion**.
//! Exposes 5 read-only disk/project/hygiene/ledger inspection tools so AI coding
//! agents (Claude Code, Cursor, Windsurf, Codex, Antigravity) can diagnose disk bloat,
//! stale `.git` build artifacts, empty dirs / broken symlinks, and cleanup history.

use crate::{artifacts, big_files, blob_dupes, catalog_specs, dead_links, disk_health, drives, env_reclaimer, history, ledger, project_roots, projects, report_export, settings, startup_audit};
use serde_json::{json, Value};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

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
            "name": "slclean_export_projects_badge",
            "description": "Scan directories for stale .git project build targets and emit a shields.io Endpoint Badge JSON summarizing reclaimable GiB (read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roots": { "type": "array", "items": { "type": "string" } },
                    "stale_days": { "type": "integer", "minimum": 0, "default": 30 }
                },
                "required": ["roots"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_dupes_badge",
            "description": "Scan directories for byte-identical duplicate AI model/build blobs and emit a shields.io Endpoint Badge JSON summarizing wasted MiB (read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roots": { "type": "array", "items": { "type": "string" } },
                    "min_dupe_bytes": { "type": "integer", "minimum": 1, "default": 1048576 }
                },
                "required": ["roots"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_envs_badge",
            "description": "Scan directories for orphaned/stale virtualenvs and build envs and emit a shields.io Endpoint Badge JSON (read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roots": { "type": "array", "items": { "type": "string" } },
                    "stale_days": { "type": "integer", "minimum": 0, "default": 30 }
                },
                "required": ["roots"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_hygiene_badge",
            "description": "Scan directories for empty leaf folders and broken symlinks (SLC010) and emit a shields.io Endpoint Badge JSON (read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roots": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["roots"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_dupes_html",
            "description": "Scan directories for byte-identical duplicate AI model/build blobs and render a self-contained dark-mode HTML Report (read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roots": { "type": "array", "items": { "type": "string" } },
                    "min_dupe_bytes": { "type": "integer", "minimum": 1, "default": 1048576 }
                },
                "required": ["roots"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_envs_html",
            "description": "Scan directories for orphaned/stale Python venvs, node_modules, Cargo targets & guarded WSL2 VHDX disks and render a self-contained dark-mode HTML Report (read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roots": { "type": "array", "items": { "type": "string" } },
                    "stale_days": { "type": "integer", "minimum": 0, "default": 30 }
                },
                "required": ["roots"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_hygiene_html",
            "description": "Scan directories for empty leaf folders and broken symlinks (SLC010) and render a self-contained dark-mode HTML Hygiene Report (read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roots": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["roots"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_extensions_badge",
            "description": "Scan directories for large files above min_mb and emit a shields.io Endpoint Badge JSON summarizing total matched storage and top file extension (read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roots": { "type": "array", "items": { "type": "string" } },
                    "min_mb": { "type": "number", "minimum": 0.001, "default": 50.0 }
                },
                "required": ["roots"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_projects_html",
            "description": "Scan directories for rebuildable .git project build targets (target/, node_modules/, build/, .venv/) and render a self-contained dark-mode HTML Project Audit Report (read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roots": { "type": "array", "items": { "type": "string" } },
                    "stale_days": { "type": "integer", "minimum": 0, "default": 30 }
                },
                "required": ["roots"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_big_files_html",
            "description": "Scan directories for large files above min_mb and render a self-contained dark-mode HTML Large File Consumers Report (read-only).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "roots": { "type": "array", "items": { "type": "string" } },
                    "min_mb": { "type": "number", "minimum": 0.001, "default": 50.0 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 200, "default": 50 }
                },
                "required": ["roots"],
                "additionalProperties": false
            },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_ledger_html",
            "description": "Render SLClean's persistent Clean Run Ledger (lifetime reclaimed bytes, Recycle Bin vs Permanent breakdown, and recent cleanup runs) as a self-contained dark-mode HTML Audit Trail (read-only).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_settings_html",
            "description": "Inspect active SLClean settings (language, Recycle Bin default, Simple Mode, free-space budget, extra/excluded path health) and render a self-contained dark-mode HTML Settings Audit Report (read-only).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_history_html",
            "description": "Render SLClean's persistent scan snapshot timeline (free space and tracked cache totals over time) as a self-contained dark-mode HTML Storage History Report (read-only).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_drives_badge",
            "description": "Inspect fixed local drives and emit a shields.io Endpoint Badge JSON summarizing total free GiB and minimum free percentage across mounted volumes (read-only).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_startup_badge",
            "description": "Audit startup entries and emit a shields.io Endpoint Badge JSON summarizing boot health grade (A-F) and estimated login overhead ms (read-only).",
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
            "name": "slclean_export_ledger_badge",
            "description": "Summarize SLClean's persistent Clean Run Ledger (lifetime GiB freed and total cleanup runs) as a shields.io Endpoint Badge JSON (read-only).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
            "annotations": { "readOnlyHint": true }
        },
        {
            "name": "slclean_export_settings_badge",
            "description": "Audit active SLClean settings & exclusion paths and emit a shields.io Endpoint Badge JSON health summary (read-only).",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
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
        "slclean_export_projects_badge" => {
            let roots: Vec<PathBuf> = args
                .and_then(|a| a.get("roots"))
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(PathBuf::from)).collect())
                .unwrap_or_default();
            if roots.is_empty() {
                return tool_outcome(id, json!({ "error": "'roots' must be a non-empty array of directories." }), true);
            }
            let stale_days = args.and_then(|a| a.get("stale_days")).and_then(|v| v.as_u64()).unwrap_or(projects::DEFAULT_STALE_PROJECT_DAYS);
            let cancel = Arc::new(AtomicBool::new(false));
            let plan = project_roots::ScanPlan { roots, excluded: Vec::new() };
            let found = artifacts::find_artifacts(&plan, &cancel, |_| {});
            let rep = projects::group_artifacts_by_project(&found, stale_days, None);
            let stale_gb = rep.stale_reclaimable_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            let color = if stale_gb >= 10.0 { "red" } else if stale_gb >= 2.0 { "yellow" } else { "brightgreen" };
            tool_outcome(
                id,
                json!({
                    "project_count": rep.project_count,
                    "stale_reclaimable_bytes": rep.stale_reclaimable_bytes,
                    "badge": {
                        "schemaVersion": 1,
                        "label": "slclean stale builds",
                        "message": format!("{:.2} GiB ({} projects)", stale_gb, rep.project_count),
                        "color": color,
                    }
                }),
                false,
            )
        }
        "slclean_export_dupes_badge" => {
            let roots: Vec<PathBuf> = args
                .and_then(|a| a.get("roots"))
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(PathBuf::from)).collect())
                .unwrap_or_default();
            if roots.is_empty() {
                return tool_outcome(id, json!({ "error": "'roots' must be a non-empty array of directories." }), true);
            }
            let min_dupe_bytes = args.and_then(|a| a.get("min_dupe_bytes")).and_then(|v| v.as_u64()).unwrap_or(1024 * 1024);
            let scan = blob_dupes::scan_blobs(&roots, min_dupe_bytes, false, &AtomicBool::new(false));
            let wasted_mb = scan.total_wasted_bytes as f64 / (1024.0 * 1024.0);
            let color = if wasted_mb >= 2048.0 { "red" } else if wasted_mb >= 256.0 { "yellow" } else { "brightgreen" };
            tool_outcome(
                id,
                json!({
                    "duplicate_group_count": scan.groups.len(),
                    "total_wasted_bytes": scan.total_wasted_bytes,
                    "badge": {
                        "schemaVersion": 1,
                        "label": "slclean dupes",
                        "message": format!("{:.1} MiB ({} groups)", wasted_mb, scan.groups.len()),
                        "color": color,
                    }
                }),
                false,
            )
        }
        "slclean_export_envs_badge" => {
            let roots: Vec<PathBuf> = args
                .and_then(|a| a.get("roots"))
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(PathBuf::from)).collect())
                .unwrap_or_default();
            if roots.is_empty() {
                return tool_outcome(id, json!({ "error": "'roots' must be a non-empty array of directories." }), true);
            }
            let stale_days = args.and_then(|a| a.get("stale_days")).and_then(|v| v.as_u64()).unwrap_or(30);
            let scan = env_reclaimer::scan_environments(&roots, stale_days, &AtomicBool::new(false));
            let gb = scan.total_reclaimable_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            let color = if scan.orphan_count > 0 || gb >= 5.0 { "yellow" } else { "brightgreen" };
            tool_outcome(
                id,
                json!({
                    "orphan_count": scan.orphan_count,
                    "total_reclaimable_bytes": scan.total_reclaimable_bytes,
                    "badge": {
                        "schemaVersion": 1,
                        "label": "slclean orphan envs",
                        "message": format!("{} orphans ({:.2} GiB)", scan.orphan_count, gb),
                        "color": color,
                    }
                }),
                false,
            )
        }
        "slclean_export_hygiene_badge" => {
            let roots: Vec<PathBuf> = args
                .and_then(|a| a.get("roots"))
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(PathBuf::from)).collect())
                .unwrap_or_default();
            if roots.is_empty() {
                return tool_outcome(id, json!({ "error": "'roots' must be a non-empty array of directories." }), true);
            }
            let rep = dead_links::scan(&roots, 5);
            let total = rep.empty_dir_count + rep.broken_link_count;
            let color = if rep.broken_link_count > 0 { "red" } else if total > 0 { "yellow" } else { "brightgreen" };
            tool_outcome(
                id,
                json!({
                    "empty_dir_count": rep.empty_dir_count,
                    "broken_link_count": rep.broken_link_count,
                    "badge": {
                        "schemaVersion": 1,
                        "label": "slclean hygiene",
                        "message": format!("{} empty · {} broken links", rep.empty_dir_count, rep.broken_link_count),
                        "color": color,
                    }
                }),
                false,
            )
        }
        "slclean_export_dupes_html" => {
            let roots: Vec<PathBuf> = args
                .and_then(|a| a.get("roots"))
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(PathBuf::from)).collect())
                .unwrap_or_default();
            if roots.is_empty() {
                return tool_outcome(id, json!({ "error": "'roots' must be a non-empty array of directories." }), true);
            }
            let min_dupe_bytes = args.and_then(|a| a.get("min_dupe_bytes")).and_then(|v| v.as_u64()).unwrap_or(1024 * 1024);
            let scan = blob_dupes::scan_blobs(&roots, min_dupe_bytes, false, &AtomicBool::new(false));
            let html = blob_dupes::render_dupes_html(&scan);
            tool_outcome(
                id,
                json!({
                    "duplicate_group_count": scan.groups.len(),
                    "total_wasted_bytes": scan.total_wasted_bytes,
                    "html_bytes": html.len(),
                    "html": html,
                }),
                false,
            )
        }
        "slclean_export_envs_html" => {
            let roots: Vec<PathBuf> = args
                .and_then(|a| a.get("roots"))
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(PathBuf::from)).collect())
                .unwrap_or_default();
            if roots.is_empty() {
                return tool_outcome(id, json!({ "error": "'roots' must be a non-empty array of directories." }), true);
            }
            let stale_days = args.and_then(|a| a.get("stale_days")).and_then(|v| v.as_u64()).unwrap_or(30);
            let scan = env_reclaimer::scan_environments(&roots, stale_days, &AtomicBool::new(false));
            let html = env_reclaimer::render_envs_html(&scan);
            tool_outcome(
                id,
                json!({
                    "orphan_count": scan.orphan_count,
                    "stale_count": scan.stale_count,
                    "guarded_vhdx_count": scan.guarded_vhdx_count,
                    "total_reclaimable_bytes": scan.total_reclaimable_bytes,
                    "html_bytes": html.len(),
                    "html": html,
                }),
                false,
            )
        }
        "slclean_export_hygiene_html" => {
            let roots: Vec<PathBuf> = args
                .and_then(|a| a.get("roots"))
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(PathBuf::from)).collect())
                .unwrap_or_default();
            if roots.is_empty() {
                return tool_outcome(id, json!({ "error": "'roots' must be a non-empty array of directories." }), true);
            }
            let rep = dead_links::scan(&roots, 5);
            let html = dead_links::render_hygiene_html(&rep);
            tool_outcome(
                id,
                json!({
                    "empty_dir_count": rep.empty_dir_count,
                    "broken_link_count": rep.broken_link_count,
                    "html_bytes": html.len(),
                    "html": html,
                }),
                false,
            )
        }
        "slclean_export_extensions_badge" => {
            let roots: Vec<PathBuf> = args
                .and_then(|a| a.get("roots"))
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(PathBuf::from)).collect())
                .unwrap_or_default();
            if roots.is_empty() {
                return tool_outcome(id, json!({ "error": "'roots' must be a non-empty array of directories." }), true);
            }
            let min_mb = args.and_then(|a| a.get("min_mb")).and_then(|v| v.as_f64()).unwrap_or(50.0).max(0.001);
            let min_bytes = (min_mb * 1024.0 * 1024.0).round() as u64;
            let scan = big_files::scan(&roots, &[], min_bytes, &AtomicBool::new(false), |_| {}, |_| {});
            let buckets = big_files::summarize_extensions(&scan.files, 10);
            let total_bytes: u64 = buckets.iter().map(|b| b.bytes).sum();
            let total_mb = total_bytes as f64 / (1024.0 * 1024.0);
            let top_ext = buckets.first().map(|b| format!(".{}", b.ext)).unwrap_or_else(|| "none".into());
            let color = if total_mb >= 10240.0 { "red" } else if total_mb >= 1024.0 { "yellow" } else { "brightgreen" };
            tool_outcome(
                id,
                json!({
                    "matched_files": scan.files.len(),
                    "top_extension": top_ext,
                    "total_bytes": total_bytes,
                    "badge": {
                        "schemaVersion": 1,
                        "label": "slclean big files",
                        "message": format!("{:.1} MiB (top {})", total_mb, top_ext),
                        "color": color,
                    }
                }),
                false,
            )
        }
        "slclean_export_projects_html" => {
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
            let led = ledger::load_default().summary();
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let html = report_export::render_html(&[], Some(&report), Some(&led), now);
            tool_outcome(
                id,
                json!({
                    "project_count": report.projects.len(),
                    "stale_project_count": report.stale_project_count,
                    "total_reclaimable_bytes": report.total_reclaimable_bytes,
                    "html_bytes": html.len(),
                    "html": html,
                }),
                false,
            )
        }
        "slclean_export_big_files_html" => {
            let roots: Vec<PathBuf> = args
                .and_then(|a| a.get("roots"))
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str().map(PathBuf::from)).collect())
                .unwrap_or_default();
            if roots.is_empty() {
                return tool_outcome(id, json!({ "error": "'roots' must be a non-empty array of directories." }), true);
            }
            let min_mb = args.and_then(|a| a.get("min_mb")).and_then(|v| v.as_f64()).unwrap_or(50.0).max(0.001);
            let limit = args.and_then(|a| a.get("limit")).and_then(|v| v.as_u64()).unwrap_or(50).clamp(1, 200) as usize;
            let min_bytes = (min_mb * 1024.0 * 1024.0).round() as u64;
            let cancel = AtomicBool::new(false);
            let mut scan = big_files::scan(&roots, &[], min_bytes, &cancel, |_| {}, |_| {});
            let matched = scan.files.len();
            let total_bytes: u64 = scan.files.iter().map(|f| f.bytes).sum();
            scan.files.truncate(limit);
            let mut rows = String::new();
            for f in &scan.files {
                let p = f.path.to_string_lossy().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
                rows.push_str(&format!(
                    "<tr><td><code>{}</code></td><td style=\"text-align:right\">{:.2} MiB</td></tr>\n",
                    p, f.bytes as f64 / (1024.0 * 1024.0)
                ));
            }
            if rows.is_empty() {
                rows.push_str("<tr><td colspan=\"2\" style=\"color:#93a89b\">No files matched the minimum size threshold.</td></tr>");
            }
            let html = format!(
                "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>SLClean Large Files Report</title><style>body{{margin:0;padding:28px;background:#101512;color:#e6efe9;font-family:system-ui,sans-serif}}.wrap{{max-width:980px;margin:0 auto}}table{{width:100%;border-collapse:collapse;background:#161d19;border:1px solid #2a3730}}th,td{{padding:8px 10px;border-bottom:1px solid #2a3730;font-size:13px;text-align:left}}th{{color:#93a89b;font-size:11px;text-transform:uppercase}}</style></head><body><div class=\"wrap\"><h1>SLClean Large File Consumers Report</h1><p>Matched: <b>{}</b> file(s) &middot; Total: <b>{:.2} MiB</b> (threshold {:.2} MiB)</p><table><thead><tr><th>Path</th><th style=\"text-align:right\">Size</th></tr></thead><tbody>{}</tbody></table></div></body></html>",
                matched,
                total_bytes as f64 / (1024.0 * 1024.0),
                min_mb,
                rows
            );
            tool_outcome(
                id,
                json!({
                    "matched_files": matched,
                    "total_bytes": total_bytes,
                    "html_bytes": html.len(),
                    "html": html,
                }),
                false,
            )
        }
        "slclean_export_ledger_html" => {
            let l = ledger::load_default();
            let s = l.summary();
            let mut rows = String::new();
            for r in l.runs.iter().rev().take(25) {
                let mode = if r.to_trash { "Recycle Bin" } else { "Permanent" };
                let src = r.source.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
                let rid = r.run_id.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
                rows.push_str(&format!(
                    "<tr><td><code>{}</code></td><td>{}</td><td>{}</td><td>{}</td><td style=\"text-align:right\">{:.2} MiB</td><td style=\"text-align:right\">{}</td></tr>\n",
                    rid, r.timestamp, mode, src, r.total_freed as f64 / (1024.0 * 1024.0), r.total_removed
                ));
            }
            if rows.is_empty() {
                rows.push_str("<tr><td colspan=\"6\" style=\"color:#93a89b\">No cleanup runs recorded in Clean Run Ledger yet.</td></tr>");
            }
            let html = format!(
                "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>SLClean Clean Run Ledger Audit Trail</title><style>body{{margin:0;padding:28px;background:#101512;color:#e6efe9;font-family:system-ui,sans-serif}}.wrap{{max-width:980px;margin:0 auto}}.cards{{display:grid;grid-template-columns:repeat(auto-fit,minmax(170px,1fr));gap:12px;margin:16px 0}}.card{{background:#161d19;border:1px solid #2a3730;border-radius:8px;padding:12px}}table{{width:100%;border-collapse:collapse;background:#161d19;border:1px solid #2a3730}}th,td{{padding:8px 10px;border-bottom:1px solid #2a3730;font-size:13px;text-align:left}}th{{color:#93a89b;font-size:11px;text-transform:uppercase}}</style></head><body><div class=\"wrap\"><h1>SLClean Clean Run Ledger Audit Trail</h1><div class=\"cards\"><div class=\"card\"><div>Lifetime Freed</div><strong>{:.2} GiB</strong></div><div class=\"card\"><div>Total Runs</div><strong>{}</strong></div><div class=\"card\"><div>Recycle Bin Runs</div><strong>{}</strong></div><div class=\"card\"><div>Permanent Runs</div><strong>{}</strong></div></div><table><thead><tr><th>Run ID</th><th>Timestamp</th><th>Mode</th><th>Source</th><th style=\"text-align:right\">Freed</th><th style=\"text-align:right\">Items</th></tr></thead><tbody>{}</tbody></table></div></body></html>",
                s.lifetime_freed_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                s.total_runs,
                s.trash_runs,
                s.permanent_runs,
                rows
            );
            tool_outcome(
                id,
                json!({
                    "total_runs": s.total_runs,
                    "lifetime_freed_bytes": s.lifetime_freed_bytes,
                    "html_bytes": html.len(),
                    "html": html,
                }),
                false,
            )
        }
        "slclean_export_settings_html" => {
            let s = settings::load_offline();
            let audit = settings::audit_settings(&s);
            let status_color = if audit.healthy { "#3ecf8e" } else { "#f59e0b" };
            let status_label = if audit.healthy { "HEALTHY" } else { "NEEDS ATTENTION" };
            let mut issue_rows = String::new();
            for p in &audit.missing_extra_roots {
                let esc = p.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
                issue_rows.push_str(&format!("<tr><td>Missing extra root</td><td><code>{}</code></td></tr>\n", esc));
            }
            for p in &audit.missing_excluded_roots {
                let esc = p.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
                issue_rows.push_str(&format!("<tr><td>Missing excluded root</td><td><code>{}</code></td></tr>\n", esc));
            }
            for p in &audit.redundant_exclusions {
                let esc = p.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
                issue_rows.push_str(&format!("<tr><td>Redundant exclusion</td><td><code>{}</code></td></tr>\n", esc));
            }
            if issue_rows.is_empty() {
                issue_rows.push_str("<tr><td colspan=\"2\" style=\"color:#3ecf8e\">All configured extra roots and exclusion rules are valid.</td></tr>");
            }
            let budget_str = audit
                .free_space_budget_gb
                .map(|g| format!("{} GB", g))
                .unwrap_or_else(|| "Not set".into());
            let html = format!(
                "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>SLClean Settings Audit</title><style>body{{margin:0;padding:28px;background:#101512;color:#e6efe9;font-family:system-ui,sans-serif}}.wrap{{max-width:920px;margin:0 auto}}.cards{{display:grid;grid-template-columns:repeat(auto-fit,minmax(170px,1fr));gap:12px;margin:16px 0}}.card{{background:#161d19;border:1px solid #2a3730;border-radius:8px;padding:12px}}table{{width:100%;border-collapse:collapse;background:#161d19;border:1px solid #2a3730}}th,td{{padding:8px 10px;border-bottom:1px solid #2a3730;font-size:13px;text-align:left}}th{{color:#93a89b;font-size:11px;text-transform:uppercase}}</style></head><body><div class=\"wrap\"><h1>SLClean Settings &amp; Path Exclusion Audit</h1><p>Status: <b style=\"color:{}\">{}</b></p><div class=\"cards\"><div class=\"card\"><div>Language</div><strong>{}</strong></div><div class=\"card\"><div>Delete Mode</div><strong>{}</strong></div><div class=\"card\"><div>Simple Mode</div><strong>{}</strong></div><div class=\"card\"><div>Free-Space Budget</div><strong>{}</strong></div><div class=\"card\"><div>Extra / Excluded Roots</div><strong>{} / {}</strong></div></div><table><thead><tr><th>Check</th><th>Path / Detail</th></tr></thead><tbody>{}</tbody></table></div></body></html>",
                status_color,
                status_label,
                audit.effective_language,
                if audit.to_trash_default { "Recycle Bin" } else { "Permanent" },
                if audit.simple_mode { "Enabled" } else { "Standard" },
                budget_str,
                audit.extra_root_count,
                audit.excluded_root_count,
                issue_rows
            );
            tool_outcome(
                id,
                json!({
                    "healthy": audit.healthy,
                    "effective_language": audit.effective_language,
                    "extra_root_count": audit.extra_root_count,
                    "excluded_root_count": audit.excluded_root_count,
                    "html_bytes": html.len(),
                    "html": html,
                }),
                false,
            )
        }
        "slclean_export_history_html" => {
            let h = history::load_default();
            let list = history::list(&h);
            let mut rows = String::new();
            for snap in list.iter().rev().take(30) {
                let drives_str = snap
                    .drives
                    .iter()
                    .map(|d| format!("{} {:.1}/{:.1} GiB", d.mount, d.free as f64 / 1073741824.0, d.total as f64 / 1073741824.0))
                    .collect::<Vec<_>>()
                    .join(" · ");
                let esc_d = drives_str.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
                rows.push_str(&format!(
                    "<tr><td>{}</td><td style=\"text-align:right\">{}</td><td style=\"text-align:right\">{:.2} MiB</td><td>{}</td></tr>\n",
                    snap.at,
                    snap.items,
                    snap.bytes as f64 / (1024.0 * 1024.0),
                    esc_d
                ));
            }
            if rows.is_empty() {
                rows.push_str("<tr><td colspan=\"4\" style=\"color:#93a89b\">No scan snapshots recorded in history.json yet.</td></tr>");
            }
            let html = format!(
                "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>SLClean Storage Snapshot History</title><style>body{{margin:0;padding:28px;background:#101512;color:#e6efe9;font-family:system-ui,sans-serif}}.wrap{{max-width:980px;margin:0 auto}}table{{width:100%;border-collapse:collapse;background:#161d19;border:1px solid #2a3730}}th,td{{padding:8px 10px;border-bottom:1px solid #2a3730;font-size:13px;text-align:left}}th{{color:#93a89b;font-size:11px;text-transform:uppercase}}</style></head><body><div class=\"wrap\"><h1>SLClean Storage Snapshot History</h1><p>Recorded snapshots: <b>{}</b> (showing up to 30 most recent)</p><table><thead><tr><th>Epoch Timestamp</th><th style=\"text-align:right\">Tracked Items</th><th style=\"text-align:right\">Tracked Cache Size</th><th>Drive Free Space</th></tr></thead><tbody>{}</tbody></table></div></body></html>",
                list.len(),
                rows
            );
            tool_outcome(
                id,
                json!({
                    "snapshot_count": list.len(),
                    "html_bytes": html.len(),
                    "html": html,
                }),
                false,
            )
        }
        "slclean_export_drives_badge" => {
            let list = drives::list_drives();
            let total_free: u64 = list.iter().map(|d| d.free).sum();
            let total_cap: u64 = list.iter().map(|d| d.total).sum();
            let min_pct = list
                .iter()
                .filter(|d| d.total > 0)
                .map(|d| (d.free as f64 / d.total as f64) * 100.0)
                .fold(100.0_f64, f64::min);
            let free_gib = total_free as f64 / 1073741824.0;
            let color = if min_pct < 5.0 {
                "red"
            } else if min_pct < 15.0 {
                "yellow"
            } else {
                "brightgreen"
            };
            tool_outcome(
                id,
                json!({
                    "drive_count": list.len(),
                    "total_free_bytes": total_free,
                    "total_capacity_bytes": total_cap,
                    "min_free_percent": (min_pct * 10.0).round() / 10.0,
                    "badge": {
                        "schemaVersion": 1,
                        "label": "slclean drives",
                        "message": format!("{:.1} GiB free ({} vol, min {:.1}%)", free_gib, list.len(), min_pct),
                        "color": color,
                    }
                }),
                false,
            )
        }
        "slclean_export_startup_badge" => {
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
            let color = match rep.grade.as_str() {
                "A" | "B" => "brightgreen",
                "C" => "yellow",
                _ => "red",
            };
            tool_outcome(
                id,
                json!({
                    "grade": rep.grade,
                    "boot_health_score": rep.boot_health_score,
                    "estimated_boot_overhead_ms": rep.estimated_boot_overhead_ms,
                    "badge": {
                        "schemaVersion": 1,
                        "label": "slclean startup",
                        "message": format!("Grade {} ({}ms overhead)", rep.grade, rep.estimated_boot_overhead_ms),
                        "color": color,
                    }
                }),
                false,
            )
        }
        "slclean_export_ledger_badge" => {
            let l = ledger::load_default();
            let s = l.summary();
            let freed_gib = s.lifetime_freed_bytes as f64 / 1073741824.0;
            tool_outcome(
                id,
                json!({
                    "total_runs": s.total_runs,
                    "lifetime_freed_bytes": s.lifetime_freed_bytes,
                    "badge": {
                        "schemaVersion": 1,
                        "label": "slclean reclaimed",
                        "message": format!("{:.2} GiB ({} runs)", freed_gib, s.total_runs),
                        "color": if s.total_runs > 0 { "brightgreen" } else { "blue" },
                    }
                }),
                false,
            )
        }
        "slclean_export_settings_badge" => {
            let s = settings::load_offline();
            let audit = settings::audit_settings(&s);
            let issues = audit.missing_extra_roots.len()
                + audit.missing_excluded_roots.len()
                + audit.redundant_exclusions.len();
            tool_outcome(
                id,
                json!({
                    "healthy": audit.healthy,
                    "issue_count": issues,
                    "badge": {
                        "schemaVersion": 1,
                        "label": "slclean settings",
                        "message": if audit.healthy { "HEALTHY".to_string() } else { format!("{} ISSUE(S)", issues) },
                        "color": if audit.healthy { "brightgreen" } else { "yellow" },
                    }
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
        let arr = tools["result"]["tools"].as_array().unwrap();
        assert_eq!(arr.len(), 38);
        let unique_names: std::collections::HashSet<&str> = arr.iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(unique_names.len(), arr.len(), "MCP tool names must be strictly unique");

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

        let proj_html_call = json!({
            "jsonrpc": "2.0",
            "id": 30,
            "method": "tools/call",
            "params": {
                "name": "slclean_export_projects_html",
                "arguments": { "roots": [dir.to_string_lossy()], "stale_days": 0 }
            }
        });
        let proj_html_resp: Value = serde_json::from_str(&handle_message(&proj_html_call.to_string()).unwrap()).unwrap();
        assert_eq!(proj_html_resp["result"]["isError"], false);
        let proj_html_payload: Value = serde_json::from_str(proj_html_resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(proj_html_payload["html"].as_str().unwrap().contains("<!doctype html>"));

        let led_html_call = json!({
            "jsonrpc": "2.0",
            "id": 32,
            "method": "tools/call",
            "params": { "name": "slclean_export_ledger_html", "arguments": {} }
        });
        let led_html_resp: Value = serde_json::from_str(&handle_message(&led_html_call.to_string()).unwrap()).unwrap();
        assert_eq!(led_html_resp["result"]["isError"], false);
        let led_html_payload: Value = serde_json::from_str(led_html_resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(led_html_payload["html"].as_str().unwrap().contains("Clean Run Ledger Audit Trail"));

        let set_html_call = json!({
            "jsonrpc": "2.0",
            "id": 33,
            "method": "tools/call",
            "params": { "name": "slclean_export_settings_html", "arguments": {} }
        });
        let set_html_resp: Value = serde_json::from_str(&handle_message(&set_html_call.to_string()).unwrap()).unwrap();
        assert_eq!(set_html_resp["result"]["isError"], false);
        let set_html_payload: Value = serde_json::from_str(set_html_resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(set_html_payload["html"].as_str().unwrap().contains("Settings &amp; Path Exclusion Audit"));

        let hist_html_call = json!({
            "jsonrpc": "2.0",
            "id": 34,
            "method": "tools/call",
            "params": { "name": "slclean_export_history_html", "arguments": {} }
        });
        let hist_html_resp: Value = serde_json::from_str(&handle_message(&hist_html_call.to_string()).unwrap()).unwrap();
        assert_eq!(hist_html_resp["result"]["isError"], false);
        let hist_html_payload: Value = serde_json::from_str(hist_html_resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(hist_html_payload["html"].as_str().unwrap().contains("Storage Snapshot History"));

        let drv_badge_call = json!({
            "jsonrpc": "2.0",
            "id": 35,
            "method": "tools/call",
            "params": { "name": "slclean_export_drives_badge", "arguments": {} }
        });
        let drv_badge_resp: Value = serde_json::from_str(&handle_message(&drv_badge_call.to_string()).unwrap()).unwrap();
        assert_eq!(drv_badge_resp["result"]["isError"], false);
        let drv_badge_payload: Value = serde_json::from_str(drv_badge_resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(drv_badge_payload["badge"]["label"], "slclean drives");

        let sb_call = json!({
            "jsonrpc": "2.0",
            "id": 36,
            "method": "tools/call",
            "params": {
                "name": "slclean_export_startup_badge",
                "arguments": {
                    "entries": [{ "name": "SecurityHealth", "command": "C:\\Windows\\System32\\SecurityHealthSystray.exe", "system": true }]
                }
            }
        });
        let sb_resp: Value = serde_json::from_str(&handle_message(&sb_call.to_string()).unwrap()).unwrap();
        assert_eq!(sb_resp["result"]["isError"], false);
        let sb_payload: Value = serde_json::from_str(sb_resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(sb_payload["badge"]["label"], "slclean startup");

        let lb_call = json!({
            "jsonrpc": "2.0",
            "id": 37,
            "method": "tools/call",
            "params": { "name": "slclean_export_ledger_badge", "arguments": {} }
        });
        let lb_resp: Value = serde_json::from_str(&handle_message(&lb_call.to_string()).unwrap()).unwrap();
        assert_eq!(lb_resp["result"]["isError"], false);

        let setb_call = json!({
            "jsonrpc": "2.0",
            "id": 38,
            "method": "tools/call",
            "params": { "name": "slclean_export_settings_badge", "arguments": {} }
        });
        let setb_resp: Value = serde_json::from_str(&handle_message(&setb_call.to_string()).unwrap()).unwrap();
        assert_eq!(setb_resp["result"]["isError"], false);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
