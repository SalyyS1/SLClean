// Luồng: quét (ổ đĩa + danh mục + artifact, chạy song song) → người dùng tick → xác nhận → dọn
// → cập nhật lại ổ đĩa. Vẽ hàng/lọc/sắp xếp ở ledger-view-and-rows.js, cài đặt ở
// settings-dialog.js, chuỗi ở i18n-strings.js (đều nạp trước, dùng chung biến toàn cục).
"use strict";

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { revealItemInDir } = window.__TAURI__.opener;

let drives = [];

// ---------- Ngôn ngữ ----------
function setLanguage(next) {
  lang = next;
  applyI18n();
  $("#btn-lang").textContent = lang === "vi" ? "EN" : "VI";
  renderAll();
  loadDrives();
  loadRecycle();
}

async function loadLanguage() {
  try {
    lang = await invoke("ui_language");
  } catch {
    lang = "vi";
  }
  applyI18n();
  $("#btn-lang").textContent = lang === "vi" ? "EN" : "VI";
}

// ---------- Ổ đĩa ----------
async function loadDrives() {
  drives = await invoke("list_drives");
  const ul = $("#drives");
  ul.innerHTML = "";
  for (const d of drives) {
    const used = d.total - d.free;
    const pct = (used / d.total) * 100;
    const li = document.createElement("li");
    li.className = "drive";
    li.dataset.mount = d.mount.toUpperCase();
    li.innerHTML = `
      <div class="drive__head">
        <span><span class="drive__mount">${esc(d.mount)}</span><span class="drive__name">${esc(d.name || "")}</span></span>
        <span class="drive__free"><b>${fmtBytes(d.free)}</b> ${t("rail.free")} / ${fmtBytes(d.total)}</span>
      </div>
      <div class="drive__bar" style="--pct:${pct.toFixed(1)}%">
        <span class="drive__used ${pct > 95 ? "drive__used--tight" : pct > 85 ? "drive__used--warm" : ""}"></span>
        <span class="drive__gain"></span>
      </div>`;
    ul.appendChild(li);
  }
  paintGains();
}

/** Vạch hổ phách trên thanh ổ đĩa = tổng các mục đã tick nằm trên ổ đó. */
function paintGains() {
  const perDrive = new Map();
  for (const it of items.values()) {
    if (!it.checked || it.gone) continue;
    const m = String(it.path).slice(0, 2).toUpperCase();
    perDrive.set(m, (perDrive.get(m) || 0) + it.bytes);
  }
  for (const li of document.querySelectorAll(".drive")) {
    const d = drives.find((x) => x.mount.toUpperCase() === li.dataset.mount);
    if (!d) continue;
    const gain = ((perDrive.get(li.dataset.mount) || 0) / d.total) * 100;
    li.querySelector(".drive__bar").style.setProperty("--gain", `${Math.min(gain, 100).toFixed(2)}%`);
  }
}

// ---------- Quyền admin ----------
async function loadElevation() {
  try {
    elevated = await invoke("is_elevated");
  } catch {
    elevated = false;
  }
  $("#btn-admin").hidden = elevated;
  $("#admin-note").hidden = !elevated;
}

// ---------- Quét ----------
const scanStats = { total: 0, done: 0, artifacts: 0 };

function paintScanProgress() {
  const bar = $("#scanbar");
  bar.hidden = !scanning;
  if (!scanning) return;
  const pct = scanStats.total ? (scanStats.done / scanStats.total) * 100 : 0;
  $("#scanbar-fill").style.setProperty("--pct", `${pct.toFixed(1)}%`);
  $("#hero-sub").textContent = t("hero.measuring", { done: scanStats.done, total: scanStats.total, art: scanStats.artifacts });
}

