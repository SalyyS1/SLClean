//! Bảng mục tĩnh của danh mục: mỗi dòng là một vị trí cache/tạm suy ra từ thư mục chuẩn
//! của Windows hoặc biến môi trường của công cụ (CARGO_HOME, GRADLE_USER_HOME, GOPATH…).
//! Nhãn và ghi chú có hai ngôn ngữ (vi, en). Mục không tồn tại trên máy bị bỏ qua.

use crate::catalog::{make_entry, CatalogEntry, Roots, Safety, Text};
use std::path::PathBuf;

struct Spec {
    id: &'static str,
    group: &'static str,
    label: (&'static str, &'static str),
    note: (&'static str, &'static str),
    safety: Safety,
    keep_root: bool,
    /// Một hoặc nhiều thư mục; thư mục đầu tiên tồn tại là đại diện hiển thị.
    paths: fn(&Roots) -> Vec<PathBuf>,
}

/// Thư mục từ biến môi trường của công cụ nếu có, nếu không thì vị trí mặc định.
fn env_or(key: &str, default: PathBuf) -> PathBuf {
    std::env::var_os(key).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or(default)
}

fn cargo_home(r: &Roots) -> PathBuf {
    env_or("CARGO_HOME", r.home.join(".cargo"))
}
fn rustup_home(r: &Roots) -> PathBuf {
    env_or("RUSTUP_HOME", r.home.join(".rustup"))
}
fn gradle_home(r: &Roots) -> PathBuf {
    env_or("GRADLE_USER_HOME", r.home.join(".gradle"))
}
fn gopath(r: &Roots) -> PathBuf {
    env_or("GOPATH", r.home.join("go"))
}
fn minecraft(r: &Roots) -> PathBuf {
    r.roaming.join(".minecraft")
}

use Safety::{Rebuild, Review, Safe};

