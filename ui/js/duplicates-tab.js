// Tab File trùng: nhóm file giống hệt nhau từng byte. Mỗi bản là một mục trong `items`
// (group "dupe") nên tick ở đây dùng chung nút Dọn ở cột trái như mọi tab khác.
//
// Luật an toàn của tab này: không bao giờ tick hộ, và nếu người dùng tick *mọi* bản trong một
// nhóm thì phải cảnh báo — xoá sạch cả nhóm nghĩa là mất luôn file, không còn bản nào.
"use strict";

/** Nhóm từ backend, giữ nguyên thứ tự lãng phí giảm dần. */
let dupGroups = [];
let dupQuery = "";
let dupMin = Number(localStorage.getItem("slclean-dup-min")) || 1024 * 1024;
// `dupScanning` khai báo ở ledger-view-and-rows.js cùng các cờ quét khác, vì nút Dọn chung
// phải biết mọi tab nào đang quét.
let dupWasted = 0;
/** Nhóm đang gập lại (theo khoá nhóm). */
const dupCollapsed = new Set();

const dupKey = (g) => `${g.bytes}:${g.files[0].path}`;

function dupMatches(g) {
  if (!dupQuery) return true;
  return g.files.some((f) => f.path.toLowerCase().includes(dupQuery));
}

/** Một bản trong nhóm → mục sổ cái, để dùng chung đường dọn với các tab khác. */
function dupItem(g, f) {
  return {
    id: `dup:${f.path}`, group: "dupe", label: String(f.path).split("\\").filter(Boolean).slice(-1)[0] || f.path,
    note: "", path: f.path, paths: [f.path], bytes: g.bytes, files: 1, safety: "review", keepRoot: false,
    needsAdmin: false, checked: false, gone: false, partial: false, age: f.modified, measuring: false,
  };
}

/** Số bản còn lại (chưa xoá) và số bản đang được tick trong một nhóm. */
function dupCounts(g) {
  let live = 0, checked = 0;
  for (const f of g.files) {
    const it = items.get(`dup:${f.path}`);
    if (!it || it.gone) continue;
    live++;
    if (it.checked) checked++;
  }
  return { live, checked };
}

function dupFileRow(g, f, oldestPath, newestPath) {
  const it = items.get(`dup:${f.path}`);
  if (!it || it.gone) return null;
  const li = document.createElement("li");
  li.className = "dupe__file";
  li.dataset.id = it.id;
  const tags = [];
  if (f.path === oldestPath) tags.push(`<b class="tag tag--src" title="${esc(t("dup.oldestTitle"))}">${t("dup.oldest")}</b>`);
  if (f.path === newestPath) tags.push(`<b class="tag tag--new">${t("dup.newest")}</b>`);
  if (f.same_dir_as_sibling) tags.push(`<b class="tag tag--exe" title="${esc(t("dup.sameDirTitle"))}">${t("dup.sameDir")}</b>`);
  li.innerHTML = `
    <label class="check"><input type="checkbox" ${it.checked ? "checked" : ""}><span class="check__box"></span></label>
    <div class="dupe__main">
      <div class="row__title"><span class="row__label">${esc(it.label)}</span>${tags.join("")}</div>
      <div class="row__path" title="${esc(f.path)}"><bdi>${esc(shortPath(f.path))}</bdi></div>
    </div>
    <span class="row__age">${esc(fmtAge(f.modified))}</span>
    <button class="row__open" title="${esc(t("row.open"))}" aria-label="${esc(t("row.open"))}">${REVEAL_SVG}</button>`;
  li.classList.toggle("row--checked", it.checked);
  const input = li.querySelector("input");
  input.addEventListener("change", () => {
    it.checked = input.checked;
    li.classList.toggle("row--checked", it.checked);
    warnIfWholeGroupChecked(g);
    refreshTotals();
    paintDupGroupHead(g);
  });
  li.addEventListener("click", (e) => {
    if (e.target.closest(".row__open") || e.target.closest(".check")) return;
    input.checked = !input.checked;
    input.dispatchEvent(new Event("change"));
  });
  li.querySelector(".row__open").addEventListener("click", () => revealItemInDir(f.path).catch(() => toast(t("toast.openFail"))));
  return li;
}

/** Tick hết một nhóm là xoá sạch file đó khỏi máy; phải nói rõ chứ không im lặng cho qua. */
function warnIfWholeGroupChecked(g) {
  const { live, checked } = dupCounts(g);
  if (live > 1 && checked === live) toast(t("dup.warnAll", { n: live }));
}

