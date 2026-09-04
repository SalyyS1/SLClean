// Kiểm tra xoá thật trên app đang chạy: quét lại, tìm artifact của dự án giả
// (make-throwaway-artifact-fixture.ps1), tick nó, đi qua hộp xác nhận thật, chờ dọn xong,
// rồi kiểm tra trên đĩa là thư mục node_modules đã biến mất còn package.json vẫn còn.
import { chromium } from "playwright-core";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const port = Number(process.argv[2] || 9223);
const FIXTURE = "D:\\tmp\\slclean-e2e-fake-project";
const TARGET = path.join(FIXTURE, "node_modules");
const here = path.dirname(fileURLToPath(import.meta.url));

if (!existsSync(TARGET)) {
  console.error("fixture missing; run scripts/make-throwaway-artifact-fixture.ps1 first");
  process.exit(1);
}
const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
const page = browser.contexts().flatMap((c) => c.pages())[0];
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));

await page.click("#btn-scan");
await page.waitForFunction(() => /^(Quét xong|Scanned in)/.test(document.querySelector("#scan-status").textContent), null, { timeout: 10 * 60 * 1000 });

const id = `art:${TARGET}`;
const found = await page.evaluate((id) => {
  const it = items.get(id);
  return it ? { bytes: it.bytes, files: it.files, safety: it.safety, keepRoot: it.keepRoot } : null;
}, id);
if (!found) {
  console.error("fixture artifact not found in scan:", id);
  process.exit(1);
}
// Bỏ mọi tick khác rồi chỉ tick fixture, để chắc chắn không xoá gì ngoài nó.
await page.click('[data-pick="none"]');
await page.fill("#search", "slclean-e2e-fake-project");
await page.waitForTimeout(200);
const row = page.locator(`.row[data-id="${id.replace(/\\/g, "\\\\")}"]`);
// Ô tick thật bị ẩn (opacity 0); bấm vào nhãn hàng, handler của hàng sẽ tick.
await row.locator(".row__label").click();
const selected = await page.evaluate(() => [...items.values()].filter((i) => i.checked).map((i) => i.id));
if (selected.length !== 1 || selected[0] !== id) {
  console.error("unexpected selection:", selected);
  process.exit(1);
}
await page.click("#btn-clean");
await page.waitForSelector("#confirm[open]");
const confirmTitle = await page.locator("#confirm-title").textContent();
const confirmList = await page.locator("#confirm-list li").allTextContents();
await page.screenshot({ path: path.join(here, "shots", "slclean-real-confirm.png") });
await page.click("#confirm-ok");
await page.waitForFunction(() => !document.querySelector("#progress-done").hidden, null, { timeout: 60000 });
const doneText = await page.locator("#progress-done").textContent();
const nowText = await page.locator("#progress-now").textContent();
await page.screenshot({ path: path.join(here, "shots", "slclean-real-done.png") });
await page.click("#progress-close");
await page.fill("#search", "");

console.log(JSON.stringify({
  found, confirmTitle, confirmList, doneText, nowText,
  targetGone: !existsSync(TARGET),
  packageJsonKept: existsSync(path.join(FIXTURE, "package.json")),
  errors,
}, null, 2));
await browser.close();
