// Bấm nút đổi ngôn ngữ trên app thật, đợi vẽ lại, chụp ảnh, rồi đổi về như cũ để không
// thay đổi cài đặt người dùng. Dùng: node scripts/switch-real-app-language-and-capture.mjs [port]
import { chromium } from "playwright-core";
import path from "node:path";
import { fileURLToPath } from "node:url";
const port = Number(process.argv[2] || 9223);
const here = path.dirname(fileURLToPath(import.meta.url));
const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
const page = browser.contexts().flatMap((c) => c.pages())[0];
await page.fill("#search", "");
await page.click('[data-filter="all"]');
const before = await page.evaluate(() => lang);
await page.click("#btn-lang");
await page.waitForTimeout(300);
const after = await page.evaluate(() => ({ lang, clean: document.querySelector("#btn-clean").textContent, first: document.querySelector(".row .row__label")?.textContent, note: document.querySelector(".row .row__note")?.textContent }));
await page.screenshot({ path: path.join(here, "shots", `slclean-real-${after.lang}.png`) });
await page.click("#btn-lang");
await page.waitForTimeout(200);
const restored = await page.evaluate(() => lang);
console.log(JSON.stringify({ before, after, restored }));
await browser.close();