function catalogItem(c, measuring) {
  return {
    id: c.id, group: c.group, label: c.label, note: c.note, path: c.path, paths: c.paths,
    bytes: c.bytes || 0, files: c.files || 0, safety: c.safety, keepRoot: c.keep_root, needsAdmin: c.needs_admin,
    checked: !measuring && c.safety === "safe" && c.bytes > 0 && !(c.needs_admin && !elevated),
    gone: false, partial: (c.denied || 0) > 0, age: null, measuring,
  };
}

async function scan() {
  if (scanning) return;
  scanning = true;
  items.clear();
  Object.assign(scanStats, { total: 0, done: 0, artifacts: 0 });
  renderAll();
  $("#empty-artifacts").hidden = false;
  $("#empty-artifacts").textContent = t("artifacts.searching");
  $("#empty-flat").hidden = true;
  $("#btn-scan").disabled = true;
  $("#btn-quick").disabled = true;
  $("#btn-cancel").hidden = false;
  $("#scan-status").textContent = t("rail.scanning");
  paintScanProgress();
  refreshTotals();

  const unStart = await listen("catalog-start", (e) => {
    scanStats.total = e.payload.length;
    for (const c of e.payload) upsert(catalogItem(c, true));
    paintScanProgress();
  });
  const unProg = await listen("catalog-progress", (e) => updateMeasuring(e.payload.id, e.payload.bytes, e.payload.files));
  const unCat = await listen("catalog-item", (e) => {
    scanStats.done++;
    upsert(catalogItem(e.payload, false));
    paintScanProgress();
  });
  const unArt = await listen("artifact-found", (e) => {
    const a = e.payload;
    scanStats.artifacts++;
    $("#empty-artifacts").hidden = true;
    const projName = a.project.split("\\").filter(Boolean).slice(-1)[0];
    upsert({
      id: `art:${a.path}`, group: "artifacts", label: `${projName} \\ ${a.kind}`, note: a.tool, path: a.path, paths: [a.path],
      bytes: a.bytes, files: a.files, safety: "rebuild", keepRoot: false, needsAdmin: false,
      checked: false, gone: false, partial: false, age: a.modified, measuring: false,
    });
    paintScanProgress();
  });

  const t0 = performance.now();
  try {
    await Promise.all([loadDrives(), invoke("scan_catalog"), invoke("scan_artifacts")]);
    const secs = ((performance.now() - t0) / 1000).toFixed(1);
    $("#scan-status").textContent = t("rail.scanDone", { s: secs, n: items.size });
    if (scanStats.artifacts === 0) $("#empty-artifacts").textContent = t("artifacts.none");
  } catch (err) {
    $("#scan-status").textContent = t("rail.scanError", { e: err });
  } finally {
    unStart(); unProg(); unCat(); unArt();
    scanning = false;
    // Mục chưa kịp đo xong (huỷ giữa chừng) không được để ở trạng thái "đang đo".
    for (const it of items.values()) if (it.measuring) it.measuring = false;
    $("#btn-scan").disabled = false;
    $("#btn-quick").disabled = false;
    $("#btn-cancel").hidden = true;
    $("#hero-sub").textContent = t("hero.subIdle");
    paintScanProgress();
    renderAll();
    loadRecycle();
  }
}

// ---------- Thùng rác ----------
async function loadRecycle() {
  try {
    const r = await invoke("recycle_bin_info");
    $("#recycle-info").textContent = r.items ? t("rail.recycleInfo", { n: r.items, b: fmtBytes(r.bytes) }) : t("rail.recycleEmpty");
    $("#btn-recycle").disabled = r.items === 0;
  } catch {
    $("#recycle-info").textContent = t("rail.recycleUnreadable");
  }
}

// ---------- Dọn ----------
function selected() {
  return [...items.values()].filter((i) => i.checked && !i.gone && i.bytes > 0);
}

