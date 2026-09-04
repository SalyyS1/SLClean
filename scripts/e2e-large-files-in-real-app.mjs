// Trên app thật (mở qua launch-app-with-devtools-port.ps1): tạo một file 300 MB trong D:\tmp,
// mở tab File lớn, kiểm chứng file đó được nêu đúng cỡ, tick nó và dọn bằng nút Dọn ở cột trái,
// rồi hỏi PowerShell xem file còn không. Không tin số app hiện; đĩa mới là trọng tài.
// Dùng: node scripts/e2e-large-files-in-real-app.mjs [port]
import { chromium } from "playwright-core";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, rmSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const port = Number(process.argv[2] || 9224);
const here = path.dirname(fileURLToPath(import.meta.url));
const DIR = "D:\\tmp\\ZzBigFileFixture";
const BLOB = `${DIR}\\huge.bin`;
const MB = 300;
const ps = (cmd) => execFileSync("powershell", ["-NoProfile", "-Command", cmd], { encoding: "utf8" }).trim();

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
mkdirSync(DIR, { recursive: true });
ps(`$f=[IO.File]::Create('${BLOB}'); $f.SetLength(${MB} * 1MB); $f.Close()`);
out.fixtureBytes = Number(ps(`(Get-Item -LiteralPath '${BLOB}').Length`));

try {
  await page.waitForFunction(() => /^(Quét xong|Scanned in|Quét lỗi|Scan failed)/.test(document.querySelector("#scan-status").textContent), null, { timeout: 15 * 60 * 1000 });

  const t0 = Date.now();
  await page.click('[data-tab="files"]');
  await page.waitForFunction(() => document.querySelector("#big-scanbar").hidden && [...items.values()].some((i) => i.group === "bigfile"), null, { timeout: 15 * 60 * 1000 });
  out.scanSeconds = ((Date.now() - t0) / 1000).toFixed(1);
  out.summary = await page.evaluate(() => {
    const all = [...items.values()].filter((i) => i.group === "bigfile");
    return {
      total: all.length, hero: document.querySelector("#big-hero").textContent, sub: document.querySelector("#big-sub").textContent,
      badge: document.querySelector("#badge-files").textContent,
      locked: all.filter((i) => i.locked).length, lockedNames: all.filter((i) => i.locked).map((i) => i.label),
      top: all.sort((a, b) => b.bytes - a.bytes).slice(0, 8).map((i) => `${fmtBytes(i.bytes)} ${i.path}`),
      // Không mục nào được tick sẵn: đây là file của người dùng.
      preChecked: all.filter((i) => i.checked).length,
    };
  });
  await page.screenshot({ path: path.join(here, "shots", "slclean-real-files.png") });

  // Bản đồ thư mục: mức gốc phải là các ổ, và đi sâu được.
  await page.click('[data-big-view="map"]');
  await page.waitForTimeout(400);
  out.mapTop = await page.evaluate(() => [...document.querySelectorAll("#big-map .drift__row")].map((r) => `${r.querySelector(".drift__label").textContent.trim()} = ${r.querySelector(".drift__delta").textContent}`));
  await page.locator("#big-map .drift__row").first().click();
  await page.waitForTimeout(300);
  out.mapLevel2 = await page.evaluate(() => [...document.querySelectorAll("#big-map .drift__row")].slice(0, 6).map((r) => `${r.querySelector(".drift__label").textContent.trim()} = ${r.querySelector(".drift__delta").textContent}`));
  await page.screenshot({ path: path.join(here, "shots", "slclean-real-files-map.png") });
  await page.locator("#big-crumbs .crumbs__item").first().click();
  await page.click('[data-big-view="list"]');

  // ----- Tìm file fixture, tick, dọn -----
  await page.fill("#big-search", "huge.bin");
  await page.waitForTimeout(300);
  const row = page.locator("#big-list .row").filter({ hasText: "huge.bin" });
  out.fixtureRow = { count: await row.count(), size: await row.locator(".row__size").textContent(), tags: await row.locator(".tag").allTextContents() };
  // Bỏ mọi tick của tab khác để nút Dọn chỉ đụng đúng file fixture.
  await page.evaluate(() => { for (const it of items.values()) it.checked = false; syncVisibleRowChecks(); refreshTotals(); });
  await row.locator(".row__label").click();
  out.tallyAfterTick = await page.locator("#tally-bytes").textContent();
  await page.click("#btn-clean");
  await page.waitForSelector("#confirm[open]");
  out.confirmTitle = await page.locator("#confirm-title").textContent();
  await page.click("#confirm-ok");
  await page.waitForFunction(() => !document.querySelector("#progress-done").hidden, null, { timeout: 120000 });
  out.done = await page.locator("#progress-done").textContent();
  await page.click("#progress-close");
  out.fileGoneOnDisk = !existsSync(BLOB);
  out.folderStillThere = existsSync(DIR);
  await page.fill("#big-search", "");
} finally {
  rmSync(DIR, { recursive: true, force: true });
  out.cleanedUp = !existsSync(DIR);
}

out.errors = errors;
console.log(JSON.stringify(out, null, 2));
await browser.close();
