// Chụp mọi ảnh README trên app thật (mở qua launch-app-with-devtools-port.ps1): ba tab ở tiếng
// Việt, tab Dọn dẹp ở tiếng Anh, và hộp Cài đặt. Chờ quét xong và toast tắt để ảnh sạch.
// Dùng: node scripts/shoot-all-tabs-in-real-app.mjs [port]
import { chromium } from "playwright-core";
import path from "node:path";
import { fileURLToPath } from "node:url";

const port = Number(process.argv[2] || 9224);
const shots = path.join(path.dirname(fileURLToPath(import.meta.url)), "shots");

let browser;
for (let i = 0; i < 30 && !browser; i++) {
  try { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`); } catch { await new Promise((r) => setTimeout(r, 1000)); }
}
if (!browser) { console.error("cannot connect to DevTools port", port); process.exit(1); }
let page;
for (let i = 0; i < 30 && !page; i++) {
  page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes("index.html") || p.url().startsWith("http://tauri.localhost") || p.url().startsWith("tauri://"));
  if (!page) await new Promise((r) => setTimeout(r, 500));
}
if (!page) { console.error("no app page"); process.exit(1); }

const shot = (name) => page.screenshot({ path: path.join(shots, `${name}.png`) });
const setLang = async (want) => {
  if ((await page.evaluate(() => lang)) !== want) {
    await page.click("#btn-lang");
    await page.waitForFunction((w) => lang === w, want, { timeout: 10000 });
    await page.waitForTimeout(400);
  }
};
/** Mở một tab và chờ nó quét/đo xong. */
const openTab = async (name) => {
  await page.click(`[data-tab="${name}"]`);
  if (name === "apps") {
    await page.waitForFunction(() => document.querySelectorAll(".app").length > 0 && document.querySelector("#apps-scanbar").hidden, null, { timeout: 10 * 60 * 1000 });
  } else if (name === "leftovers") {
    await page.waitForFunction(() => document.querySelectorAll("#left-list .row").length > 0 && document.querySelector("#left-scanbar").hidden, null, { timeout: 10 * 60 * 1000 });
  }
  await page.waitForTimeout(600);
};

await page.waitForFunction(() => /^(Quét xong|Scanned in|Quét lỗi|Scan failed)/.test(document.querySelector("#scan-status").textContent), null, { timeout: 15 * 60 * 1000 });
// Toast của lần chạy trước có thể còn hiện; đợi nó tắt hẳn.
await page.waitForFunction(() => !document.querySelector("#toast").classList.contains("is-on"), null, { timeout: 20000 }).catch(() => {});

await setLang("vi");
await openTab("clean");
await shot("slclean-real-vi");
await openTab("apps");
await shot("slclean-real-apps");
// Nhóm thành phần hệ thống: chụp bộ lọc riêng của nó (máy sạch thì tab "mục chết" trống).
await page.click('[data-apps-filter="component"]');
await page.waitForTimeout(300);
await shot("slclean-real-apps-components");
await page.click('[data-apps-filter="all"]');
await openTab("leftovers");
await shot("slclean-real-leftovers");

await openTab("clean");
await setLang("en");
await shot("slclean-real-en");
await page.click("#btn-settings");
await page.waitForSelector("#settings[open]");
await page.waitForTimeout(400);
await shot("slclean-settings");
await page.keyboard.press("Escape");
await setLang("vi");

console.log("shots taken");
await browser.close();
