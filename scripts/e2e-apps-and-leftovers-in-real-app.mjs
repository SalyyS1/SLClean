// Trên app thật (mở qua launch-app-with-devtools-port.ps1) với fixture từ
// make-throwaway-app-fixtures.ps1: mở tab Ứng dụng, xoá mục chết (kèm thư mục sót), gỡ app giả
// qua trình gỡ của nó rồi dọn thư mục sót; mở tab Thư mục thừa, tick thư mục mồ côi và dọn qua
// nút ở thanh trái. Sau mỗi bước kiểm tra registry và đĩa bằng PowerShell, không tin số app hiện.
// Dùng: node scripts/e2e-apps-and-leftovers-in-real-app.mjs [port]
import { chromium } from "playwright-core";
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const port = Number(process.argv[2] || 9224);
const here = path.dirname(fileURLToPath(import.meta.url));
const LOCAL = process.env.LOCALAPPDATA;
const DEAD_DIR = path.join(LOCAL, "ZzFixtureDead");
const LIVE_DIR = path.join(LOCAL, "ZzFixtureLive");
const ORPHAN_DIR = path.join(LOCAL, "ZzOrphanFixtureApp");
const ps = (cmd) => execFileSync("powershell", ["-NoProfile", "-Command", cmd], { encoding: "utf8" }).trim();
const keyExists = (name) => ps(`Test-Path 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\${name}'`) === "True";