function openConfirm() {
  const sel = selected();
  if (!sel.length) return;
  const toTrash = $("#to-trash").checked;
  $("#confirm-title").innerHTML = t(sel.length === 1 ? "confirm.titleOne" : "confirm.title", { n: sel.length, b: `<span class="mono">${fmtBytes(sel.reduce((s, i) => s + i.bytes, 0))}</span>` });
  $("#confirm-mode").textContent = toTrash ? t("confirm.trash") : t("confirm.delete");
  $("#confirm-ok").textContent = toTrash ? t("confirm.okTrash") : t("confirm.okDelete");
  const ul = $("#confirm-list");
  ul.innerHTML = "";
  for (const it of sel.sort((a, b) => b.bytes - a.bytes)) {
    const li = document.createElement("li");
    li.innerHTML = `<span class="path" title="${esc(it.paths.join("\n"))}">${esc(L(it.label))}</span><span class="mono">${fmtBytes(it.bytes)}</span>`;
    ul.appendChild(li);
  }
  const reviews = sel.filter((i) => i.safety === "review");
  const warn = $("#confirm-warn");
  warn.hidden = reviews.length === 0;
  if (reviews.length) warn.textContent = t("confirm.warn", { n: reviews.length });
  $("#confirm").showModal();
}

/** Mã lỗi từ backend → câu hiển thị. */
function errorText(code) {
  if (!code) return "";
  if (code.startsWith("trash: ")) return t("err.trash", { e: code.slice(7) });
  return t(`err.${code}`);
}

async function runClean() {
  const sel = selected();
  const toTrash = $("#to-trash").checked;
  const dlg = $("#progress");
  $("#progress-label").textContent = toTrash ? t("progress.trashing") : t("progress.cleaning");
  $("#progress-done").hidden = true;
  $("#progress-close").hidden = true;
  $("#progress-fill").style.setProperty("--pct", "0%");
  $("#progress-now").textContent = "";
  dlg.showModal();

  let done = 0, freed = 0, skipped = 0;
  const un = await listen("clean-progress", (e) => {
    const r = e.payload;
    done++;
    freed += r.freed;
    skipped += r.skipped;
    const it = items.get(r.id);
    $("#progress-fill").style.setProperty("--pct", `${((done / sel.length) * 100).toFixed(1)}%`);
    $("#progress-now").textContent = `${done}/${sel.length} · ${it ? L(it.label) : r.id}${r.error ? " — " + errorText(r.error) : ""}`;
    if (it) {
      it.bytes = Math.max(0, it.bytes - r.freed);
      if (it.bytes === 0 || !r.error) { it.gone = !it.keepRoot || it.bytes === 0; it.checked = false; }
      const li = document.querySelector(`[data-id="${CSS.escape(r.id)}"]`);
      if (li) {
        li.querySelector(".row__size").textContent = fmtBytes(it.bytes);
        li.classList.toggle("row--gone", it.gone);
        li.querySelector("input").checked = false;
        li.classList.remove("row--checked");
        if (it.bytes === 0) li.querySelector("input").disabled = true;
      }
    }
  });
  try {
    await invoke("clean", { items: sel.map((i) => ({ id: i.id, paths: i.paths, keep_root: i.keepRoot })), toTrash });
    $("#progress-label").textContent = t("progress.done");
    const doneEl = $("#progress-done");
    doneEl.hidden = false;
    doneEl.innerHTML = t("progress.freed", { b: fmtBytes(freed) }) + (skipped ? t("progress.skipped", { n: skipped }) : "") + ".";
  } catch (err) {
    $("#progress-label").textContent = t("progress.failed");
    $("#progress-now").textContent = String(err);
  } finally {
    un();
    refreshTotals();
    $("#progress-close").hidden = false;
    loadDrives();
    loadRecycle();
  }
}

// ---------- Toast ----------
let toastTimer;
function toast(msg) {
  const el = $("#toast");
  el.textContent = msg;
  el.classList.add("is-on");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.remove("is-on"), 2600);
}

