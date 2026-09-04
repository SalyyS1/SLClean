// Đo lệnh Thùng rác trên app thật đang chạy với cổng DevTools (launch-app-with-devtools-port.ps1):
// chờ quét xong, gọi `recycle_bin_info` và đo mất bao lâu, đồng thời hỏi Windows xem cửa sổ
// có "Not responding" trong lúc đó không. Dùng để tái hiện treo khi Thùng rác có nhiều mục.
// Dùng: node scripts/measure-recycle-bin-command-in-real-app.mjs [port] [pid]
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

await page.waitForFunction(() => /^(Quét xong|Scanned in|Quét lỗi|Scan failed)/.test(document.querySelector("#scan-status").textContent), null, { timeout: 15 * 60 * 1000 });
console.log("scan:", await page.evaluate(() => document.querySelector("#scan-status").textContent));

/** Hỏi Windows: tiến trình có còn trả lời message không (đúng cột "Not responding" của Task Manager). */
function responding() {
  if (!pid) return "n/a";
  const out = execFileSync("powershell", ["-NoProfile", "-Command", `(Get-Process -Id ${pid}).Responding`], { encoding: "utf8" });
  return out.trim();
}

const probe = page.evaluate(async () => {
  const t0 = performance.now();
  const r = await window.__TAURI__.core.invoke("recycle_bin_info");
  return { ms: Math.round(performance.now() - t0), ...r };
});
const samples = [];
for (let i = 0; i < 20; i++) {
  await new Promise((r) => setTimeout(r, 1500));
  samples.push(responding());
  const done = await Promise.race([probe.then(() => true), new Promise((r) => setTimeout(() => r(false), 10))]);
  if (done) break;
}
console.log("responding samples while command ran:", samples.join(" "));
console.log("recycle_bin_info:", JSON.stringify(await probe));
console.log("recycle text:", await page.evaluate(() => document.querySelector("#recycle-info").textContent));
await browser.close();
