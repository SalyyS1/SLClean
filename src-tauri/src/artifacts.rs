//! Tìm artifact build có thể tạo lại (node_modules, target, build, ...) dưới các gốc trong
//! `ScanPlan`. Không bao giờ đi sâu vào artifact đã tìm thấy hoặc .git, nên quét nhanh kể cả
//! khi đi cả ổ đĩa. Mỗi artifact được đối chiếu với "dấu hiệu dự án" bên cạnh nó (Cargo.toml
//! cạnh `target`, build.gradle cạnh `build`, ...) để không nhầm thư mục thường có tên trùng.

use crate::project_roots::{is_blocked_root_child, is_under, ScanPlan};
use jwalk::{Parallelism, WalkDir};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

#[derive(Clone, Debug, Serialize)]
pub struct Artifact {
    pub path: PathBuf,
    /// Thư mục dự án chứa artifact (cha trực tiếp).
    pub project: PathBuf,
    pub kind: String,
    pub tool: String,
    pub bytes: u64,
    pub files: u64,
    /// Lần sửa cuối của artifact (giây epoch) để UI gợi ý "lâu không đụng".
    pub modified: u64,
}

/// Tên thư mục luôn là artifact, không cần dấu hiệu bên cạnh.
const ALWAYS: &[(&str, &str)] = &[
    ("__pycache__", "Python"),
    (".gradle", "Gradle (project)"),
    (".kotlin", "Kotlin"),
    (".next", "Next.js"),
    (".nuxt", "Nuxt"),
    (".turbo", "Turborepo"),
    (".parcel-cache", "Parcel"),
    (".svelte-kit", "SvelteKit"),
    (".angular", "Angular"),
    (".vite", "Vite"),
    (".mypy_cache", "mypy"),
    (".pytest_cache", "pytest"),
    (".ruff_cache", "ruff"),
    (".dart_tool", "Dart"),
];

const SKIP: &[&str] = &[".git", ".hg", ".svn", "$RECYCLE.BIN", "System Volume Information"];

fn has_any(dir: &Path, names: &[&str]) -> bool {
    names.iter().any(|n| dir.join(n).exists())
}

fn has_ext(dir: &Path, ext: &str) -> bool {
    std::fs::read_dir(dir)
        .map(|it| it.flatten().any(|e| e.path().extension().map(|x| x == ext).unwrap_or(false)))
        .unwrap_or(false)
}

/// Trả về tên công cụ nếu `name` là artifact hợp lệ trong thư mục cha `parent`.
fn classify(name: &str, parent: &Path, path: &Path) -> Option<&'static str> {
    if name == "node_modules" {
        // Bản cài Node (nvm/fnm/volta, msys) cũng có node_modules chứa npm: không phải artifact.
        let node_install = !parent.join("package.json").is_file() && path.join("npm/bin/npm-cli.js").is_file();
        return if node_install { None } else { Some("npm / pnpm / yarn") };
    }
    if let Some((_, tool)) = ALWAYS.iter().find(|(n, _)| *n == name) {
        return Some(tool);
    }
    match name {
        "target" if has_any(parent, &["Cargo.toml"]) => Some("Cargo"),
        "target" if has_any(parent, &["pom.xml"]) => Some("Maven"),
        "build" if has_any(parent, &["build.gradle", "build.gradle.kts", "settings.gradle", "settings.gradle.kts", "gradlew", "pom.xml", "CMakeLists.txt", "package.json", "meson.build"]) => Some("Gradle / Maven / CMake"),
        "out" if has_any(parent, &["package.json", "tsconfig.json", "build.gradle", "build.gradle.kts", ".idea"]) || has_ext(parent, "iml") => Some("IDE / TS output"),
        "dist" if has_any(parent, &["package.json", "pyproject.toml", "setup.py", "vite.config.ts", "vite.config.js"]) => Some("bundler output"),
        "bin" | "obj" if has_ext(parent, "csproj") || has_ext(parent, "sln") || has_ext(parent, "fsproj") => Some(".NET"),
        ".venv" | "venv" | "env" if path.join("pyvenv.cfg").is_file() => Some("Python venv"),
        "Library" | "Temp" if parent.join("Assets").is_dir() && parent.join("ProjectSettings").is_dir() => Some("Unity"),
        "run" if has_any(parent, &["build.gradle.kts", "build.gradle"]) && path.join("eula.txt").exists() => Some("Minecraft test server (run-paper)"),
        n if n.starts_with("cmake-build-") && has_any(parent, &["CMakeLists.txt"]) => Some("CMake (CLion)"),
        _ => None,
    }
}

