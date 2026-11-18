//! OASIS SARIF v2.1.0 report generator and CI disk-budget gate for headless SLClean runs.
//!
//! Enables self-hosted GitHub Actions, GitLab CI, and Windows/Linux build runners to audit
//! reclaimable developer caches, build artifacts, and duplicate AI model weight blobs and upload
//! findings directly to GitHub Code Scanning (`github/codeql-action/upload-sarif`) or fail builds
//! with exit code `3` when `--max-reclaimable-gb` is exceeded.

use crate::{artifacts, blob_dupes, catalog};
use serde::Serialize;
use std::path::Path;

pub const GIB_BYTES: f64 = 1024.0 * 1024.0 * 1024.0;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BudgetGate {
    pub max_reclaimable_gb: f64,
    pub max_reclaimable_bytes: u64,
    pub actual_reclaimable_bytes: u64,
    pub actual_reclaimable_gb: f64,
    pub exceeded: bool,
}

impl BudgetGate {
    pub fn evaluate(max_gb: f64, actual_bytes: u64) -> Self {
        let max_bytes = (max_gb.max(0.0) * GIB_BYTES).round() as u64;
        let actual_gb = (actual_bytes as f64) / GIB_BYTES;
        let exceeded = actual_bytes > max_bytes;
        Self {
            max_reclaimable_gb: max_gb,
            max_reclaimable_bytes: max_bytes,
            actual_reclaimable_bytes: actual_bytes,
            actual_reclaimable_gb: (actual_gb * 1000.0).round() / 1000.0,
            exceeded,
        }
    }
}

#[derive(Serialize)]
pub struct SarifLog {
    #[serde(rename = "$schema")]
    pub schema: &'static str,
    pub version: &'static str,
    pub runs: Vec<SarifRun>,
}

#[derive(Serialize)]
pub struct SarifRun {
    pub tool: SarifTool,
    pub results: Vec<SarifResult>,
}

#[derive(Serialize)]
pub struct SarifTool {
    pub driver: SarifDriver,
}

#[derive(Serialize)]
pub struct SarifDriver {
    pub name: &'static str,
    pub version: &'static str,
    #[serde(rename = "informationUri")]
    pub information_uri: &'static str,
    pub rules: Vec<SarifRule>,
}

#[derive(Serialize)]
pub struct SarifRule {
    pub id: &'static str,
    pub name: &'static str,
    #[serde(rename = "shortDescription")]
    pub short_description: SarifMessage,
    #[serde(rename = "helpUri")]
    pub help_uri: &'static str,
}

#[derive(Serialize)]
pub struct SarifMessage {
    pub text: String,
}

#[derive(Serialize)]
pub struct SarifResult {
    #[serde(rename = "ruleId")]
    pub rule_id: &'static str,
    pub level: &'static str,
    pub message: SarifMessage,
    pub locations: Vec<SarifLocation>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "relatedLocations")]
    pub related_locations: Option<Vec<SarifRelatedLocation>>,
}

#[derive(Serialize)]
pub struct SarifLocation {
    #[serde(rename = "physicalLocation")]
    pub physical_location: SarifPhysicalLocation,
}

#[derive(Serialize)]
pub struct SarifRelatedLocation {
    pub id: usize,
    pub message: SarifMessage,
    #[serde(rename = "physicalLocation")]
    pub physical_location: SarifPhysicalLocation,
}

#[derive(Serialize)]
pub struct SarifPhysicalLocation {
    #[serde(rename = "artifactLocation")]
    pub artifact_location: SarifArtifactLocation,
}

#[derive(Serialize)]
pub struct SarifArtifactLocation {
    pub uri: String,
}

fn path_to_uri(p: &Path) -> String {
    let norm = p.to_string_lossy().replace('\\', "/");
    if norm.starts_with('/') {
        format!("file://{norm}")
    } else {
        format!("file:///{norm}")
    }
}

fn format_bytes(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    if (bytes as f64) >= GIB_BYTES {
        format!("{:.2} GiB", (bytes as f64) / GIB_BYTES)
    } else if (bytes as f64) >= MIB {
        format!("{:.1} MiB", (bytes as f64) / MIB)
    } else {
        format!("{bytes} B")
    }
}

