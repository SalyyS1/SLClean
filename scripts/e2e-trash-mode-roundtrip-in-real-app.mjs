// Kiểm tra ô "Đưa vào Thùng rác" trên app thật (mở qua launch-app-with-devtools-port.ps1):
// 1) tick → dọn dự án giả → thư mục phải nằm trong Thùng rác;
// 2) tạo lại dự án giả, bỏ tick → dọn → thư mục mất hẳn, Thùng rác không tăng.
// Đọc số mục Thùng rác từ shell Windows (không tin số app hiện). Cần fixture từ
// make-throwaway-artifact-fixture.ps1. Dùng: node scripts/e2e-trash-mode-roundtrip-in-real-app.mjs [port]
import { chromium } from "playwright-core";
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const port = Number(process.argv[2] || 9224);
const FIXTURE = "D:\\tmp\\slclean-e2e-fake-project";
const TARGET = path.join(FIXTURE, "node_modules");
const here = path.dirname(fileURLToPath(import.meta.url));

const ps = (cmd) => execFileSync("powershell", ["-NoProfile", "-Command", cmd], { encoding: "utf8" }).trim();
const binCount = () => Number(ps("(New-Object -ComObject Shell.Application).NameSpace(10).Items().Count"));
const binNames = () => ps("(New-Object -ComObject Shell.Application).NameSpace(10).Items() | ForEach-Object { $_.Name }").split(/\r?\n/).filter(Boolean);
const makeFixture = () => ps(`& '${path.join(here, "make-throwaway-artifact-fixture.ps1")}' | Out-Null`);

const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
const page = browser.contexts().flatMap((c) => c.pages())[0];
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));

async function cleanFixture(wantTrash) {
  if (!existsSync(TARGET)) makeFixture();
  await page.click("#btn-scan");
  await page.waitForFunction(() => /^(Quét xong|Scanned in)/.test(document.querySelector("#scan-status").textContent), null, { timeout: 10 * 60 * 1000 });
  // Đặt ô tick đúng trạng thái muốn thử bằng cách bấm như người dùng, rồi đọc lại.
  const now = await page.evaluate(() => document.querySelector("#to-trash").checked);
  if (now !== wantTrash) await page.click(".rail__actions .mode__box");
  const boxChecked = await page.evaluate(() => document.querySelector("#to-trash").checked);
  const id = `art:${TARGET}`;
  await page.click('[data-pick="none"]');
  await page.fill("#search", "slclean-e2e-fake-project");
  await page.waitForTimeout(200);
  await page.locator(`.row[data-id="${id.replace(/\\/g, "\\\\")}"] .row__label`).click();
  const selected = await page.evaluate(() => [...items.values()].filter((i) => i.checked).map((i) => i.id));
  if (selected.length !== 1 || selected[0] !== id) throw new Error("unexpected selection: " + JSON.stringify(selected));
  const before = binCount();
  await page.click("#btn-clean");
  await page.waitForSelector("#confirm[open]");
  const confirmMode = await page.locator("#confirm-mode").textContent();
  const okLabel = await page.locator("#confirm-ok").textContent();
  await page.click("#confirm-ok");
  await page.waitForFunction(() => !document.querySelector("#progress-done").hidden, null, { timeout: 60000 });
  const progressLabel = await page.locator("#progress-label").textContent();
  const doneText = await page.locator("#progress-done").textContent();
  await page.click("#progress-close");
  await page.fill("#search", "");
  await page.waitForTimeout(500);
  return { wantTrash, boxChecked, confirmMode, okLabel, progressLabel, doneText, binBefore: before, binAfter: binCount(), targetGone: !existsSync(TARGET), railRecycle: await page.locator("#recycle-info").textContent() };
}

const ticked = await cleanFixture(true);
const binNow = binNames();
const unticked = await cleanFixture(false);
console.log(JSON.stringify({ ticked, binNamesAfterTicked: binNow, unticked, errors }, null, 2));
await browser.close();