let browser;
for (let i = 0; i < 30; i++) {
  try { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`); break; } catch { await new Promise((r) => setTimeout(r, 1000)); }
}
if (!browser) { console.error("cannot connect to DevTools port", port); process.exit(1); }
// Webview có thể chưa tạo trang xong ngay sau khi cổng DevTools mở: thử lại vài giây.
let page;
for (let i = 0; i < 30 && !page; i++) {
  page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes("index.html") || p.url().startsWith("http://tauri.localhost") || p.url().startsWith("tauri://"));
  if (!page) await new Promise((r) => setTimeout(r, 500));
}
if (!page) { console.error("no app page; pages:", browser.contexts().flatMap((c) => c.pages()).map((p) => p.url())); process.exit(1); }
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
const out = { before: { deadKey: keyExists("ZzFixtureDead"), liveKey: keyExists("ZzFixtureLive"), deadDir: existsSync(DEAD_DIR), liveDir: existsSync(LIVE_DIR), orphanDir: existsSync(ORPHAN_DIR) } };

await page.waitForFunction(() => /^(Quét xong|Scanned in|Quét lỗi|Scan failed)/.test(document.querySelector("#scan-status").textContent), null, { timeout: 15 * 60 * 1000 });

// ----- Tab Ứng dụng -----
const t0 = Date.now();
await page.click('[data-tab="apps"]');
await page.waitForFunction(() => document.querySelectorAll(".app").length > 0 && document.querySelector("#apps-scanbar").hidden && !document.querySelector(".app--measuring"), null, { timeout: 10 * 60 * 1000 });
out.appsScanSeconds = ((Date.now() - t0) / 1000).toFixed(1);
out.apps = await page.evaluate(() => {
  const all = [...apps.values()];
  const top = all.filter((a) => !a.dead).sort((a, b) => b.bytes - a.bytes).slice(0, 8).map((a) => `${a.name} ${fmtBytes(a.bytes)}${a.measured ? "" : " (est)"}`);
  const stale = all.filter((a) => !a.dead && a.last_used > 0).sort((a, b) => a.last_used - b.last_used).slice(0, 5).map((a) => `${a.name} ${fmtAge(a.last_used)}`);
  return {
    total: all.length, desktop: all.filter((a) => a.kind === "desktop").length, store: all.filter((a) => a.kind === "store").length,
    dead: all.filter((a) => a.dead).length, never: all.filter((a) => !a.dead && a.usage_known && !a.last_used && !a.running).length,
    unknown: all.filter((a) => !a.dead && !a.usage_known).length, withLastUsed: all.filter((a) => a.last_used > 0).length, running: all.filter((a) => a.running).length,
    unknownNames: all.filter((a) => !a.dead && !a.usage_known).map((a) => a.name).slice(0, 40),
    neverNames: all.filter((a) => !a.dead && a.usage_known && !a.last_used && !a.running && a.kind === "desktop").map((a) => a.name).slice(0, 60),
    measured: all.filter((a) => a.measured).length, hero: document.querySelector("#apps-hero").textContent, sub: document.querySelector("#apps-sub").textContent,
    badge: document.querySelector("#badge-apps").textContent, firstRows: [...document.querySelectorAll(".app .app__name")].slice(0, 6).map((e) => e.textContent),
    top, stale, unresolvedNames: all.filter((a) => /ms-resource|^@\{/.test(a.name)).length,
  };
});
await page.screenshot({ path: path.join(here, "shots", "slclean-real-apps.png") });

// Xoá mục chết của fixture, kèm thư mục còn sót.
await page.fill("#apps-search", "zz fixture dead");
await page.waitForTimeout(200);
const deadRow = page.locator(".app").filter({ hasText: "Zz Fixture Dead App" });
out.deadRowTags = await deadRow.locator(".tag").allTextContents();
await deadRow.locator('[data-act="dead"]').click();
await page.waitForSelector("#ask[open]");
out.askDead = { title: await page.locator("#ask-title").textContent(), option: await page.locator("#ask-option-text").textContent(), checked: await page.locator("#ask-option-check").isChecked() };
await page.click("#ask-ok");
await page.waitForFunction(() => [...document.querySelectorAll(".app--gone")].some((r) => r.textContent.includes("Zz Fixture Dead App")), null, { timeout: 15000 });
out.afterDead = { toast: await page.locator("#toast").textContent(), key: keyExists("ZzFixtureDead"), dir: existsSync(DEAD_DIR) };

// Gỡ app giả qua trình gỡ của nó (reg.exe tự xoá khoá) rồi dọn thư mục sót.
await page.fill("#apps-search", "zz fixture live");
await page.waitForTimeout(200);
const liveRow = page.locator(".app").filter({ hasText: "Zz Fixture Live App" });
out.liveRowInstalled = await liveRow.locator(".app__col b").nth(1).textContent();
await liveRow.locator('[data-act="uninstall"]').click();
await page.waitForSelector("#ask[open]");
out.askUninstall = await page.locator("#ask-title").textContent();
await page.click("#ask-ok");
await page.waitForFunction(() => document.querySelector("#ask").open && /Zz Fixture Live App/.test(document.querySelector("#ask-body").textContent), null, { timeout: 60000 });
out.askLeftover = { title: await page.locator("#ask-title").textContent(), path: await page.locator("#ask-path").textContent(), keyGoneBeforeAnswer: !keyExists("ZzFixtureLive") };
await page.click("#ask-ok");
await page.waitForFunction(() => document.querySelector("#toast").classList.contains("is-on"), null, { timeout: 15000 });
await page.waitForTimeout(400);
out.afterLive = { toast: await page.locator("#toast").textContent(), key: keyExists("ZzFixtureLive"), dir: existsSync(LIVE_DIR), rowGone: await liveRow.evaluate((el) => el.classList.contains("app--gone")) };
await page.fill("#apps-search", "");

// ----- Tab Thư mục thừa -----
const t1 = Date.now();
await page.click('[data-tab="leftovers"]');
await page.waitForFunction(() => document.querySelector("#left-scanbar").hidden && [...items.values()].some((i) => i.group === "leftover") && ![...items.values()].some((i) => i.group === "leftover" && i.measuring), null, { timeout: 10 * 60 * 1000 });
out.leftScanSeconds = ((Date.now() - t1) / 1000).toFixed(1);
out.leftovers = await page.evaluate(() => {
  const all = [...items.values()].filter((i) => i.group === "leftover");
  return {
    total: all.length, hero: document.querySelector("#left-hero").textContent, badge: document.querySelector("#badge-leftovers").textContent,
    byArea: Object.fromEntries(["appdata", "programdata", "programs", "packages"].map((a) => [a, all.filter((i) => i.area === a).length])),
    top: all.sort((a, b) => b.bytes - a.bytes).slice(0, 8).map((i) => `${i.label} ${fmtBytes(i.bytes)} [${i.area}]${i.needsAdmin ? " admin" : ""}`),
    locked: document.querySelectorAll("#left-list .row input:disabled").length,
  };
});
await page.screenshot({ path: path.join(here, "shots", "slclean-real-leftovers.png") });
// Bỏ mọi tick tự động của tab Dọn dẹp để nút Dọn chỉ xoá đúng thư mục mồ côi giả.
await page.evaluate(() => { for (const it of items.values()) it.checked = false; syncVisibleRowChecks(); refreshTotals(); });
await page.fill("#left-search", "zzorphan");
await page.waitForTimeout(200);
const orphanRow = page.locator("#left-list .row").filter({ hasText: "ZzOrphanFixtureApp" });
out.orphanRow = { count: await orphanRow.count(), tags: await orphanRow.locator(".tag").allTextContents(), size: await orphanRow.locator(".row__size").textContent() };
await orphanRow.locator(".row__label").click();
out.tallyAfterTick = await page.locator("#tally-bytes").textContent();
await page.click("#btn-clean");
await page.waitForSelector("#confirm[open]");
out.confirmTitle = await page.locator("#confirm-title").textContent();
await page.click("#confirm-ok");
await page.waitForFunction(() => !document.querySelector("#progress-done").hidden, null, { timeout: 60000 });
out.afterOrphan = { done: await page.locator("#progress-done").textContent(), dir: existsSync(ORPHAN_DIR) };
await page.click("#progress-close");
await page.fill("#left-search", "");
out.errors = errors;
console.log(JSON.stringify(out, null, 2));
await browser.close();