fn rules_catalog() -> Vec<SarifRule> {
    vec![
        SarifRule {
            id: "SLC001",
            name: "SafeCacheReclaimable",
            short_description: SarifMessage {
                text: "Safe temporary or package cache directory reclaimable without side effects.".into(),
            },
            help_uri: "https://github.com/SalyyS1/SLClean#headless-cli--ci-integration",
        },
        SarifRule {
            id: "SLC002",
            name: "RebuildableCacheBloat",
            short_description: SarifMessage {
                text: "Rebuildable developer, compiler, or AI model cache occupying disk space.".into(),
            },
            help_uri: "https://github.com/SalyyS1/SLClean#headless-cli--ci-integration",
        },
        SarifRule {
            id: "SLC003",
            name: "ProjectBuildArtifact",
            short_description: SarifMessage {
                text: "Project build output or dependency directory (node_modules, target, .venv).".into(),
            },
            help_uri: "https://github.com/SalyyS1/SLClean#headless-cli--ci-integration",
        },
        SarifRule {
            id: "SLC004",
            name: "DuplicateModelOrArtifactBlob",
            short_description: SarifMessage {
                text: "Byte-identical AI model weight or build artifact blob duplicated across caches.".into(),
            },
            help_uri: "https://github.com/SalyyS1/SLClean#duplicate-ai-blob-analyzer",
        },
        SarifRule {
            id: "SLC005",
            name: "DiskBudgetExceeded",
            short_description: SarifMessage {
                text: "Reclaimable cache or duplicate blob footprint exceeded --max-reclaimable-gb budget.".into(),
            },
            help_uri: "https://github.com/SalyyS1/SLClean#ci-disk-budget-gate",
        },
    ]
}

fn budget_result(gate: &BudgetGate) -> Option<SarifResult> {
    if !gate.exceeded {
        return None;
    }
    Some(SarifResult {
        rule_id: "SLC005",
        level: "error",
        message: SarifMessage {
            text: format!(
                "Reclaimable disk footprint {} ({:.3} GiB) exceeds configured budget of {:.3} GiB ({} bytes).",
                format_bytes(gate.actual_reclaimable_bytes),
                gate.actual_reclaimable_gb,
                gate.max_reclaimable_gb,
                gate.max_reclaimable_bytes
            ),
        },
        locations: vec![SarifLocation {
            physical_location: SarifPhysicalLocation {
                artifact_location: SarifArtifactLocation { uri: "file:///".into() },
            },
        }],
        related_locations: None,
    })
}

pub fn build_scan_sarif(
    catalog_items: &[crate::CatalogItem],
    found_artifacts: &[artifacts::Artifact],
    budget: Option<&BudgetGate>,
) -> SarifLog {
    let mut results = Vec::new();

    if let Some(gate) = budget {
        if let Some(r) = budget_result(gate) {
            results.push(r);
        }
    }

    for item in catalog_items {
        if item.bytes == 0 {
            continue;
        }
        let (rule_id, level) = match item.entry.safety {
            catalog::Safety::Safe => ("SLC001", "note"),
            catalog::Safety::Rebuild => ("SLC002", "note"),
            catalog::Safety::Review => continue,
        };
        results.push(SarifResult {
            rule_id,
            level,
            message: SarifMessage {
                text: format!(
                    "{} ({}) occupies {} across {} files.",
                    item.entry.label.en,
                    item.entry.id,
                    format_bytes(item.bytes),
                    item.files
                ),
            },
            locations: vec![SarifLocation {
                physical_location: SarifPhysicalLocation {
                    artifact_location: SarifArtifactLocation {
                        uri: path_to_uri(&item.entry.path),
                    },
                },
            }],
            related_locations: None,
        });
    }

    for art in found_artifacts {
        if art.bytes == 0 {
            continue;
        }
        results.push(SarifResult {
            rule_id: "SLC003",
            level: "note",
            message: SarifMessage {
                text: format!(
                    "Build artifact '{}' ({}) occupies {}.",
                    art.kind,
                    art.tool,
                    format_bytes(art.bytes)
                ),
            },
            locations: vec![SarifLocation {
                physical_location: SarifPhysicalLocation {
                    artifact_location: SarifArtifactLocation {
                        uri: path_to_uri(&art.path),
                    },
                },
            }],
            related_locations: None,
        });
    }

    SarifLog {
        schema: "https://json.schemastore.org/sarif-2.1.0.json",
        version: "2.1.0",
        runs: vec![SarifRun {
            tool: SarifTool {
                driver: SarifDriver {
                    name: "SLClean",
                    version: env!("CARGO_PKG_VERSION"),
                    information_uri: "https://github.com/SalyyS1/SLClean",
                    rules: rules_catalog(),
                },
            },
            results,
        }],
    }
}

