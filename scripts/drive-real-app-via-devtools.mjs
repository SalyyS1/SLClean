// Nối vào app SLClean thật đang chạy với cổng DevTools (launch-app-with-devtools-port.ps1),
// chờ quét xong, in số liệu thật từ DOM (không phải dữ liệu giả), rồi chụp ảnh webview.
// Dùng: node scripts/drive-real-app-via-devtools.mjs [port]
import { chromium } from "playwright-core";
import path from "node:path";
import { fileURLToPath } from "node:url";

const port = Number(process.argv[2] || 9223);
const here = path.dirname(fileURLToPath(import.meta.url));
const shots = path.join(here, "shots");

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
  console.error("no app page; pages:", browser.contexts().flatMap((c) => c.pages()).map((p) => p.url()));
  process.exit(1);
}
const errors = [];
page.on("pageerror", (e) => errors.push(String(e)));
page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });

const t0 = Date.now();
await page.waitForFunction(() => /^(Quét xong|Scanned in|Quét lỗi|Scan failed)/.test(document.querySelector("#scan-status").textContent), null, { timeout: 15 * 60 * 1000 });
const waited = ((Date.now() - t0) / 1000).toFixed(1);

const data = await page.evaluate(() => {
  const all = [...items.values()];
  const byGroup = {};
  for (const it of all) {
    const g = (byGroup[it.group] ||= { n: 0, bytes: 0 });
    g.n++;
    g.bytes += it.bytes;
  }
  const top = all.sort((a, b) => b.bytes - a.bytes).slice(0, 25).map((it) => ({ id: it.id, label: typeof it.label === "string" ? it.label : it.label.vi, bytes: fmtBytes(it.bytes), safety: it.safety, paths: it.paths.length, partial: it.partial, admin: it.needsAdmin }));
  return {
    lang,
    scanStatus: document.querySelector("#scan-status").textContent,
    hero: document.querySelector("#hero-bytes").textContent,
    selected: document.querySelector("#tally-bytes").textContent,
    items: all.length,
    measuringLeft: all.filter((i) => i.measuring).length,
    zero: all.filter((i) => i.bytes === 0).length,
    byGroup: Object.fromEntries(Object.entries(byGroup).map(([k, v]) => [k, `${v.n} · ${fmtBytes(v.bytes)}`])),
    top,
    drives: [...document.querySelectorAll(".drive")].map((d) => d.textContent.replace(/\s+/g, " ").trim()),
    recycle: document.querySelector("#recycle-info").textContent,
    overflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
    viewport: [window.innerWidth, window.innerHeight],
  };
});
await page.screenshot({ path: path.join(shots, "slclean-real-scanned.png") });
console.log(JSON.stringify({ waited, ...data, errors }, null, 2));
await browser.close();
