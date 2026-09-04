// Mở ui/index.html trong Chrome headless với một cầu nối window.__TAURI__ giả (dữ liệu
// giống máy này) để kiểm tra bố cục, font, i18n, tìm kiếm, luồng tick → tổng → xác nhận →
// dọn, hộp cài đặt, và chụp ảnh. Không thay thế việc chạy app thật; chỉ bắt lỗi giao diện
// sớm mà không cần build Rust.
import { chromium } from "playwright-core";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { mkdir } from "node:fs/promises";

const here = path.dirname(fileURLToPath(import.meta.url));
const indexUrl = pathToFileURL(path.join(here, "..", "ui", "index.html")).href;
const shots = path.join(here, "shots");
await mkdir(shots, { recursive: true });

const GB = 1024 ** 3;
const NOW = Math.floor(Date.now() / 1000);
const DAY = 86400;
const T = (vi, en) => ({ vi, en });
const app = (id, kind, name, publisher, dir, extra = {}) => ({
  id, kind, name, publisher, version: extra.version ?? "1.0", install_dir: dir, installed: extra.installed ?? NOW - 100 * DAY,
  last_used: extra.last_used ?? 0, usage_known: extra.usage_known ?? true, run_count: extra.run_count ?? 0, running: extra.running ?? false, bytes: extra.bytes ?? 0, files: 0, denied: 0,
  measured: false, dead: extra.dead ?? false, system_component: extra.system_component ?? false,
  folder_exists: extra.folder_exists ?? true, needs_admin: extra.needs_admin ?? false, msi: false,
});
const left = (p, area, extra = {}) => ({
  id: `left:${p.toLowerCase()}`, path: p, area, note: T("Cài đặt và dữ liệu của app không còn cài.", "Settings and data of an app that is no longer installed."),
  modified: extra.modified ?? NOW - 120 * DAY, has_exe: extra.has_exe ?? false, last_used: extra.last_used ?? 0, needs_admin: extra.needs_admin ?? false,
});
const cat = (id, group, label, note, p, safety, bytes, extra = {}) => ({
  id, group, label, note, path: p, paths: extra.paths || [p], safety, keep_root: extra.keep_root ?? true,
  needs_admin: extra.needs_admin ?? false, bytes, files: Math.round(bytes / 40000), denied: extra.denied ?? 0,
});
const FAKE = {
  drives: [
    { mount: "C:", name: "Windows", total: 119.8 * GB, free: 5.5 * GB },
    { mount: "D:", name: "Data", total: 117.2 * GB, free: 2.1 * GB },
  ],
  catalog: [
    cat("wu-download", "system", T("Windows Update — Download", "Windows Update — Download"), T("Gói cập nhật đã cài xong.", "Already-installed update packages."), "C:\\Windows\\SoftwareDistribution\\Download", "safe", 6.51 * GB, { needs_admin: true }),
    cat("codex-sessions", "ai", T("Codex — phiên", "Codex — sessions"), T("Bản ghi các phiên OpenAI Codex CLI.", "OpenAI Codex CLI session transcripts."), "C:\\Users\\Salyyy\\.codex\\sessions", "review", 3.08 * GB),
    cat("modrinth-meta", "game", T("Modrinth — meta (assets, libraries, Java)", "Modrinth — meta (assets, libraries, Java)"), T("Asset/library và JRE do Modrinth tải.", "Assets/libraries and JREs downloaded by Modrinth."), "C:\\Users\\Salyyy\\AppData\\Roaming\\ModrinthApp\\meta", "rebuild", 2.82 * GB),
    cat("claude-projects", "ai", T("Claude Code — transcript phiên", "Claude Code — session transcripts"), T("Bản ghi hội thoại từng dự án (.jsonl).", "Per-project conversation transcripts (.jsonl)."), "C:\\Users\\Salyyy\\.claude\\projects", "review", 1.2 * GB),
    cat("chrome-default", "browser", T("Chrome — Trần Đức Nhân (Default) — cache", "Chrome — Trần Đức Nhân (Default) — cache"), T("Cache web, JS đã biên dịch, GPU và service worker của profile này.", "Web, compiled-JS, GPU and service-worker caches of this profile."), "C:\\Users\\Salyyy\\AppData\\Local\\Google\\Chrome\\User Data\\Default", "safe", 0.9 * GB, { paths: ["C:\\Users\\Salyyy\\AppData\\Local\\Google\\Chrome\\User Data\\Default\\Cache", "C:\\Users\\Salyyy\\AppData\\Local\\Google\\Chrome\\User Data\\Default\\Code Cache", "C:\\Users\\Salyyy\\AppData\\Local\\Google\\Chrome\\User Data\\Default\\GPUCache"] }),
    cat("gradle-caches", "package", T("Gradle — caches", "Gradle — caches"), T("Dependency Maven/Gradle đã tải.", "Downloaded Maven/Gradle dependencies."), "C:\\Users\\Salyyy\\.gradle\\caches", "rebuild", 0.72 * GB),
    cat("cargo-registry-src", "package", T("Cargo — registry src", "Cargo — registry src"), T("Mã nguồn crate đã giải nén.", "Extracted crate sources."), "C:\\Users\\Salyyy\\.cargo\\registry\\src", "rebuild", 0.55 * GB),
    cat("npm-cache", "package", T("npm — cache", "npm — cache"), T("Cache tarball npm.", "npm tarball cache."), "C:\\Users\\Salyyy\\AppData\\Local\\npm-cache", "safe", 0.55 * GB),
    cat("app-discord", "app", T("Discord — cache", "Discord — cache"), T("Cache web/JS/GPU của app (nền Electron).", "The app's web/JS/GPU caches (Electron-based)."), "C:\\Users\\Salyyy\\AppData\\Roaming\\discord", "safe", 0.42 * GB, { paths: ["C:\\Users\\Salyyy\\AppData\\Roaming\\discord\\Cache", "C:\\Users\\Salyyy\\AppData\\Roaming\\discord\\Code Cache"] }),
    cat("app-code-vsix", "editor", T("VS Code — extension đã tải", "VS Code — downloaded extensions"), T("Gói extension đã tải để cài.", "Extension packages downloaded for install."), "C:\\Users\\Salyyy\\AppData\\Roaming\\Code\\CachedExtensionVSIXs", "safe", 0.31 * GB),
    cat("user-temp", "system", T("Temp người dùng", "User Temp"), T("%TEMP%.", "%TEMP%."), "C:\\Users\\Salyyy\\AppData\\Local\\Temp", "safe", 0.9 * GB, { denied: 3 }),
    cat("root-d-tmp", "temp", T("D:\\tmp", "D:\\tmp"), T("Thư mục tạm do bạn hoặc công cụ tạo ở gốc ổ đĩa.", "Temp folder created by you or a tool at the drive root."), "D:\\tmp", "review", 0.74 * GB),
    cat("crash-dumps", "system", T("CrashDumps", "CrashDumps"), T("Dump khi ứng dụng sập.", "Dumps from crashed apps."), "C:\\Users\\Salyyy\\AppData\\Local\\CrashDumps", "safe", 0),
  ],
  artifacts: [
    { path: "D:\\Project\\.WhatChage\\src-tauri\\target", project: "D:\\Project\\.WhatChage\\src-tauri", kind: "target", tool: "Cargo", bytes: 7.0 * GB, files: 30000, modified: Date.now() / 1000 - 3600 },
    { path: "D:\\Project\\VNClientPortal\\node_modules", project: "D:\\Project\\VNClientPortal", kind: "node_modules", tool: "npm / pnpm / yarn", bytes: 0.61 * GB, files: 42000, modified: Date.now() / 1000 - 40 * 86400 },
    { path: "D:\\Project\\VN-BUNDLE\\VNOmniSpawner\\build", project: "D:\\Project\\VN-BUNDLE\\VNOmniSpawner", kind: "build", tool: "Gradle / Maven / CMake", bytes: 0.18 * GB, files: 800, modified: Date.now() / 1000 - 9 * 86400 },
  ],
  settings: { language: null, extra_roots: ["E:\\Work"], excluded_roots: [], to_trash: false },
  roots: { discovered: ["C:\\", "D:\\", "C:\\Users\\Salyyy"], extra: ["E:\\Work"], excluded: [] },
  apps: [
    app("reg:hkcu:0:Blockbench", "desktop", "Blockbench", "JannisX11", "C:\\Users\\Salyyy\\AppData\\Local\\Programs\\Blockbench", { version: "4.12.4", last_used: NOW - 3 * DAY, run_count: 40 }),
    app("reg:hkcu:0:NexoMaker", "desktop", "Nexo Maker", "Nexo", "C:\\Users\\Salyyy\\AppData\\Local\\Programs\\NexoMaker", { dead: true, folder_exists: false }),
    app("reg:hklm:0:AutoTune", "desktop", "Auto-Tune Central 2.0.0", "Antares Audio Technologies", "C:\\Program Files\\Auto-Tune Central", { dead: true, folder_exists: false, needs_admin: true, bytes: 0.45 * GB }),
    app("appx:Microsoft.Paint_11_x64__8wekyb3d8bbwe", "store", "Paint", "Microsoft Corporation", "C:\\Program Files\\WindowsApps\\Microsoft.Paint_11_x64__8wekyb3d8bbwe", { version: "11.2510.31.0", installed: 0, last_used: NOW - DAY, run_count: 12, running: true }),
    app("reg:hklm:1:OldGame", "desktop", "Old Game Launcher", "Some Studio", "C:\\Program Files (x86)\\OldGame", { installed: NOW - 400 * DAY, bytes: 1.2 * GB }),
    // MSI không ghi InstallLocation: không có thư mục để đối chiếu nhật ký → "không rõ", không tính là chưa từng mở.
    app("reg:hklm:0:{686EA7E1-608A-4B99-A50A-448A2B2A7E73}", "desktop", "Node.js", "Node.js Foundation", null, { version: "24.1.0", usage_known: false, folder_exists: false, bytes: 0.09 * GB }),
    // Thành phần nền: không lối mở nào nên không tính là "chưa từng mở", xuống cuối danh sách.
    app("reg:hklm:1:VCRedist", "desktop", "Microsoft Visual C++ 2013 Redistributable (x64)", "Microsoft Corporation", "C:\\Program Files\\Common Files\\VC", { version: "12.0.40664", system_component: true, bytes: 0.02 * GB }),
  ],
  leftovers: [
    left("C:\\Users\\Salyyy\\AppData\\Local\\Spotify", "appdata", { modified: NOW - 90 * DAY }),
    left("C:\\ProgramData\\Privax", "programdata", { needs_admin: true }),
    left("C:\\Program Files (x86)\\iLok License Manager", "programs", { has_exe: true, last_used: NOW - 20 * DAY, needs_admin: true }),
    left("C:\\Users\\Salyyy\\AppData\\Local\\Packages\\windows_ie_ac_001", "packages", { modified: NOW - 300 * DAY }),
  ],
  sizes: {
    "reg:hkcu:0:Blockbench": 0.4 * GB,
    "appx:Microsoft.Paint_11_x64__8wekyb3d8bbwe": 0.02 * GB,
    "reg:hklm:1:OldGame": 1.2 * GB,
    "left:c:\\users\\salyyy\\appdata\\local\\spotify": 0.3 * GB,
    "left:c:\\programdata\\privax": 0.01 * GB,
    "left:c:\\program files (x86)\\ilok license manager": 0.05 * GB,
    "left:c:\\users\\salyyy\\appdata\\local\\packages\\windows_ie_ac_001": 0,
  },
};