pub fn build_dupes_sarif(report: &blob_dupes::BlobDupReport, budget: Option<&BudgetGate>) -> SarifLog {
    let mut results = Vec::new();

    if let Some(gate) = budget {
        if let Some(r) = budget_result(gate) {
            results.push(r);
        }
    }

    for group in &report.groups {
        let Some(primary) = group.locations.first() else { continue };
        let related: Vec<SarifRelatedLocation> = group
            .locations
            .iter()
            .skip(1)
            .enumerate()
            .map(|(idx, loc)| SarifRelatedLocation {
                id: idx + 1,
                message: SarifMessage {
                    text: format!(
                        "Duplicate {} blob{}",
                        loc.ecosystem,
                        loc.model_hint
                            .as_deref()
                            .map(|h| format!(" ({h})"))
                            .unwrap_or_default()
                    ),
                },
                physical_location: SarifPhysicalLocation {
                    artifact_location: SarifArtifactLocation {
                        uri: path_to_uri(&loc.path),
                    },
                },
            })
            .collect();

        results.push(SarifResult {
            rule_id: "SLC004",
            level: if group.cross_ecosystem { "warning" } else { "note" },
            message: SarifMessage {
                text: format!(
                    "SHA-256 {} ({} each, {} wasted across {} copies in [{}]). {}",
                    &group.sha256[..16],
                    format_bytes(group.bytes),
                    format_bytes(group.wasted_bytes),
                    group.locations.len(),
                    group.ecosystems.join(", "),
                    group.remediation
                ),
            },
            locations: vec![SarifLocation {
                physical_location: SarifPhysicalLocation {
                    artifact_location: SarifArtifactLocation {
                        uri: path_to_uri(&primary.path),
                    },
                },
            }],
            related_locations: if related.is_empty() { None } else { Some(related) },
        });
    }

    SarifLog {
        schema: "https://json.schemastore.org/sarif-2.1.0.json",
        version: "2.1.0",
        runs: vec![SarifRun {
            tool: SarifTool {
                driver: SarifDriver {
                    name: "SLClean",
                    version: env!("CARGO_PKG_VERSION"),
                    information_uri: "https://github.com/SalyyS1/SLClean",
                    rules: rules_catalog(),
                },
            },
            results,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn budget_gate_evaluates_exact_gib_threshold() {
        let half_gib = (0.5 * GIB_BYTES) as u64;
        let ok_gate = BudgetGate::evaluate(1.0, half_gib);
        assert!(!ok_gate.exceeded);

        let over_gate = BudgetGate::evaluate(0.25, half_gib);
        assert!(over_gate.exceeded);
        assert_eq!(over_gate.actual_reclaimable_gb, 0.5);
    }

    #[test]
    fn dupes_sarif_emits_slc004_and_slc005_on_budget_breach() {
        let rep = blob_dupes::BlobDupReport {
            roots_scanned: vec![PathBuf::from("/tmp/models")],
            min_blob_bytes: 1024,
            trust_cas_names: true,
            total_wasted_bytes: 2 * 1024 * 1024 * 1024,
            cross_ecosystem_wasted_bytes: 2 * 1024 * 1024 * 1024,
            groups: vec![blob_dupes::BlobDupGroup {
                sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into(),
                bytes: 2 * 1024 * 1024 * 1024,
                wasted_bytes: 2 * 1024 * 1024 * 1024,
                cross_ecosystem: true,
                ecosystems: vec!["huggingface".into(), "ollama".into()],
                locations: vec![
                    blob_dupes::BlobLocation {
                        path: PathBuf::from("/tmp/hf/blobs/0123"),
                        ecosystem: "huggingface".into(),
                        model_hint: Some("Qwen/Qwen2.5-3B".into()),
                        modified: 100,
                    },
                    blob_dupes::BlobLocation {
                        path: PathBuf::from("/tmp/ollama/blobs/sha256-0123"),
                        ecosystem: "ollama".into(),
                        model_hint: None,
                        modified: 200,
                    },
                ],
                remediation: "Hardlink or prune duplicate copy.".into(),
            }],
            ..blob_dupes::BlobDupReport::default()
        };

        let gate = BudgetGate::evaluate(1.0, rep.total_wasted_bytes);
        assert!(gate.exceeded);
        let sarif = build_dupes_sarif(&rep, Some(&gate));
        assert_eq!(sarif.version, "2.1.0");
        assert_eq!(sarif.runs[0].results.len(), 2);
        assert_eq!(sarif.runs[0].results[0].rule_id, "SLC005");
        assert_eq!(sarif.runs[0].results[0].level, "error");
        assert_eq!(sarif.runs[0].results[1].rule_id, "SLC004");
        assert_eq!(sarif.runs[0].results[1].level, "warning");
        assert_eq!(
            sarif.runs[0].results[1].related_locations.as_ref().unwrap().len(),
            1
        );
    }
}