// ---------- Chọn nhanh ----------
function pick(mode) {
  for (const it of items.values()) {
    if (it.gone || unselectable(it)) continue;
    if (mode === "none") { it.checked = false; continue; }
    if (!passesFilter(it)) continue;
    it.checked = mode === "safe" ? it.safety === "safe" : it.safety === "safe" || it.safety === "rebuild";
  }
  syncVisibleRowChecks();
  refreshTotals();
}

// ---------- Sự kiện ----------
$("#btn-scan").addEventListener("click", scan);
$("#btn-cancel").addEventListener("click", () => invoke("cancel_scan"));
$("#btn-clean").addEventListener("click", openConfirm);
$("#btn-quick").addEventListener("click", () => { pick("safe"); openConfirm(); });
$("#confirm").addEventListener("close", () => { if ($("#confirm").returnValue === "ok") runClean(); });
$("#progress-close").addEventListener("click", () => $("#progress").close());
$("#btn-recycle").addEventListener("click", async () => {
  try {
    const before = await invoke("empty_recycle_bin");
    toast(t("toast.recycleDone", { b: fmtBytes(before.bytes) }));
  } catch (err) {
    toast(t("toast.recycleFail", { e: err }));
  }
  loadDrives();
  loadRecycle();
});
$("#btn-admin").addEventListener("click", async () => {
  try {
    await invoke("relaunch_as_admin");
  } catch (err) {
    const code = String(err);
    toast(code === "uac-cancelled" ? t("toast.uacCancel") : code === "already-elevated" ? t("toast.alreadyElevated") : code);
  }
});
$("#btn-lang").addEventListener("click", async () => {
  const next = lang === "vi" ? "en" : "vi";
  setLanguage(next);
  try {
    const s = await invoke("get_settings");
    s.language = next;
    await invoke("save_settings", { settings: s });
  } catch { /* không lưu được thì vẫn đổi cho phiên này */ }
});

// Cách xem + bộ lọc: nhớ lựa chọn qua localStorage, vẽ lại khi đổi.
function syncViewButtons() {
  for (const b of document.querySelectorAll("[data-view]")) b.classList.toggle("is-active", b.dataset.view === view);
  for (const b of document.querySelectorAll("[data-filter]")) b.classList.toggle("is-active", b.dataset.filter === filter);
}
for (const b of document.querySelectorAll("[data-view]")) {
  b.addEventListener("click", () => { view = b.dataset.view; localStorage.setItem("slclean-view", view); syncViewButtons(); renderAll(); });
}
for (const b of document.querySelectorAll("[data-filter]")) {
  b.addEventListener("click", () => { filter = b.dataset.filter; localStorage.setItem("slclean-filter", filter); syncViewButtons(); renderAll(); });
}
let searchTimer;
$("#search").addEventListener("input", (e) => {
  clearTimeout(searchTimer);
  searchTimer = setTimeout(() => { query = e.target.value.trim().toLowerCase(); renderAll(); }, 80);
});
$("#search").addEventListener("keydown", (e) => { if (e.key === "Escape") { e.target.value = ""; query = ""; renderAll(); } });

for (const btn of document.querySelectorAll("[data-pick]")) btn.addEventListener("click", () => pick(btn.dataset.pick));

// Tick cả nhóm (hoặc cả danh sách phẳng): chỉ các hàng đang hiển thị trong nhóm đó.
for (const gc of document.querySelectorAll("[data-group-check]")) {
  gc.addEventListener("change", () => {
    const list = $(`#rows-${gc.dataset.groupCheck}`);
    for (const li of list.children) {
      const it = items.get(li.dataset.id);
      if (!it || it.gone || unselectable(it)) continue;
      it.checked = gc.checked;
    }
    syncVisibleRowChecks();
    refreshTotals();
  });
}

// ---------- Khởi động ----------
(async () => {
  syncViewButtons();
  await loadLanguage();
  await loadElevation();
  try {
    const s = await invoke("get_settings");
    $("#to-trash").checked = !!s.to_trash;
  } catch { /* mặc định xoá thẳng */ }
  scan();
})();