const browser = await chromium.launch({ executablePath: "C:/Program Files/Google/Chrome/Application/chrome.exe", headless: true });
const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1 });
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });

await page.addInitScript((fake) => {
  const listeners = new Map();
  const emit = (name, payload) => (listeners.get(name) || []).forEach((cb) => cb({ payload }));
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  let settings = { ...fake.settings };
  window.__FAKE_SETTINGS = () => settings;
  window.__TAURI__ = {
    core: {
      async invoke(cmd, args) {
        switch (cmd) {
          case "list_drives": return fake.drives;
          case "ui_language": return settings.language || "vi";
          case "is_elevated": return false;
          case "get_settings": return { ...settings };
          case "save_settings": settings = { ...args.settings }; return null;
          case "project_roots": return { ...fake.roots, extra: settings.extra_roots, excluded: settings.excluded_roots };
          case "pick_folder": return "F:\\Picked";
          case "scan_catalog":
            emit("catalog-start", fake.catalog.map((c) => ({ ...c, bytes: 0, files: 0, denied: 0 })));
            for (const c of fake.catalog) {
              await sleep(10);
              emit("catalog-progress", { id: c.id, bytes: c.bytes / 2, files: 10 });
              await sleep(10);
              emit("catalog-item", c);
            }
            return fake.catalog;
          case "scan_artifacts":
            for (const a of fake.artifacts) { await sleep(25); emit("artifact-found", a); }
            return fake.artifacts;
          case "scan_apps": {
            emit("apps-list", fake.apps);
            for (const a of fake.apps) {
              if (!a.folder_exists) continue;
              const bytes = fake.sizes[a.id] ?? 0;
              await sleep(10);
              emit("app-size", { id: a.id, bytes: bytes / 2, files: 5, denied: 0, done: false });
              await sleep(10);
              emit("app-size", { id: a.id, bytes, files: 10, denied: a.kind === "store" ? 1 : 0, done: true });
            }
            return fake.apps.map((a) => ({ ...a, bytes: fake.sizes[a.id] ?? a.bytes, measured: a.folder_exists }));
          }
          case "scan_leftovers": {
            emit("leftovers-list", fake.leftovers);
            for (const l of fake.leftovers) {
              const bytes = fake.sizes[l.id] ?? 0;
              await sleep(10);
              emit("leftover-size", { id: l.id, bytes: bytes / 2, files: 3, denied: 0, done: false });
              await sleep(10);
              emit("leftover-size", { id: l.id, bytes, files: 6, denied: 0, done: true });
            }
            return fake.leftovers;
          }
          case "uninstall_app": {
            await sleep(60);
            const a = fake.apps.find((x) => x.id === args.id);
            if (!a) throw "missing";
            return a.kind === "store" ? { gone: true, leftover_dir: null, freed: 0 } : { gone: true, leftover_dir: a.install_dir, freed: 0 };
          }
          case "remove_dead_app": {
            const a = fake.apps.find((x) => x.id === args.id);
            if (!a) throw "missing";
            if (a.needs_admin) throw "needs-admin";
            return { gone: true, leftover_dir: null, freed: args.deleteFolder && a.folder_exists ? 1234 : 0 };
          }
          case "recycle_bin_info": return { items: 3, bytes: 120 * 1024 * 1024 };
          case "empty_recycle_bin": return { items: 3, bytes: 120 * 1024 * 1024 };
          case "cancel_scan": return null;
          case "clean": {
            const out = [];
            for (const it of args.items) {
              await sleep(30);
              const src = fake.catalog.find((c) => c.id === it.id) || fake.artifacts.find((a) => `art:${a.path}` === it.id);
              const r = { id: it.id, freed: src ? src.bytes : (fake.sizes[it.id] ?? 0), removed: 10, skipped: it.id === "user-temp" ? 2 : 0, error: it.id === "user-temp" ? "skipped" : null };
              emit("clean-progress", r);
              out.push(r);
            }
            return out;
          }
          default: throw new Error("unknown cmd " + cmd);
        }
      },
    },
    event: { async listen(name, cb) { listeners.set(name, [...(listeners.get(name) || []), cb]); return () => listeners.set(name, (listeners.get(name) || []).filter((x) => x !== cb)); } },
    opener: { async revealItemInDir() {} },
  };
}, FAKE);

