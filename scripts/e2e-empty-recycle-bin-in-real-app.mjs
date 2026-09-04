// Bấm nút Thùng rác trên app thật (mở qua launch-app-with-devtools-port.ps1) và kiểm tra:
// toast báo đã dọn, số mục về 0, và cửa sổ không bị "Not responding" trong lúc dọn.
// Chỉ chạy khi Thùng rác chứa toàn file thử của bạn: nút này dọn sạch thật.
// Dùng: node scripts/e2e-empty-recycle-bin-in-real-app.mjs [port] [pid]
import { chromium } from "playwright-core";
import { execFileSync } from "node:child_process";

const port = Number(process.argv[2] || 9224);
const pid = Number(process.argv[3] || 0);

let browser;
for (let i = 0; i < 30; i++) {
  try {
    browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
    break;
  } catch {
    await new Promise((r) => setTimeout(r, 1000));
  }
}
if (!browser) {
  console.error("cannot connect to DevTools port", port);
  process.exit(1);
}
const page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes("index.html") || p.url().startsWith("http://tauri.localhost") || p.url().startsWith("tauri://"));
if (!page) {
  console.error("no app page");
  process.exit(1);
}
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));

function ps(cmd) {
  return execFileSync("powershell", ["-NoProfile", "-Command", cmd], { encoding: "utf8" }).trim();
}
const binCount = () => Number(ps("(New-Object -ComObject Shell.Application).NameSpace(10).Items().Count"));
const responding = () => (pid ? ps(`(Get-Process -Id ${pid}).Responding`) : "n/a");

await page.waitForFunction(() => /^(Quét xong|Scanned in|Quét lỗi|Scan failed)/.test(document.querySelector("#scan-status").textContent), null, { timeout: 15 * 60 * 1000 });
await page.waitForFunction(() => !document.querySelector("#btn-recycle").disabled, null, { timeout: 60_000 });
console.log("before:", await page.evaluate(() => document.querySelector("#recycle-info").textContent), "| shell count:", binCount());

const t0 = Date.now();
await page.click("#btn-recycle");
const samples = [];
let toastText = "";
while (Date.now() - t0 < 120_000) {
  samples.push(responding());
  toastText = await page.evaluate(() => (document.querySelector("#toast").classList.contains("is-on") ? document.querySelector("#toast").textContent : ""));
  if (toastText) break;
  await new Promise((r) => setTimeout(r, 700));
}
console.log("toast:", JSON.stringify(toastText), "after", ((Date.now() - t0) / 1000).toFixed(1), "s");
console.log("responding samples while emptying:", samples.join(" "));
await page.waitForFunction(() => document.querySelector("#btn-recycle").disabled, null, { timeout: 30_000 }).catch(() => {});
console.log("after:", await page.evaluate(() => document.querySelector("#recycle-info").textContent), "| shell count:", binCount());
console.log("js errors:", errors.length ? errors : "none");
await browser.close();
