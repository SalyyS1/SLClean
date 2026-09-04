// Chuyển tab (Dọn dẹp / Ứng dụng / Thư mục thừa), huy hiệu trên tab, và hộp hỏi một câu dùng
// chung cho gỡ app / xoá mục chết / dọn thư mục còn sót. Nạp sau ledger-view-and-rows.js,
// trước apps-tab.js và leftovers-tab.js (hai file đó đăng ký hàm quét lười qua `onTabFirstShow`).
"use strict";

/** Tab đang hiện: "clean" | "apps" | "leftovers". */
let activeTab = localStorage.getItem("slclean-tab") || "clean";
const tabFirstShow = new Map();
const tabShown = new Set();

/** Đăng ký việc cần làm lần đầu tab hiện ra (quét lười). */
function onTabFirstShow(name, fn) {
  tabFirstShow.set(name, fn);
}

function switchTab(name) {
  activeTab = name;
  localStorage.setItem("slclean-tab", name);
  for (const b of document.querySelectorAll(".tab")) {
    const on = b.dataset.tab === name;
    b.classList.toggle("is-active", on);
    b.setAttribute("aria-selected", on ? "true" : "false");
  }
  for (const p of document.querySelectorAll(".pane")) p.hidden = p.id !== `pane-${name}`;
  if (!tabShown.has(name)) {
    tabShown.add(name);
    tabFirstShow.get(name)?.();
  }
}

/** Huy hiệu số trên tab; 0 hoặc rỗng thì ẩn. `warn` tô đỏ (mục chết). */
function setTabBadge(name, n, warn) {
  const b = $(`#badge-${name}`);
  b.hidden = !n;
  b.textContent = n || "";
  b.classList.toggle("tab__badge--warn", !!warn);
}

for (const b of document.querySelectorAll(".tab")) b.addEventListener("click", () => switchTab(b.dataset.tab));

/**
 * Hộp hỏi một câu. Trả Promise<{ ok, option }>; `option` là trạng thái ô tick nếu có.
 * @param {{title:string, body:string, path?:string, option?:string, optionDefault?:boolean, ok:string, danger?:boolean}} o
 */
function ask(o) {
  const dlg = $("#ask");
  $("#ask-title").textContent = o.title;
  $("#ask-body").textContent = o.body;
  const p = $("#ask-path");
  p.hidden = !o.path;
  p.textContent = o.path || "";
  const opt = $("#ask-option");
  opt.hidden = !o.option;
  $("#ask-option-text").textContent = o.option || "";
  $("#ask-option-check").checked = !!o.optionDefault;
  const okBtn = $("#ask-ok");
  okBtn.textContent = o.ok;
  okBtn.classList.toggle("btn--ember", o.danger !== false);
  okBtn.classList.toggle("btn--amber", o.danger === false);
  return new Promise((resolve) => {
    const done = () => {
      dlg.removeEventListener("close", done);
      resolve({ ok: dlg.returnValue === "ok", option: $("#ask-option-check").checked });
    };
    dlg.addEventListener("close", done);
    dlg.showModal();
  });
}

/** Hộp tiến trình ở chế độ "đang chờ" (không có phần trăm). Trả hàm đóng. */
function showBusy(label) {
  const dlg = $("#progress");
  dlg.classList.add("progress--busy");
  $("#progress-label").textContent = label;
  $("#progress-now").textContent = "";
  $("#progress-done").hidden = true;
  $("#progress-close").hidden = true;
  $("#progress-fill").style.setProperty("--pct", "0%");
  if (!dlg.open) dlg.showModal();
  return () => {
    dlg.classList.remove("progress--busy");
    if (dlg.open) dlg.close();
  };
}

/** Lỗi mã ngắn từ backend (gỡ app / mục chết) → câu hiển thị. */
function actionErrorText(code) {
  const s = String(code || "");
  for (const k of ["no-uninstaller", "uninstaller-missing", "uac-cancelled", "needs-admin", "not-dead", "missing"]) {
    if (s === k) return t(`err.${k}`);
  }
  if (s.startsWith("launch-failed")) return t("err.launch-failed", { e: s.replace(/^launch-failed:?\s*/, "") });
  if (s.startsWith("store-refused")) return t("err.store-refused", { e: s.replace(/^store-refused:?\s*/, "") });
  return s;
}