const scanned = () => page.waitForFunction(() => /^(Quét xong|Scanned in)/.test(document.querySelector("#scan-status").textContent), null, { timeout: 15000 });

await page.goto(indexUrl);
await scanned();

const fonts = await page.evaluate(async () => {
  await document.fonts.ready;
  return ["Bricolage Grotesque", "Be Vietnam Pro", "JetBrains Mono"].map((f) => [f, document.fonts.check(`16px "${f}"`)]);
});
const rowsCount = await page.locator(".row").count();
const heroBytes = await page.locator("#hero-bytes").textContent();
const tallyAfterScan = await page.locator("#tally-bytes").textContent();
const measuringLeft = await page.locator(".row--measuring").count();
const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
await page.screenshot({ path: path.join(shots, "slclean-after-scan.png") });

// Tìm kiếm: chỉ hàng khớp "chrome" còn lại.
await page.fill("#search", "chrome");
await page.waitForTimeout(150);
const searchRows = await page.locator(".row").count();
await page.screenshot({ path: path.join(shots, "slclean-search.png") });
await page.fill("#search", "");
await page.waitForTimeout(150);

// Theo nhóm: các nhóm mới (app, game) hiện đúng.
await page.click('[data-view="group"]');
const groupCounts = await page.evaluate(() => Object.fromEntries([...document.querySelectorAll(".group[data-group]")].filter((g) => !g.hidden && !g.classList.contains("group--empty")).map((g) => [g.dataset.group, g.querySelectorAll(".row").length])));
await page.screenshot({ path: path.join(shots, "slclean-groups.png") });
await page.click('[data-view="size"]');

