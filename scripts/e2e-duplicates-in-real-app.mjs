// Trên app thật (mở qua launch-app-with-devtools-port.ps1): tạo ba bản y hệt nhau 50 MB (một
// bản ở thư mục khác) cộng một file cùng cỡ nhưng khác nội dung, mở tab File trùng, kiểm chứng
// nhóm có đúng ba bản và lãng phí đúng 100 MB, tick hai bản rồi dọn, PowerShell xác nhận trên
// đĩa còn đúng một bản. Không tin số app hiện.
// Dùng: node scripts/e2e-duplicates-in-real-app.mjs [port]
import { chromium } from "playwright-core";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, rmSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const port = Number(process.argv[2] || 9224);
const here = path.dirname(fileURLToPath(import.meta.url));
// Không đặt trong D:\tmp: thư mục đó là một mục của danh mục cache, mà tab File trùng cố tình
// loại mọi đường dẫn trong danh mục ra (tab Dọn dẹp đã dọn cả cụm rồi).
const ROOT = "D:\\ZzDupFixture";
const A = `${ROOT}\\a\\copy1.bin`;
const B = `${ROOT}\\a\\copy2.bin`;
const C = `${ROOT}\\b\\copy3.bin`;
const IMPOSTOR = `${ROOT}\\b\\different.bin`;
const MB = 50;
const ps = (cmd) => execFileSync("powershell", ["-NoProfile", "-Command", cmd], { encoding: "utf8" }).trim();
const alive = (p) => existsSync(p);

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

const out = {};
mkdirSync(`${ROOT}\\a`, { recursive: true });
mkdirSync(`${ROOT}\\b`, { recursive: true });
// Ba bản giống hệt nhau, và một file cùng cỡ nhưng nội dung khác (byte cuối khác).
ps(`$b=New-Object byte[] (${MB}*1MB); (New-Object Random 42).NextBytes($b); [IO.File]::WriteAllBytes('${A}',$b); [IO.File]::WriteAllBytes('${B}',$b); [IO.File]::WriteAllBytes('${C}',$b); $b[$b.Length-1]=$b[$b.Length-1] -bxor 255; [IO.File]::WriteAllBytes('${IMPOSTOR}',$b)`);
out.fixture = {
  sizes: ps(`@('${A}','${B}','${C}','${IMPOSTOR}') | ForEach-Object { (Get-Item -LiteralPath $_).Length } | Sort-Object -Unique`),
  // Băm để chắc chắn ba bản thật sự giống nhau và impostor thật sự khác.
  hashes: ps(`@('${A}','${B}','${C}','${IMPOSTOR}') | ForEach-Object { (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash.Substring(0,8) }`).split(/\s+/),
};

try {
  await page.waitForFunction(() => /^(Quét xong|Scanned in|Quét lỗi|Scan failed)/.test(document.querySelector("#scan-status").textContent), null, { timeout: 15 * 60 * 1000 });

  const t0 = Date.now();
  await page.click('[data-tab="dupes"]');
  await page.waitForFunction(() => document.querySelector("#dup-scanbar").hidden && typeof dupGroups !== "undefined" && dupGroups.length >= 0 && !dupScanning, null, { timeout: 20 * 60 * 1000 });
  out.scanSeconds = ((Date.now() - t0) / 1000).toFixed(1);
  out.summary = await page.evaluate(() => ({
    groups: dupGroups.length, hero: document.querySelector("#dup-hero").textContent, sub: document.querySelector("#dup-sub").textContent,
    badge: document.querySelector("#badge-dupes").textContent,
    preChecked: document.querySelectorAll(".dupe__file input:checked").length,
    top: dupGroups.slice(0, 6).map((g) => `${g.files.length}x ${fmtBytes(g.bytes)} → ${fmtBytes(g.wasted)} :: ${g.files[0].path}`),
  }));
  await page.screenshot({ path: path.join(here, "shots", "slclean-real-dupes.png") });

  // Nhóm của fixture: đúng ba bản, không có impostor, lãng phí = 2 bản.
  out.fixtureGroup = await page.evaluate((root) => {
    const g = dupGroups.find((x) => x.files.some((f) => f.path.startsWith(root)));
    if (!g) return null;
    return { files: g.files.map((f) => f.path), bytes: g.bytes, wasted: g.wasted, hasImpostor: g.files.some((f) => f.path.includes("different.bin")) };
  }, ROOT);

  if (!out.fixtureGroup) throw new Error(`không tìm thấy nhóm của fixture (${out.summary.groups} nhóm tổng)`);

  // Tick hai bản (giữ bản cũ nhất) rồi dọn bằng nút chung.
  await page.fill("#dup-search", "zzdupfixture");
  await page.waitForTimeout(400);
  out.rowsAfterSearch = await page.locator(".dupe").count();
  await page.evaluate(() => { for (const it of items.values()) it.checked = false; renderDupes(); refreshTotals(); });
  await page.locator(".dupe").first().locator(".dupe__file").nth(1).locator(".row__label").click();
  await page.locator(".dupe").first().locator(".dupe__file").nth(2).locator(".row__label").click();
  await page.waitForTimeout(200);
  out.tallyAfterTick = await page.locator("#tally-bytes").textContent();
  out.warnedOnPartial = await page.evaluate(() => document.querySelector(".dupe").classList.contains("dupe--all-checked"));
  await page.click("#btn-clean");
  await page.waitForSelector("#confirm[open]");
  out.confirmTitle = await page.locator("#confirm-title").textContent();
  await page.click("#confirm-ok");
  await page.waitForFunction(() => !document.querySelector("#progress-done").hidden, null, { timeout: 120000 });
  out.done = await page.locator("#progress-done").textContent();
  await page.click("#progress-close");
  await page.waitForTimeout(500);
  out.onDisk = { copy1: alive(A), copy2: alive(B), copy3: alive(C), impostor: alive(IMPOSTOR) };
  out.survivors = [A, B, C].filter(alive).length;
  // Nhóm còn một bản thì không còn là nhóm trùng nữa.
  out.groupGoneFromUi = await page.evaluate(() => document.querySelectorAll(".dupe").length === 0);
  await page.fill("#dup-search", "");
} finally {
  rmSync(ROOT, { recursive: true, force: true });
  out.cleanedUp = !existsSync(ROOT);
}

out.errors = errors;
console.log(JSON.stringify(out, null, 2));
await browser.close();
