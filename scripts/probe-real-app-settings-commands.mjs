// Gọi các lệnh cài đặt/thư mục trên app thật qua DevTools để chắc backend trả đúng dữ liệu
// (không phải cầu nối giả): project_roots, get_settings, save_settings roundtrip giữ nguyên
// backslash, ui_language. Cài đặt gốc được khôi phục sau khi thử.
import { chromium } from "playwright-core";
const port = Number(process.argv[2] || 9223);
const browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
const page = browser.contexts().flatMap((c) => c.pages())[0];
const out = await page.evaluate(async () => {
  const inv = window.__TAURI__.core.invoke;
  const roots = await inv("project_roots");
  const before = await inv("get_settings");
  const langBefore = await inv("ui_language");
  const extra = ["D:\\Project\\slclean\\scripts"];
  const excluded = ["D:\\Work"];
  await inv("save_settings", { settings: { ...before, extra_roots: extra, excluded_roots: excluded } });
  const after = await inv("get_settings");
  const rootsAfter = await inv("project_roots");
  await inv("save_settings", { settings: before });
  const restored = await inv("get_settings");
  return { roots, before, langBefore, after, rootsAfter, restored, roundtripOk: after.extra_roots[0] === extra[0] && after.excluded_roots[0] === excluded[0] };
});
console.log(JSON.stringify(out, null, 2));
await browser.close();