// Đổi sang tiếng Anh: chuỗi tĩnh và nhãn từ backend đổi theo.
await page.click("#btn-lang");
await page.waitForTimeout(100);
const enTexts = await page.evaluate(() => [document.querySelector("#btn-clean").textContent, document.querySelector(".row .row__label").textContent, document.querySelector("#hero-sub").textContent.slice(0, 30)]);
const langSaved = await page.evaluate(() => window.__FAKE_SETTINGS().language);
await page.screenshot({ path: path.join(shots, "slclean-english.png") });

// Hộp cài đặt: mở, thêm thư mục qua "pick", lưu → toast nhắc quét lại.
await page.click("#btn-settings");
await page.waitForSelector("#settings[open]");
await page.click("#set-add-root");
await page.waitForTimeout(80);
const chipCount = await page.locator("#roots-extra .chip:not(.chip--none)").count();
await page.screenshot({ path: path.join(shots, "slclean-settings.png") });
await page.click('#settings button[value="save"]');
await page.waitForTimeout(150);
const savedRoots = await page.evaluate(() => window.__FAKE_SETTINGS().extra_roots);
const toastText = await page.locator("#toast").textContent();

// Về tiếng Việt, chọn "+ tạo lại được" rồi mở xác nhận.
await page.click("#btn-lang");
await page.waitForTimeout(100);
await page.click('[data-pick="rebuild"]');
const tallyRebuild = await page.locator("#tally-bytes").textContent();
const gainD = await page.locator('.drive[data-mount="D:"] .drive__bar').evaluate((el) => getComputedStyle(el).getPropertyValue("--gain").trim());
await page.click("#btn-clean");
await page.waitForSelector("#confirm[open]");
const confirmTitle = await page.locator("#confirm-title").textContent();
await page.screenshot({ path: path.join(shots, "slclean-confirm.png") });