function paintDupGroupHead(g) {
  const el = document.querySelector(`[data-dup="${CSS.escape(dupKey(g))}"]`);
  if (!el) return;
  const { live, checked } = dupCounts(g);
  el.classList.toggle("dupe--all-checked", live > 1 && checked === live);
  el.querySelector(".dupe__count").textContent = t("dup.copies", { n: live });
}

function dupGroupEl(g) {
  const sec = document.createElement("section");
  sec.className = "dupe";
  sec.dataset.dup = dupKey(g);
  const collapsed = dupCollapsed.has(dupKey(g));
  const sorted = [...g.files].sort((a, b) => a.modified - b.modified);
  const oldestPath = sorted[0]?.path;
  const newestPath = sorted[sorted.length - 1]?.path;
  const head = document.createElement("header");
  head.className = "dupe__head";
  head.innerHTML = `
    <button class="dupe__toggle" aria-expanded="${!collapsed}">${collapsed ? "▸" : "▾"}</button>
    <span class="dupe__name">${esc(sorted[0] ? String(sorted[0].path).split("\\").filter(Boolean).slice(-1)[0] : "")}</span>
    <span class="dupe__count"></span>
    <span class="dupe__each mono">${fmtBytes(g.bytes)}</span>
    <span class="dupe__wasted mono" title="${esc(t("dup.wastedTitle"))}">${fmtBytes(g.wasted)}</span>`;
  sec.appendChild(head);
  const list = document.createElement("ol");
  list.className = "dupe__files";
  list.hidden = collapsed;
  for (const f of sorted) {
    const row = dupFileRow(g, f, oldestPath, newestPath);
    if (row) list.appendChild(row);
  }
  sec.appendChild(list);
  head.querySelector(".dupe__toggle").addEventListener("click", () => {
    const key = dupKey(g);
    if (dupCollapsed.has(key)) dupCollapsed.delete(key);
    else dupCollapsed.add(key);
    renderDupes();
  });
  return sec;
}

function renderDupes() {
  const box = $("#dup-groups");
  box.innerHTML = "";
  const shown = dupGroups.filter((g) => dupMatches(g) && dupCounts(g).live > 1);
  for (const g of shown) {
    box.appendChild(dupGroupEl(g));
    paintDupGroupHead(g);
  }
  $("#dup-empty").hidden = shown.length > 0 || dupScanning;
  $("#dup-empty").textContent = dupGroups.length ? t("dup.emptyFilter") : t("dup.none", { b: fmtBytes(dupMin) });
  const hidden = dupGroups.length - shown.length;
  $("#dup-hint").textContent = hidden ? t("dup.hiddenHint", { n: hidden }) : "";
  refreshDupTotals();
}

function refreshDupTotals() {
  // Lãng phí tính lại từ số bản còn sống: xoá bớt bản là con số phải giảm theo.
  let wasted = 0, groups = 0;
  for (const g of dupGroups) {
    const { live } = dupCounts(g);
    if (live > 1) { groups++; wasted += g.bytes * (live - 1); }
  }
  dupWasted = wasted;
  $("#dup-hero").textContent = groups ? fmtBytes(wasted) : "—";
  if (!dupScanning) $("#dup-sub").textContent = groups ? t("dup.summary", { n: groups, b: fmtBytes(wasted) }) : (dupGroups.length ? t("dup.allResolved") : t("dup.subIdle"));
  setTabBadge("dupes", groups, false);
}

function paintDupProgress() {
  $("#dup-scanbar").hidden = !dupScanning;
  $("#btn-dup-cancel").hidden = !dupScanning;
  if (!dupScanning) return;
  $("#dup-scanbar-fill").style.setProperty("--pct", "100%");
  $("#dup-sub").textContent = t("dup.scanning", { n: dupGroups.length });
}

async function scanDuplicates() {
  if (dupScanning) return;
  dupScanning = true;
  dupGroups = [];
  dupCollapsed.clear();
  for (const [k, it] of items) if (it.group === "dupe") items.delete(k);
  $("#btn-dup-scan").disabled = true;
  renderDupes();
  paintDupProgress();
  const unGroup = await listen("dup-group", (e) => {
    dupGroups.push(e.payload);
    for (const f of e.payload.files) items.set(`dup:${f.path}`, dupItem(e.payload, f));
    paintDupProgress();
  });
  try {
    const out = await invoke("scan_duplicates", { minSize: dupMin });
    // Kết quả cuối là nguồn sự thật (quét huỷ giữa chừng có thể thiếu event).
    dupGroups = out.groups;
    for (const [k, it] of items) if (it.group === "dupe") items.delete(k);
    for (const g of out.groups) for (const f of g.files) items.set(`dup:${f.path}`, dupItem(g, f));
  } catch (err) {
    toast(String(err));
  } finally {
    unGroup();
    dupScanning = false;
    $("#btn-dup-scan").disabled = false;
    paintDupProgress();
    renderDupes();
    refreshTotals();
  }
}