const SPECS: &[Spec] = &[
    // ---------- Công cụ AI ----------
    Spec { id: "codex-sessions", group: "ai", label: ("Codex — phiên", "Codex — sessions"), note: ("Bản ghi các phiên OpenAI Codex CLI. Xoá thì mất lịch sử resume; cấu hình vẫn giữ.", "OpenAI Codex CLI session transcripts. Deleting loses resume history; settings are kept."), safety: Review, keep_root: true, paths: |r| vec![r.home.join(".codex/sessions")] },
    Spec { id: "codex-log", group: "ai", label: ("Codex — log", "Codex — logs"), note: ("File log của Codex CLI.", "Codex CLI log files."), safety: Safe, keep_root: true, paths: |r| vec![r.home.join(".codex/log")] },
    Spec { id: "codex-archived", group: "ai", label: ("Codex — phiên lưu trữ", "Codex — archived sessions"), note: ("Phiên đã archive.", "Archived sessions."), safety: Review, keep_root: true, paths: |r| vec![r.home.join(".codex/archived_sessions")] },
    Spec { id: "orca-sessions", group: "ai", label: ("Orca (Codex desktop) — phiên", "Orca (Codex desktop) — sessions"), note: ("Bản ghi phiên Codex chạy qua Orca. Xoá thì mất lịch sử resume trong Orca.", "Codex sessions run through Orca. Deleting loses resume history in Orca."), safety: Review, keep_root: true, paths: |r| vec![r.roaming.join("orca/codex-runtime-home/home/sessions")] },
    Spec { id: "claude-projects", group: "ai", label: ("Claude Code — transcript phiên", "Claude Code — session transcripts"), note: ("Bản ghi hội thoại từng dự án (.jsonl). Xoá sẽ mất lịch sử /resume; memory và cấu hình vẫn giữ.", "Per-project conversation transcripts (.jsonl). Deleting loses /resume history; memory and settings are kept."), safety: Review, keep_root: true, paths: |r| vec![r.home.join(".claude/projects")] },
    Spec { id: "claude-file-history", group: "ai", label: ("Claude Code — file history", "Claude Code — file history"), note: ("Bản sao file trước mỗi lần sửa, dùng cho /rewind. Xoá thì không quay lui được các phiên cũ.", "Snapshots of files before each edit, used by /rewind. Deleting disables rewinding old sessions."), safety: Review, keep_root: true, paths: |r| vec![r.home.join(".claude/file-history")] },
    Spec { id: "claude-shell-snapshots", group: "ai", label: ("Claude Code — shell snapshots", "Claude Code — shell snapshots"), note: ("Ảnh chụp môi trường shell, tự tạo lại.", "Shell environment snapshots, recreated automatically."), safety: Safe, keep_root: true, paths: |r| vec![r.home.join(".claude/shell-snapshots")] },
    Spec { id: "claude-debug", group: "ai", label: ("Claude Code — debug log", "Claude Code — debug logs"), note: ("Log gỡ lỗi.", "Debug logs."), safety: Safe, keep_root: true, paths: |r| vec![r.home.join(".claude/debug")] },
    Spec { id: "claude-todos", group: "ai", label: ("Claude Code — todo cũ", "Claude Code — old todos"), note: ("Danh sách việc của các phiên đã qua.", "Task lists from past sessions."), safety: Safe, keep_root: true, paths: |r| vec![r.home.join(".claude/todos")] },
    Spec { id: "claude-paste-cache", group: "ai", label: ("Claude Code — paste cache", "Claude Code — paste cache"), note: ("Nội dung dán vào prompt đã lưu tạm.", "Cached pasted prompt content."), safety: Safe, keep_root: true, paths: |r| vec![r.home.join(".claude/paste-cache")] },
    Spec { id: "claude-temp", group: "ai", label: ("Claude Code — thư mục tạm", "Claude Code — temp folder"), note: ("File tạm của các tool (skill đóng gói, output tác vụ nền).", "Tool temp files (bundled skills, background task output)."), safety: Safe, keep_root: true, paths: |r| vec![r.temp.join("claude")] },
    Spec { id: "gemini-tmp", group: "ai", label: ("Gemini CLI — thư mục tạm", "Gemini CLI — temp folder"), note: ("File tạm của Gemini CLI.", "Gemini CLI temp files."), safety: Safe, keep_root: true, paths: |r| vec![r.home.join(".gemini/tmp")] },
    Spec { id: "huggingface", group: "ai", label: ("Hugging Face — hub cache", "Hugging Face — hub cache"), note: ("Model/dataset đã tải; tải lại khi cần.", "Downloaded models/datasets; re-downloaded on demand."), safety: Rebuild, keep_root: true, paths: |r| vec![env_or("HF_HOME", r.home.join(".cache/huggingface"))] },
    Spec { id: "torch", group: "ai", label: ("PyTorch — cache", "PyTorch — cache"), note: ("Checkpoint đã tải qua torch.hub.", "Checkpoints downloaded via torch.hub."), safety: Rebuild, keep_root: true, paths: |r| vec![r.home.join(".cache/torch")] },
    Spec { id: "ollama", group: "ai", label: ("Ollama — models", "Ollama — models"), note: ("Model chạy local. Xoá thì phải pull lại.", "Local models. Deleting requires pulling them again."), safety: Review, keep_root: true, paths: |r| vec![env_or("OLLAMA_MODELS", r.home.join(".ollama/models"))] },
    Spec { id: "lmstudio-models", group: "ai", label: ("LM Studio — models", "LM Studio — models"), note: ("Model đã tải trong LM Studio.", "Models downloaded in LM Studio."), safety: Review, keep_root: true, paths: |r| vec![r.home.join(".lmstudio/models"), r.home.join(".cache/lm-studio/models")] },
    Spec { id: "chrome-devtools-mcp", group: "ai", label: ("chrome-devtools-mcp — profile", "chrome-devtools-mcp — profile"), note: ("Profile trình duyệt riêng của MCP.", "The MCP server's private browser profile."), safety: Safe, keep_root: false, paths: |r| vec![r.local.join("chrome-devtools-mcp")] },
    Spec { id: "ms-playwright", group: "ai", label: ("Playwright — trình duyệt", "Playwright — browsers"), note: ("Chromium/Firefox/WebKit do Playwright tải. Chạy lại `playwright install` khi cần.", "Chromium/Firefox/WebKit downloaded by Playwright. Run `playwright install` again when needed."), safety: Rebuild, keep_root: false, paths: |r| vec![env_or("PLAYWRIGHT_BROWSERS_PATH", r.local.join("ms-playwright"))] },
    Spec { id: "puppeteer", group: "ai", label: ("Puppeteer — trình duyệt", "Puppeteer — browsers"), note: ("Chromium do Puppeteer tải.", "Chromium downloaded by Puppeteer."), safety: Rebuild, keep_root: false, paths: |r| vec![r.home.join(".cache/puppeteer"), r.local.join("puppeteer")] },

    // ---------- Kho gói ----------
    Spec { id: "npm-cache", group: "package", label: ("npm — cache", "npm — cache"), note: ("Cache tarball npm. `npm install` tải lại.", "npm tarball cache. `npm install` re-downloads."), safety: Safe, keep_root: true, paths: |r| vec![env_or("npm_config_cache", r.local.join("npm-cache")), r.roaming.join("npm-cache")] },
    Spec { id: "pnpm-store", group: "package", label: ("pnpm — store", "pnpm — store"), note: ("Store nội dung pnpm. Dự án đang dùng hard-link sẽ phải cài lại.", "pnpm content store. Projects using hard links will need reinstalling."), safety: Rebuild, keep_root: true, paths: |r| vec![r.local.join("pnpm/store")] },
    Spec { id: "pnpm-cache", group: "package", label: ("pnpm — metadata cache", "pnpm — metadata cache"), note: ("Cache metadata registry của pnpm.", "pnpm registry metadata cache."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("pnpm-cache")] },
    Spec { id: "yarn-cache", group: "package", label: ("Yarn — cache", "Yarn — cache"), note: ("Cache gói Yarn (classic và Berry).", "Yarn package cache (classic and Berry)."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("Yarn/Cache"), r.local.join("Yarn/Berry/cache")] },
    Spec { id: "bun-cache", group: "package", label: ("Bun — cache", "Bun — cache"), note: ("Cache gói Bun.", "Bun package cache."), safety: Safe, keep_root: true, paths: |r| vec![env_or("BUN_INSTALL", r.home.join(".bun")).join("install/cache")] },
    Spec { id: "deno-cache", group: "package", label: ("Deno — cache", "Deno — cache"), note: ("Module đã tải của Deno.", "Deno's downloaded modules."), safety: Rebuild, keep_root: true, paths: |r| vec![env_or("DENO_DIR", r.local.join("deno"))] },
    Spec { id: "pip-cache", group: "package", label: ("pip — cache", "pip — cache"), note: ("Wheel đã tải.", "Downloaded wheels."), safety: Safe, keep_root: true, paths: |r| vec![env_or("PIP_CACHE_DIR", r.local.join("pip/cache"))] },
    Spec { id: "uv-cache", group: "package", label: ("uv — cache", "uv — cache"), note: ("Cache của uv (Python).", "uv (Python) cache."), safety: Safe, keep_root: true, paths: |r| vec![env_or("UV_CACHE_DIR", r.local.join("uv/cache"))] },
    Spec { id: "poetry-cache", group: "package", label: ("Poetry — cache", "Poetry — cache"), note: ("Cache gói của Poetry.", "Poetry package cache."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("pypoetry/Cache")] },
    Spec { id: "cargo-registry-cache", group: "package", label: ("Cargo — registry cache (.crate)", "Cargo — registry cache (.crate)"), note: ("File .crate đã tải; giải nén lại từ src nếu cần.", "Downloaded .crate files; re-extracted from src when needed."), safety: Safe, keep_root: true, paths: |r| vec![cargo_home(r).join("registry/cache")] },
    Spec { id: "cargo-registry-src", group: "package", label: ("Cargo — registry src", "Cargo — registry src"), note: ("Mã nguồn crate đã giải nén. Build sau sẽ tải lại.", "Extracted crate sources. The next build re-downloads."), safety: Rebuild, keep_root: true, paths: |r| vec![cargo_home(r).join("registry/src")] },
    Spec { id: "cargo-git", group: "package", label: ("Cargo — git checkouts", "Cargo — git checkouts"), note: ("Dependency lấy từ git.", "Git-sourced dependencies."), safety: Rebuild, keep_root: true, paths: |r| vec![cargo_home(r).join("git")] },
    Spec { id: "rustup-downloads", group: "package", label: ("rustup — downloads/tmp", "rustup — downloads/tmp"), note: ("File tải dở và tạm của rustup.", "rustup partial downloads and temp files."), safety: Safe, keep_root: true, paths: |r| vec![rustup_home(r).join("downloads"), rustup_home(r).join("tmp")] },
    Spec { id: "go-mod", group: "package", label: ("Go — module cache", "Go — module cache"), note: ("Module đã tải. `go build` tải lại.", "Downloaded modules. `go build` re-downloads."), safety: Rebuild, keep_root: true, paths: |r| vec![env_or("GOMODCACHE", gopath(r).join("pkg/mod"))] },
    Spec { id: "go-build", group: "package", label: ("Go — build cache", "Go — build cache"), note: ("Kết quả biên dịch trung gian.", "Intermediate compilation output."), safety: Safe, keep_root: true, paths: |r| vec![env_or("GOCACHE", r.local.join("go-build"))] },
    Spec { id: "gradle-caches", group: "package", label: ("Gradle — caches", "Gradle — caches"), note: ("Dependency Maven/Gradle đã tải, mọi dự án (kể cả plugin Minecraft) dùng chung. Build sau tải lại.", "Downloaded Maven/Gradle dependencies shared by all projects (including Minecraft plugins). The next build re-downloads."), safety: Rebuild, keep_root: true, paths: |r| vec![gradle_home(r).join("caches")] },
    Spec { id: "gradle-daemon", group: "package", label: ("Gradle — daemon logs", "Gradle — daemon logs"), note: ("Log daemon Gradle, tăng theo thời gian.", "Gradle daemon logs, grow over time."), safety: Safe, keep_root: true, paths: |r| vec![gradle_home(r).join("daemon")] },
    Spec { id: "gradle-wrapper", group: "package", label: ("Gradle — wrapper dists", "Gradle — wrapper dists"), note: ("Các bản Gradle đã tải qua wrapper.", "Gradle distributions downloaded by the wrapper."), safety: Rebuild, keep_root: true, paths: |r| vec![gradle_home(r).join("wrapper/dists")] },
    Spec { id: "gradle-jdks", group: "package", label: ("Gradle — JDK tự tải", "Gradle — auto-provisioned JDKs"), note: ("JDK do toolchain Gradle tải về. Build sau tải lại.", "JDKs downloaded by Gradle toolchains. The next build re-downloads."), safety: Rebuild, keep_root: true, paths: |r| vec![gradle_home(r).join("jdks")] },
    Spec { id: "maven", group: "package", label: ("Maven — repository", "Maven — repository"), note: ("Kho .m2. Build sau tải lại.", "The .m2 repository. The next build re-downloads."), safety: Rebuild, keep_root: true, paths: |r| vec![r.home.join(".m2/repository")] },
    Spec { id: "nuget", group: "package", label: ("NuGet — packages", "NuGet — packages"), note: ("Gói .NET đã tải.", "Downloaded .NET packages."), safety: Rebuild, keep_root: true, paths: |r| vec![env_or("NUGET_PACKAGES", r.home.join(".nuget/packages"))] },
    Spec { id: "nuget-http", group: "package", label: ("NuGet — HTTP cache", "NuGet — HTTP cache"), note: ("Cache phản hồi từ nuget.org.", "Cached nuget.org responses."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("NuGet/v3-cache")] },
    Spec { id: "composer", group: "package", label: ("Composer — cache", "Composer — cache"), note: ("Cache gói PHP.", "PHP package cache."), safety: Safe, keep_root: true, paths: |r| vec![env_or("COMPOSER_CACHE_DIR", r.local.join("Composer"))] },
    Spec { id: "pub-cache", group: "package", label: ("Dart/Flutter — pub cache", "Dart/Flutter — pub cache"), note: ("Gói pub đã tải. `flutter pub get` tải lại.", "Downloaded pub packages. `flutter pub get` re-downloads."), safety: Rebuild, keep_root: true, paths: |r| vec![env_or("PUB_CACHE", r.local.join("Pub/Cache"))] },
    Spec { id: "scoop-cache", group: "package", label: ("Scoop — cache", "Scoop — cache"), note: ("Bộ cài đã tải của Scoop.", "Installers downloaded by Scoop."), safety: Safe, keep_root: true, paths: |r| vec![r.home.join("scoop/cache")] },
    Spec { id: "android-cache", group: "package", label: ("Android — cache", "Android — cache"), note: ("Cache của Android SDK/AVD.", "Android SDK/AVD cache."), safety: Safe, keep_root: true, paths: |r| vec![r.home.join(".android/cache")] },
    Spec { id: "electron-cache", group: "package", label: ("Electron — download cache", "Electron — download cache"), note: ("Bản Electron đã tải khi `npm install`.", "Electron binaries downloaded during `npm install`."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("electron/Cache"), r.local.join("electron-builder/Cache")] },
    Spec { id: "node-gyp", group: "package", label: ("node-gyp — headers", "node-gyp — headers"), note: ("Header Node đã tải để build native module.", "Node headers downloaded to build native modules."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("node-gyp/Cache")] },
    Spec { id: "cypress-cache", group: "package", label: ("Cypress — binary cache", "Cypress — binary cache"), note: ("Bản Cypress đã tải.", "Downloaded Cypress binaries."), safety: Rebuild, keep_root: true, paths: |r| vec![r.local.join("Cypress/Cache")] },
    Spec { id: "unity-cache", group: "package", label: ("Unity — cache", "Unity — cache"), note: ("Cache gói và asset của Unity Hub/Editor.", "Unity Hub/Editor package and asset cache."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("Unity/cache")] },
    Spec { id: "unreal-ddc", group: "package", label: ("Unreal — DerivedDataCache", "Unreal — DerivedDataCache"), note: ("Asset đã biên dịch của Unreal; mở lại dự án sẽ build lại.", "Unreal's compiled asset cache; reopening a project rebuilds it."), safety: Rebuild, keep_root: true, paths: |r| vec![r.local.join("UnrealEngine/Common/DerivedDataCache")] },

    // ---------- Ứng dụng ----------
    Spec { id: "spotify-storage", group: "app", label: ("Spotify — Storage", "Spotify — Storage"), note: ("Cache nhạc đã phát.", "Cache of streamed music."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("Spotify/Storage")] },
    Spec { id: "spotify-data", group: "app", label: ("Spotify — nhạc offline", "Spotify — offline music"), note: ("Nhạc đã tải để nghe offline. Xoá thì phải tải lại.", "Music downloaded for offline listening. Deleting requires re-downloading."), safety: Review, keep_root: true, paths: |r| vec![r.local.join("Spotify/Data")] },
    Spec { id: "telegram-cache", group: "app", label: ("Telegram — cache media", "Telegram — media cache"), note: ("Ảnh, video, file đã xem trong Telegram Desktop. Tải lại khi mở.", "Photos, videos and files viewed in Telegram Desktop. Re-downloaded when opened."), safety: Safe, keep_root: true, paths: |r| vec![r.roaming.join("Telegram Desktop/tdata/user_data/cache"), r.roaming.join("Telegram Desktop/tdata/user_data/media_cache")] },
    Spec { id: "zoom-logs", group: "app", label: ("Zoom — logs", "Zoom — logs"), note: ("Log của Zoom.", "Zoom logs."), safety: Safe, keep_root: true, paths: |r| vec![r.roaming.join("Zoom/logs")] },
    Spec { id: "sideloadly-cache", group: "app", label: ("Sideloadly — cache .ipa", "Sideloadly — .ipa cache"), note: ("Bản sao .ipa đã sideload; file gốc vẫn còn.", "Copies of sideloaded .ipa files; the originals remain."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("cache/sideloadly")] },
    Spec { id: "squirrel-temp", group: "app", label: ("Squirrel — temp cài đặt", "Squirrel — installer temp"), note: ("File tạm của bộ cài Squirrel (Discord, Slack…).", "Squirrel installer temp files (Discord, Slack…)."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("SquirrelTemp")] },
    Spec { id: "onedrive-logs", group: "app", label: ("OneDrive — logs", "OneDrive — logs"), note: ("Log đồng bộ OneDrive.", "OneDrive sync logs."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("Microsoft/OneDrive/logs")] },
    Spec { id: "adobe-media-cache", group: "app", label: ("Adobe — Media Cache", "Adobe — Media Cache"), note: ("Cache render của Premiere/After Effects; mở lại dự án sẽ tạo lại.", "Premiere/After Effects render cache; reopening a project recreates it."), safety: Rebuild, keep_root: true, paths: |r| vec![r.local.join("Adobe/Common/Media Cache Files"), r.roaming.join("Adobe/Common/Media Cache")] },
    Spec { id: "rdp-cache", group: "app", label: ("Remote Desktop — bitmap cache", "Remote Desktop — bitmap cache"), note: ("Cache màn hình của Remote Desktop.", "Remote Desktop screen cache."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("Microsoft/Terminal Server Client/Cache")] },

    // ---------- Game ----------
    Spec { id: "steam-htmlcache", group: "game", label: ("Steam — web cache", "Steam — web cache"), note: ("Cache trình duyệt nhúng của Steam.", "Steam's embedded browser cache."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("Steam/htmlcache")] },
    Spec { id: "epic-webcache", group: "game", label: ("Epic Games — web cache/logs", "Epic Games — web cache/logs"), note: ("Cache và log của Epic Games Launcher.", "Epic Games Launcher cache and logs."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("EpicGamesLauncher/Saved/webcache"), r.local.join("EpicGamesLauncher/Saved/Logs")] },
    Spec { id: "minecraft-webcache", group: "game", label: ("Minecraft Launcher — web cache/logs", "Minecraft Launcher — web cache/logs"), note: ("Cache web, log và crash report của launcher/game.", "Launcher web cache, game logs and crash reports."), safety: Safe, keep_root: true, paths: |r| vec![minecraft(r).join("webcache2"), minecraft(r).join("logs"), minecraft(r).join("crash-reports")] },
    Spec { id: "minecraft-assets", group: "game", label: ("Minecraft — assets & libraries", "Minecraft — assets & libraries"), note: ("Asset/library vanilla do launcher tải. Mở game sẽ tải lại; thế giới không bị đụng.", "Vanilla assets/libraries downloaded by the launcher. Re-downloaded on launch; worlds are untouched."), safety: Rebuild, keep_root: true, paths: |r| vec![minecraft(r).join("assets"), minecraft(r).join("libraries")] },
    Spec { id: "minecraft-versions", group: "game", label: ("Minecraft — versions", "Minecraft — versions"), note: ("Jar từng phiên bản, kể cả Forge/Fabric đã cài. Xoá thì phải cài lại mod loader.", "Per-version jars, including installed Forge/Fabric. Deleting requires reinstalling mod loaders."), safety: Review, keep_root: true, paths: |r| vec![minecraft(r).join("versions")] },
    Spec { id: "modrinth-backups", group: "game", label: ("Modrinth — Backups", "Modrinth — Backups"), note: ("Bản sao lưu profile do Modrinth App tạo. Xem lại trước khi xoá.", "Profile backups created by Modrinth App. Review before deleting."), safety: Review, keep_root: true, paths: |r| vec![r.local.join("Modrinth/Backups")] },
    Spec { id: "modrinth-meta", group: "game", label: ("Modrinth — meta (assets, libraries, Java)", "Modrinth — meta (assets, libraries, Java)"), note: ("Asset/library và JRE do Modrinth tải. Mở lại instance sẽ tải lại; thế giới và mod không bị đụng.", "Assets/libraries and JREs downloaded by Modrinth. Reopening an instance re-downloads; worlds and mods are untouched."), safety: Rebuild, keep_root: true, paths: |r| vec![r.roaming.join("ModrinthApp/meta")] },
    Spec { id: "prism-meta", group: "game", label: ("Prism Launcher — assets, libraries, meta", "Prism Launcher — assets, libraries, meta"), note: ("Dữ liệu tải về dùng chung; instance và thế giới không bị đụng.", "Shared downloaded data; instances and worlds are untouched."), safety: Rebuild, keep_root: true, paths: |r| vec![r.roaming.join("PrismLauncher/assets"), r.roaming.join("PrismLauncher/libraries"), r.roaming.join("PrismLauncher/meta")] },

    // ---------- Hệ thống Windows ----------
    Spec { id: "user-temp", group: "system", label: ("Temp người dùng", "User Temp"), note: ("%TEMP%. File đang mở sẽ được bỏ qua.", "%TEMP%. Files in use are skipped."), safety: Safe, keep_root: true, paths: |r| vec![r.temp.clone()] },
    Spec { id: "windows-temp", group: "system", label: ("Windows\\Temp", "Windows\\Temp"), note: ("Temp hệ thống.", "System temp."), safety: Safe, keep_root: true, paths: |r| vec![r.system_root.join("Temp")] },
    Spec { id: "wu-download", group: "system", label: ("Windows Update — Download", "Windows Update — Download"), note: ("Gói cập nhật đã cài xong. Windows tự tải lại nếu thiếu.", "Already-installed update packages. Windows re-downloads if needed."), safety: Safe, keep_root: true, paths: |r| vec![r.system_root.join("SoftwareDistribution/Download")] },
    Spec { id: "delivery-opt", group: "system", label: ("Delivery Optimization", "Delivery Optimization"), note: ("Cache chia sẻ cập nhật P2P.", "Peer-to-peer update sharing cache."), safety: Safe, keep_root: true, paths: |r| vec![r.system_root.join("SoftwareDistribution/DeliveryOptimization"), r.system_root.join("ServiceProfiles/NetworkService/AppData/Local/Microsoft/Windows/DeliveryOptimization/Cache")] },
    Spec { id: "cbs-logs", group: "system", label: ("Windows — CBS logs", "Windows — CBS logs"), note: ("Log servicing của Windows, có thể phình tới vài GB.", "Windows servicing logs, can grow to several GB."), safety: Safe, keep_root: true, paths: |r| vec![r.system_root.join("Logs/CBS")] },
    Spec { id: "wer", group: "system", label: ("Windows Error Reporting", "Windows Error Reporting"), note: ("Báo cáo lỗi đã hoặc chờ gửi.", "Sent or queued error reports."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("Microsoft/Windows/WER"), r.program_data.join("Microsoft/Windows/WER")] },
    Spec { id: "crash-dumps", group: "system", label: ("CrashDumps", "CrashDumps"), note: ("Dump khi ứng dụng sập; chỉ hữu ích khi đang gỡ lỗi.", "Dumps from crashed apps; only useful while debugging."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("CrashDumps")] },
    Spec { id: "d3d-cache", group: "system", label: ("D3DSCache", "D3DSCache"), note: ("Cache shader Direct3D.", "Direct3D shader cache."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("D3DSCache")] },
    Spec { id: "nv-cache", group: "system", label: ("NVIDIA — shader cache", "NVIDIA — shader cache"), note: ("Cache shader driver NVIDIA; game sẽ biên dịch lại.", "NVIDIA driver shader cache; games recompile it."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("NVIDIA/DXCache"), r.local.join("NVIDIA/GLCache"), r.local.join("NVIDIA Corporation/NV_Cache")] },
    Spec { id: "nv-downloader", group: "system", label: ("NVIDIA — driver đã tải", "NVIDIA — downloaded drivers"), note: ("Bộ cài driver GeForce Experience/NVIDIA App đã tải.", "Driver installers downloaded by GeForce Experience/NVIDIA App."), safety: Safe, keep_root: true, paths: |r| vec![r.program_data.join("NVIDIA Corporation/Downloader")] },
    Spec { id: "amd-cache", group: "system", label: ("AMD — shader cache", "AMD — shader cache"), note: ("Cache shader driver AMD.", "AMD driver shader cache."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("AMD/DxCache"), r.local.join("AMD/GLCache"), r.local.join("AMD/VkCache")] },
    Spec { id: "intel-cache", group: "system", label: ("Intel — shader cache", "Intel — shader cache"), note: ("Cache shader driver Intel.", "Intel driver shader cache."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("Intel/ShaderCache")] },
    Spec { id: "inetcache", group: "system", label: ("Windows — INetCache", "Windows — INetCache"), note: ("Cache WinINet dùng bởi các app cũ và Office.", "WinINet cache used by legacy apps and Office."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("Microsoft/Windows/INetCache")] },
    Spec { id: "explorer-thumbs", group: "system", label: ("Explorer — thumbnail/icon cache", "Explorer — thumbnail/icon cache"), note: ("Ảnh thu nhỏ và icon đã tạo; Explorer tạo lại. File đang mở bị bỏ qua.", "Generated thumbnails and icons; Explorer rebuilds them. Files in use are skipped."), safety: Safe, keep_root: true, paths: |r| vec![r.local.join("Microsoft/Windows/Explorer")] },
    Spec { id: "package-cache", group: "system", label: ("Package Cache (Visual Studio)", "Package Cache (Visual Studio)"), note: ("Bộ cài Visual Studio/.NET giữ lại để sửa chữa hoặc gỡ. Xoá thì repair/uninstall có thể đòi tải lại.", "Installers kept by Visual Studio/.NET for repair or uninstall. Deleting may make repair/uninstall re-download."), safety: Review, keep_root: true, paths: |r| vec![r.local.join("Package Cache"), r.program_data.join("Package Cache")] },
];

pub fn entries(r: &Roots) -> Vec<CatalogEntry> {
    SPECS
        .iter()
        .filter_map(|s| {
            let paths: Vec<PathBuf> = (s.paths)(r).into_iter().filter(|p| p.is_dir()).collect();
            let first = paths.first()?.clone();
            make_entry(r, s.id, s.group, Text::new(s.label.0, s.label.1), Text::new(s.note.0, s.note.1), first, paths, s.safety, s.keep_root)
        })
        .collect()
}