// Xác nhận → tiến trình → xong (user-temp báo "skipped" phải hiện câu dịch).
await page.click("#confirm-ok");
await page.waitForFunction(() => !document.querySelector("#progress-done").hidden, null, { timeout: 10000 });
const doneText = await page.locator("#progress-done").textContent();
const nowText = await page.locator("#progress-now").textContent();
const goneRows = await page.locator(".row--gone").count();
await page.screenshot({ path: path.join(shots, "slclean-done.png") });
await page.click("#progress-close");
const tallyAfterClean = await page.locator("#tally-bytes").textContent();

// Dọn nhanh: tick an toàn + mở xác nhận trong một bấm (sau khi quét lại).
await page.click("#btn-scan");
await scanned();
await page.click("#btn-quick");
await page.waitForSelector("#confirm[open]");
const quickTitle = await page.locator("#confirm-title").textContent();
await page.click('#confirm button[value="cancel"]');

// Tab Ứng dụng: quét lười khi mở lần đầu, sắp "lâu không mở" (mục chết trước), huy hiệu đỏ.
await page.click('[data-tab="apps"]');
await page.waitForFunction(() => document.querySelectorAll(".app").length >= 6 && !document.querySelector(".app--measuring") && !document.querySelector("#apps-scanbar:not([hidden])"), null, { timeout: 15000 });
const appsHero = await page.locator("#apps-hero").textContent();
const appsSub = await page.locator("#apps-sub").textContent();
const appsOrder = await page.evaluate(() => [...document.querySelectorAll(".app .app__name")].map((e) => e.textContent));
const appsBadge = await page.locator("#badge-apps").textContent();
const appsLastUsed = await page.evaluate(() => Object.fromEntries([...document.querySelectorAll(".app")].map((r) => [r.querySelector(".app__name").textContent, r.querySelectorAll(".app__col b")[0].textContent])));
await page.screenshot({ path: path.join(shots, "slclean-apps.png") });
// Bộ lọc "chưa từng mở" không được gom app không rõ (không có thư mục cài) hay thành phần nền.
await page.click('[data-apps-filter="never"]');
const neverRows = await page.evaluate(() => [...document.querySelectorAll(".app .app__name")].map((e) => e.textContent));
await page.click('[data-apps-filter="component"]');
const componentRows = await page.evaluate(() => [...document.querySelectorAll(".app")].map((r) => ({ name: r.querySelector(".app__name").textContent, lastUsed: r.querySelectorAll(".app__col b")[0].textContent, tags: [...r.querySelectorAll(".tag")].map((t) => t.textContent) })));
await page.click('[data-apps-filter="all"]');