fn is_artifact_name(name: &str) -> bool {
    name == "node_modules"
        || ALWAYS.iter().any(|(n, _)| *n == name)
        || matches!(name, "target" | "build" | "out" | "dist" | "bin" | "obj" | ".venv" | "venv" | "env" | "run" | "Library" | "Temp")
        || name.starts_with("cmake-build-")
}

/// Thư mục chương trình đã cài theo bố cục Electron (`…\resources\app\…`, `*.asar.unpacked`)
/// hoặc toolchain (msys/mingw). VS Code và Cursor ship sẵn `out`, `dist`, `node_modules` ở đó:
/// `npm install` không tạo lại được và xoá là hỏng editor, nên không bao giờ coi là artifact.
fn inside_installed_app(path: &Path) -> bool {
    let parts: Vec<String> = path.components().map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase()).collect();
    parts.windows(2).any(|w| w[0] == "resources" && w[1] == "app")
        || parts.iter().any(|p| p.ends_with(".asar.unpacked"))
        || parts.iter().any(|p| matches!(p.as_str(), "msys64" | "msys2" | "cygwin64" | "mingw64" | "mingw32" | "ucrt64" | "clang64" | "program files" | "program files (x86)" | "windowsapps"))
}

/// Artifact đã nhận diện nhưng chưa đo dung lượng.
struct Candidate {
    path: PathBuf,
    parent: PathBuf,
    name: String,
    tool: &'static str,
}

