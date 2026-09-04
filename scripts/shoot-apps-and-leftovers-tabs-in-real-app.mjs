// Chụp lại hai tab trên app thật (cổng DevTools 9224), chờ toast tắt để ảnh sạch.
import { chromium } from "playwright-core";
const browser = await chromium.connectOverCDP("http://127.0.0.1:9224");
const page = browser.contexts().flatMap((c) => c.pages()).find((p) => p.url().includes("index.html") || p.url().startsWith("http://tauri.localhost") || p.url().startsWith("tauri://"));
await page.fill("#left-search", "");
await page.click('[data-tab="apps"]');
await page.waitForFunction(() => document.querySelectorAll(".app").length > 0 && document.querySelector("#apps-scanbar").hidden, null, { timeout: 60000 });
await page.waitForTimeout(4500);
await page.screenshot({ path: "D:/Project/sweep/scripts/shots/slclean-real-apps.png" });
await page.click('[data-tab="leftovers"]');
await page.waitForFunction(() => document.querySelector("#left-scanbar").hidden && document.querySelectorAll("#left-list .row").length > 0, null, { timeout: 60000 });
await page.waitForTimeout(1000);
await page.screenshot({ path: "D:/Project/sweep/scripts/shots/slclean-real-leftovers.png" });
console.log("shots taken");
await browser.close();