// Bộ lọc "mục chết": 2 hàng, mục dưới HKLM bị khoá (cần admin).
await page.click('[data-apps-filter="dead"]');
const deadRows = await page.locator(".app").count();
const deadLocked = await page.locator(".app .app__actions button[disabled]").count();
// Xoá mục chết Nexo Maker qua hộp hỏi → hàng gạch bỏ, toast.
await page.locator('.app[data-app="reg:hkcu:0:NexoMaker"] [data-act="dead"]').click();
await page.waitForSelector("#ask[open]");
const askDeadTitle = await page.locator("#ask-title").textContent();
const askDeadOptionHidden = await page.locator("#ask-option").isHidden();
await page.click("#ask-ok");
await page.waitForFunction(() => document.querySelector('.app[data-app="reg:hkcu:0:NexoMaker"]')?.classList.contains("app--gone"), null, { timeout: 5000 });
const toastDead = await page.locator("#toast").textContent();
const appsBadgeAfter = await page.locator("#badge-apps").textContent();
await page.click('[data-apps-filter="all"]');

// Gỡ Blockbench: hộp hỏi → trình gỡ giả → hộp hỏi thư mục sót → xoá qua lệnh clean.
await page.locator('.app[data-app="reg:hkcu:0:Blockbench"] [data-act="uninstall"]').click();
await page.waitForSelector("#ask[open]");
const askUninstallTitle = await page.locator("#ask-title").textContent();
await page.click("#ask-ok");
await page.waitForFunction(() => document.querySelector("#ask").open && /Blockbench/.test(document.querySelector("#ask-body").textContent), null, { timeout: 5000 });
const askLeftoverTitle = await page.locator("#ask-title").textContent();
const askLeftoverPath = await page.locator("#ask-path").textContent();
await page.click("#ask-ok");
await page.waitForFunction(() => document.querySelector("#toast").classList.contains("is-on") && /2\.86|Freed|Đã xoá thư mục sót|Leftover folder deleted/.test(document.querySelector("#toast").textContent), null, { timeout: 5000 }).catch(() => {});
const toastLeftover = await page.locator("#toast").textContent();
const blockbenchGone = await page.locator('.app[data-app="reg:hkcu:0:Blockbench"]').evaluate((el) => el.classList.contains("app--gone"));
// App đang chạy (Paint) không gỡ được; tìm kiếm lọc theo tên.
const paintUninstallDisabled = await page.locator('.app[data-app^="appx:"] [data-act="uninstall"]').isDisabled();
await page.fill("#apps-search", "old game");
await page.waitForTimeout(150);
const appsSearchRows = await page.locator(".app").count();
await page.fill("#apps-search", "");
await page.waitForTimeout(150);