/// Hai pha: (1) đi qua cây, cắt nhánh ngay tại artifact/.git/thư mục bị chặn nên rất nhanh;
/// (2) đo dung lượng các artifact song song trên pool riêng, phát từng cái khi đo xong.
pub fn find_artifacts(plan: &ScanPlan, cancel: &Arc<AtomicBool>, on_found: impl Fn(&Artifact) + Sync) -> Vec<Artifact> {
    let mut candidates = Vec::new();
    let excluded = Arc::new(plan.excluded.clone());
    for root in &plan.roots {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let root_owned = root.clone();
        let at_drive_root = root.parent().is_none();
        let excluded = excluded.clone();
        // Pool rayon riêng cho pha tìm: không có busy-timeout như pool toàn cục nên
        // không bao giờ bỏ dở giữa chừng khi máy đang bận.
        let walk = WalkDir::new(root)
            .follow_links(false)
            .skip_hidden(false)
            .max_depth(9)
            .parallelism(Parallelism::RayonNewPool(4))
            .process_read_dir(move |_, parent, _, children| {
                let is_root = parent == root_owned;
                children.retain(|c| match c {
                    Ok(child) if child.file_type().is_dir() && !child.file_type().is_symlink() => {
                        let name = child.file_name().to_string_lossy();
                        if is_root && is_blocked_root_child(&name, at_drive_root) {
                            return false;
                        }
                        !excluded.iter().any(|ex| is_under(&child.path(), ex))
                    }
                    _ => true,
                });
                for child in children.iter_mut().flatten() {
                    if !child.file_type().is_dir() || child.file_type().is_symlink() {
                        continue;
                    }
                    let name = child.file_name().to_string_lossy().to_string();
                    if SKIP.contains(&name.as_str()) || (is_artifact_name(&name) && classify(&name, parent, &child.path()).is_some()) {
                        child.read_children = None;
                    }
                }
            });
        for entry in walk.into_iter().flatten() {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            if !entry.file_type().is_dir() || entry.file_type().is_symlink() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if !is_artifact_name(&name) {
                continue;
            }
            let path = entry.path();
            if inside_installed_app(&path) {
                continue;
            }
            let Some(parent) = path.parent().map(Path::to_path_buf) else { continue };
            let Some(tool) = classify(&name, &parent, &path) else { continue };
            candidates.push(Candidate { path, parent, name, tool });
        }
    }

    let mut found = crate::parallel::map(candidates, crate::parallel::default_workers(), |c| {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let stats = crate::sizer::dir_stats(&c.path, cancel);
        let modified = std::fs::metadata(&c.path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let art = Artifact { path: c.path, project: c.parent, kind: c.name, tool: c.tool.to_string(), bytes: stats.bytes, files: stats.files, modified };
        on_found(&art);
        Some(art)
    })
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    found.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn program_files_of_installed_editors_are_not_artifacts() {
        for p in [
            r"D:\Work\cursor\resources\app\node_modules",
            r"D:\Work\Microsoft VS Code\1b6a188127\resources\app\out",
            r"D:\Work\Microsoft VS Code\1b6a188127\resources\app\extensions\copilot\dist",
            r"D:\Work\Microsoft VS Code\1b6a188127\resources\app\node_modules.asar.unpacked\node-pty\build",
            r"C:\msys64\mingw64\lib\node_modules",
            r"C:\Program Files\nodejs\node_modules",
        ] {
            assert!(inside_installed_app(Path::new(p)), "{p}");
        }
        for p in [r"D:\Project\VNClientPortal\node_modules", r"D:\Project\slclean\src-tauri\target", r"D:\Project\x\src\main\resources\build"] {
            assert!(!inside_installed_app(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn scan_finds_artifacts_and_respects_exclusions() {
        let root = std::env::temp_dir().join(format!("slclean-art-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        // Dự án npm hợp lệ, dự án cargo hợp lệ, thư mục "target" không có Cargo.toml, và một
        // dự án bị loại trừ.
        fs::create_dir_all(root.join("web/node_modules/pkg")).unwrap();
        fs::write(root.join("web/package.json"), "{}").unwrap();
        fs::write(root.join("web/node_modules/pkg/i.js"), "1").unwrap();
        fs::create_dir_all(root.join("rs/target/debug")).unwrap();
        fs::write(root.join("rs/Cargo.toml"), "[package]").unwrap();
        fs::write(root.join("rs/target/debug/a.bin"), vec![0u8; 500]).unwrap();
        fs::create_dir_all(root.join("plain/target")).unwrap();
        fs::create_dir_all(root.join("keep/node_modules")).unwrap();
        fs::write(root.join("keep/package.json"), "{}").unwrap();
        let plan = ScanPlan { roots: vec![root.clone()], excluded: vec![root.join("keep")] };
        let found = find_artifacts(&plan, &Arc::new(AtomicBool::new(false)), |_| {});
        let kinds: Vec<(String, String)> = found.iter().map(|a| (a.project.file_name().unwrap().to_string_lossy().to_string(), a.kind.clone())).collect();
        assert!(kinds.contains(&("web".into(), "node_modules".into())), "{kinds:?}");
        assert!(kinds.contains(&("rs".into(), "target".into())), "{kinds:?}");
        assert!(!kinds.iter().any(|(p, _)| p == "plain"), "{kinds:?}");
        assert!(!kinds.iter().any(|(p, _)| p == "keep"), "{kinds:?}");
        assert_eq!(found.iter().find(|a| a.kind == "target").unwrap().bytes, 500);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn node_install_is_not_an_artifact() {
        let root = std::env::temp_dir().join(format!("slclean-nodeinst-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("v22/node_modules/npm/bin")).unwrap();
        fs::write(root.join("v22/node_modules/npm/bin/npm-cli.js"), "").unwrap();
        assert!(classify("node_modules", &root.join("v22"), &root.join("v22/node_modules")).is_none());
        fs::write(root.join("v22/package.json"), "{}").unwrap();
        assert!(classify("node_modules", &root.join("v22"), &root.join("v22/node_modules")).is_some());
        let _ = fs::remove_dir_all(&root);
    }
}