/** Sau khi dọn: bản đã xoá biến mất khỏi nhóm, nhóm còn một bản thì không còn là nhóm trùng. */
function refreshDupesAfterClean() {
  if (dupGroups.length) renderDupes();
}

$("#btn-dup-scan").addEventListener("click", scanDuplicates);
if ($("#btn-dup-export-html")) {
  $("#btn-dup-export-html").addEventListener("click", () => {
    const activeGroups = dupGroups.filter((g) => dupCounts(g).live > 1);
    const wasted = activeGroups.reduce((s, g) => s + Number(g.bytes || 0) * Math.max(0, dupCounts(g).live - 1), 0);
    const tr = activeGroups.slice(0, 150).map((g) => {
      const livePaths = g.files.filter((f) => { const it = items.get(`dup:${f.path}`); return it && !it.gone; });
      const pathsHtml = livePaths.map((f) => `<code>${esc(f.path)}</code>`).join("<br>");
      return `<tr><td>${livePaths.length}</td><td>${esc(fmtBytes(g.bytes || 0))}</td><td>${esc(fmtBytes((g.bytes || 0) * Math.max(0, livePaths.length - 1)))}</td><td>${pathsHtml}</td></tr>`;
    }).join("\n");
    const html = `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>SLClean Duplicate Files Report</title><style>body{margin:0;padding:28px;background:#0f1411;color:#e6efe9;font-family:system-ui,sans-serif}.wrap{max-width:1040px;margin:0 auto}.card{background:#161d19;border:1px solid #2a3730;border-radius:10px;padding:18px;margin-bottom:16px}table{width:100%;border-collapse:collapse;font-size:12.5px}th,td{padding:8px 10px;border-bottom:1px solid #24302a;text-align:left;vertical-align:top}th{color:#93a89b}code{font-family:ui-monospace,monospace;color:#4ade80}</style></head><body><div class="wrap"><div class="card"><h1>SLClean Byte-Identical Duplicate Files Report</h1><p>Groups: <b>${activeGroups.length}</b> &middot; Reclaimable Waste: <b>${esc(fmtBytes(wasted))}</b></p></div><div class="card"><table><thead><tr><th>Copies</th><th>File Size</th><th>Wasted</th><th>Paths</th></tr></thead><tbody>${tr}</tbody></table></div></div></body></html>`;
    if (typeof downloadBlob === "function") {
      downloadBlob("slclean-duplicates-report.html", "text/html;charset=utf-8", html);
      toast(lang === "vi" ? "Đã xuất slclean-duplicates-report.html" : "Exported slclean-duplicates-report.html");
    }
  });
}
$("#btn-dup-cancel").addEventListener("click", () => invoke("cancel_scan"));
$("#dup-min").addEventListener("change", (e) => {
  dupMin = Number(e.target.value);
  localStorage.setItem("slclean-dup-min", String(dupMin));
  scanDuplicates();
});
let dupSearchTimer;
$("#dup-search").addEventListener("input", (e) => {
  clearTimeout(dupSearchTimer);
  dupSearchTimer = setTimeout(() => { dupQuery = e.target.value.trim().toLowerCase(); renderDupes(); }, 80);
});
$("#dup-search").addEventListener("keydown", (e) => { if (e.key === "Escape") { e.target.value = ""; dupQuery = ""; renderDupes(); } });

// Chọn nhanh an toàn: luôn giữ lại đúng 1 bản gốc (mới nhất hoặc cũ nhất) trong mỗi nhóm hiển thị
function pickDuplicates(mode) {
  const shown = dupGroups.filter((g) => dupMatches(g) && dupCounts(g).live > 1);
  for (const g of shown) {
    const liveFiles = g.files
      .filter((f) => { const it = items.get(`dup:${f.path}`); return it && !it.gone; })
      .sort((a, b) => a.modified - b.modified);
    if (liveFiles.length < 2) continue;
    const keepPath = mode === "keep-newest"
      ? liveFiles[liveFiles.length - 1].path
      : mode === "keep-oldest"
        ? liveFiles[0].path
        : null;
    for (const f of liveFiles) {
      const it = items.get(`dup:${f.path}`);
      if (!it) continue;
      it.checked = mode === "clear" ? false : f.path !== keepPath;
    }
  }
  renderDupes();
  refreshTotals();
}
for (const btn of document.querySelectorAll("[data-dup-pick]")) {
  btn.addEventListener("click", () => pickDuplicates(btn.dataset.dupPick));
}

$("#dup-min").value = String(dupMin);
onTabFirstShow("dupes", scanDuplicates);