// Tab Thư mục thừa: quét lười, nhãn vùng/exe/lần mở, mục cần admin bị khoá, tick + dọn qua nút ở rail.
await page.click('[data-tab="leftovers"]');
await page.waitForFunction(() => document.querySelectorAll("#left-list .row").length >= 4 && !document.querySelector("#left-list .row--measuring") && document.querySelector("#left-scanbar").hidden, null, { timeout: 15000 });
const leftHero = await page.locator("#left-hero").textContent();
const leftBadge = await page.locator("#badge-leftovers").textContent();
const leftRows = await page.evaluate(() => [...document.querySelectorAll("#left-list .row")].map((r) => ({ label: r.querySelector(".row__label").textContent, tags: [...r.querySelectorAll(".tag")].map((t) => t.textContent), age: r.querySelector(".row__age")?.textContent, size: r.querySelector(".row__size").textContent, locked: r.querySelector("input").disabled })));
await page.screenshot({ path: path.join(shots, "slclean-leftovers.png") });
await page.locator("#left-list .row").filter({ hasText: "Spotify" }).locator(".row__label").click();
const tallyLeftover = await page.locator("#tally-bytes").textContent();
const cleanBadge = await page.locator("#badge-clean").textContent();
await page.click("#btn-clean");
await page.waitForSelector("#confirm[open]");
const leftoverConfirmTitle = await page.locator("#confirm-title").textContent();
await page.click("#confirm-ok");
await page.waitForFunction(() => !document.querySelector("#progress-done").hidden, null, { timeout: 10000 });
const leftoverDoneText = await page.locator("#progress-done").textContent();
await page.click("#progress-close");
const leftoverGoneRows = await page.locator("#left-list .row--gone").count();
await page.click('[data-left-filter="packages"]');
const leftPackagesRows = await page.locator("#left-list .row:not(.row--gone)").count();
await page.click('[data-left-filter="all"]');
// Quét lại cache không được xoá mất danh sách thư mục thừa.
await page.click('[data-tab="clean"]');
await page.click("#btn-scan");
await scanned();
const leftRowsAfterRescan = await page.evaluate(() => [...items.values()].filter((i) => i.group === "leftover").length);
const tabStored = await page.evaluate(() => localStorage.getItem("slclean-tab"));

console.log(JSON.stringify({ fonts, rowsCount, heroBytes, tallyAfterScan, measuringLeft, searchRows, groupCounts, enTexts, langSaved, chipCount, savedRoots, toastText, tallyRebuild, gainD, confirmTitle, doneText, nowText, goneRows, tallyAfterClean, quickTitle, overflow,
  appsHero, appsSub, appsOrder, appsBadge, appsLastUsed, neverRows, componentRows, deadRows, deadLocked, askDeadTitle, askDeadOptionHidden, toastDead, appsBadgeAfter, askUninstallTitle, askLeftoverTitle, askLeftoverPath, toastLeftover, blockbenchGone, paintUninstallDisabled, appsSearchRows,
  leftHero, leftBadge, leftRows, tallyLeftover, cleanBadge, leftoverConfirmTitle, leftoverDoneText, leftoverGoneRows, leftPackagesRows, leftRowsAfterRescan, tabStored, errors }, null, 2));
await browser.close();
