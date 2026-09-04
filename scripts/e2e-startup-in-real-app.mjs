// Trên app thật (mở qua launch-app-with-devtools-port.ps1) với fixture từ
// make-throwaway-startup-fixture.ps1: mở tab Khởi động, tắt mục giả qua công tắc, kiểm registry
// bằng PowerShell (cờ StartupApproved byte 0 = 3), bật lại (= 2), rồi xoá mục hỏng và xác nhận
// giá trị Run đã biến mất. Không tin số app hiện; mọi khẳng định đối chiếu với registry thật.
// Dùng: node scripts/e2e-startup-in-real-app.mjs [port]
import { chromium } from "playwright-core";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const port = Number(process.argv[2] || 9224);
const here = path.dirname(fileURLToPath(import.meta.url));
const LIVE = "ZzStartupFixture";
const DEAD = "ZzStartupFixtureBroken";
const RUN = "HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const APPR = "HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";
const ps = (cmd) => execFileSync("powershell", ["-NoProfile", "-Command", cmd], { encoding: "utf8" }).trim();
/** Byte 0 của cờ approved, hoặc "none" khi Windows chưa từng tắt mục đó. */
const flagOf = (name) => ps(`$v=(Get-ItemProperty '${APPR}' -EA SilentlyContinue).'${name}'; if ($v) { $v[0] } else { 'none' }`);
const runValue = (name) => ps(`$v=(Get-ItemProperty '${RUN}' -EA SilentlyContinue).'${name}'; if ($v) { $v } else { 'none' }`);

let browser;
for (let i = 0; i < 30; i++) {
  try { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`); break; } catch { await new Promise((r) => setTimeout(r, 1000)); }
}
if (!browser) { console.error("cannot connect to DevTools port", port); process.exit(1); }
let page;
for (let i = 0; i < 30 && !page; i++) {
  page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes("index.html") || p.url().startsWith("http://tauri.localhost") || p.url().startsWith("tauri://"));
  if (!page) await new Promise((r) => setTimeout(r, 500));
}
if (!page) { console.error("no app page"); process.exit(1); }
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });

const out = { before: { liveFlag: flagOf(LIVE), deadFlag: flagOf(DEAD), liveValue: runValue(LIVE) !== "none", deadValue: runValue(DEAD) !== "none" } };
const flip = (id) => page.locator(`.boot[data-start="${id}"] .switch__track`).click();

await page.waitForFunction(() => /^(Quét xong|Scanned in|Quét lỗi|Scan failed)/.test(document.querySelector("#scan-status").textContent), null, { timeout: 15 * 60 * 1000 });

const t0 = Date.now();
await page.click('[data-tab="startup"]');
await page.waitForFunction(() => document.querySelectorAll(".boot").length > 0 && document.querySelector("#start-scanbar").hidden, null, { timeout: 5 * 60 * 1000 });
out.scanSeconds = ((Date.now() - t0) / 1000).toFixed(1);
out.summary = await page.evaluate(() => {
  const all = [...startups.values()];
  const by = (s) => all.filter((e) => e.source === s).length;
  return {
    total: all.length, hero: document.querySelector("#start-hero").textContent, sub: document.querySelector("#start-sub").textContent,
    badge: document.querySelector("#badge-startup").textContent,
    bySource: { run: by("run"), runmachine: by("runmachine"), folder: by("folder"), foldermachine: by("foldermachine"), task: by("task") },
    enabled: all.filter((e) => e.enabled).length, broken: all.filter((e) => e.broken).length, system: all.filter((e) => e.system).length,
    withLastUsed: all.filter((e) => e.last_used > 0).length,
    // Đây là câu tab này sinh ra để trả lời: đang bật, có bản ghi mở, mà đã hơn 30 ngày.
    staleNames: all.filter((e) => e.enabled && !e.broken && e.usage_known && e.last_used > 0 && Date.now() / 1000 - e.last_used > 30 * 86400).map((e) => `${e.name} ${fmtAge(e.last_used)}`),
    brokenNames: all.filter((e) => e.broken).map((e) => e.name),
    microsoftLeak: all.filter((e) => e.id.startsWith("task:\\Microsoft\\")).length,
  };
});
await page.screenshot({ path: path.join(here, "shots", "slclean-real-startup.png") });

// Fixture phải có mặt, mục hỏng phải được nhận ra là hỏng.
out.fixtureRows = await page.evaluate((ids) => ids.map((id) => {
  const e = startups.get(id);
  return e ? { name: e.name, enabled: e.enabled, broken: e.broken, needs_admin: e.needs_admin, source: e.source } : null;
}), [`run:hkcu:0:${LIVE}`, `run:hkcu:0:${DEAD}`]);

// ----- Tắt mục giả, kiểm registry -----
await flip(`run:hkcu:0:${LIVE}`);
await page.waitForTimeout(600);
out.afterOff = { toast: await page.locator("#toast").textContent(), flag: flagOf(LIVE), stillInRun: runValue(LIVE) !== "none", uiOn: await page.locator(`.boot[data-start="run:hkcu:0:${LIVE}"] input`).isChecked() };

// ----- Bật lại, kiểm registry -----
await flip(`run:hkcu:0:${LIVE}`);
await page.waitForTimeout(600);
out.afterOn = { toast: await page.locator("#toast").textContent(), flag: flagOf(LIVE), uiOn: await page.locator(`.boot[data-start="run:hkcu:0:${LIVE}"] input`).isChecked() };

// ----- Xoá mục hỏng qua hộp hỏi -----
await page.fill("#start-search", "zzstartupfixturebroken");
await page.waitForTimeout(200);
await page.locator(`.boot[data-start="run:hkcu:0:${DEAD}"] [data-act="remove"]`).click();
await page.waitForSelector("#ask[open]");
out.askRemove = { title: await page.locator("#ask-title").textContent(), path: await page.locator("#ask-path").textContent() };
await page.click("#ask-ok");
await page.waitForTimeout(800);
out.afterRemove = { toast: await page.locator("#toast").textContent(), runValue: runValue(DEAD), flag: flagOf(DEAD), rowGone: (await page.locator(`.boot[data-start="run:hkcu:0:${DEAD}"]`).count()) === 0 };
await page.fill("#start-search", "");

// Mục còn lại không được suy suyển: chỉ mục fixture bị đụng tới.
out.liveStillThere = runValue(LIVE) !== "none";
out.errors = errors;
console.log(JSON.stringify(out, null, 2));
await browser.close();
