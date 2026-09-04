// Trên app thật (mở qua launch-app-with-devtools-port.ps1): quét một lần để có mốc đầu, tạo một
// file 200 MB trong %TEMP% (mục "Temp người dùng" của danh mục), quét lại, rồi mở tab Nhật ký và
// kiểm chứng phần chênh có nêu đúng mục Temp với đúng cỡ. Xoá file ngay sau khi đo.
// Nhật ký thật gộp theo ngày nên script phải chèn thẳng một mốc "hôm qua" vào history.json để
// hai lần quét trong cùng phiên vẫn cho ra hai mốc so được.
// Dùng: node scripts/e2e-history-in-real-app.mjs [port]
import { chromium } from "playwright-core";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync, unlinkSync, mkdirSync, rmSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const port = Number(process.argv[2] || 9224);
const here = path.dirname(fileURLToPath(import.meta.url));
const HISTORY = path.join(process.env.APPDATA, "vn.salyyy.slclean", "history.json");
// Thư mục con riêng trong %TEMP% để file to không lẫn vào rác thật của máy.
const BLOB_DIR = path.join(process.env.TEMP, "ZzHistoryFixture");
const BLOB = path.join(BLOB_DIR, "blob.bin");
const MB = 1024 * 1024;
const BLOB_MB = 200;
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

const out = { historyExisted: existsSync(HISTORY) };
const scanned = () => page.waitForFunction(() => /^(Quét xong|Scanned in|Quét lỗi|Scan failed)/.test(document.querySelector("#scan-status").textContent), null, { timeout: 15 * 60 * 1000 });
const readHistory = () => JSON.parse(readFileSync(HISTORY, "utf8"));

// ----- Lần quét 1: mốc "trước" -----
await scanned();
await page.waitForTimeout(800);
out.snapsAfterFirstScan = readHistory().snapshots.length;
const first = readHistory();
const firstSnap = first.snapshots[first.snapshots.length - 1];
out.firstSnapItems = firstSnap.items.length;
out.tempBefore = firstSnap.items.find((i) => i.id === "user-temp")?.bytes ?? null;

// Lùi mốc vừa ghi về hôm qua để lần quét sau không gộp cùng ngày với nó.
firstSnap.at -= 86400;
writeFileSync(HISTORY, JSON.stringify(first));

// ----- Tạo file 200 MB trong %TEMP% -----
mkdirSync(BLOB_DIR, { recursive: true });
ps(`$f=[IO.File]::Create('${BLOB}'); $f.SetLength(${BLOB_MB} * 1MB); $f.Close()`);
out.blobBytes = Number(ps(`(Get-Item -LiteralPath '${BLOB}').Length`));

// ----- Lần quét 2: mốc "sau" -----
try {
  await page.click("#btn-scan");
  await page.waitForTimeout(500);
  await scanned();
  await page.waitForTimeout(1200);
  const after = readHistory();
  out.snapsAfterSecondScan = after.snapshots.length;
  const lastSnap = after.snapshots[after.snapshots.length - 1];
  out.tempAfter = lastSnap.items.find((i) => i.id === "user-temp")?.bytes ?? null;
  out.tempGrewMB = out.tempBefore !== null && out.tempAfter !== null ? ((out.tempAfter - out.tempBefore) / MB).toFixed(1) : null;

  // ----- Tab Nhật ký: mục Temp phải nằm trong danh sách chênh, cỡ khớp file vừa tạo -----
  await page.click('[data-tab="history"]');
  await page.waitForFunction(() => document.querySelectorAll("#hist-list .drift__row").length > 0, null, { timeout: 30000 });
  out.hero = await page.locator("#hist-hero").textContent();
  out.sub = await page.locator("#hist-sub").textContent();
  out.pickerOptions = await page.evaluate(() => [...document.querySelectorAll("#hist-from option")].map((o) => o.textContent));
  out.drives = await page.evaluate(() => [...document.querySelectorAll("#hist-drives .drift__row")].map((r) => `${r.querySelector(".drift__label").textContent.trim()} ${r.querySelector(".drift__delta").textContent}`));
  out.rows = await page.evaluate(() => [...document.querySelectorAll("#hist-list .drift__row")].slice(0, 8).map((r) => ({ label: r.querySelector(".drift__label").textContent.trim(), span: r.querySelector(".drift__span").textContent, delta: r.querySelector(".drift__delta").textContent })));
  out.tempRow = await page.evaluate(() => {
    const c = (typeof histDiff === "object" && histDiff ? histDiff.changes : []).find((x) => x.id === "user-temp");
    return c ? { vi: c.vi, before: c.before, after: c.after, delta: c.delta } : null;
  });
  out.tempDeltaMB = out.tempRow ? (out.tempRow.delta / MB).toFixed(1) : null;
  // Bộ lọc "Phình ra" phải giữ lại mục Temp.
  await page.click('[data-hist-filter="grew"]');
  await page.waitForTimeout(200);
  out.grewHasTemp = await page.evaluate(() => [...document.querySelectorAll("#hist-list .drift__label")].some((e) => /Temp/i.test(e.textContent)));
  await page.click('[data-hist-filter="all"]');
  await page.screenshot({ path: path.join(here, "shots", "slclean-real-history.png") });
} finally {
  // File 200 MB không được để lại trên máy người dùng dù script hỏng giữa chừng.
  if (existsSync(BLOB)) unlinkSync(BLOB);
  rmSync(BLOB_DIR, { recursive: true, force: true });
  out.blobCleanedUp = !existsSync(BLOB_DIR);
}

out.errors = errors;
console.log(JSON.stringify(out, null, 2));
await browser.close();
