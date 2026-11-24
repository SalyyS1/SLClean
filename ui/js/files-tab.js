// Tab File lớn: file đơn lẻ ≥ ngưỡng trên mọi ổ, và bản đồ dung lượng theo thư mục để lần ra
// chỗ chứa. Mỗi file là một mục trong `items` (group "bigfile") nên tick ở đây cộng vào tổng
// "Đã chọn để dọn" và dùng chung nút Dọn ở thanh trái — không có đường xoá riêng nào.
"use strict";

let bigView = "list";
let bigQuery = "";
let bigThreshold = Number(localStorage.getItem("slclean-big-threshold")) || 100 * 1024 * 1024;
/** Bản đồ thư mục theo gốc, và thư mục đang mở (null = đang ở mức gốc). */
let bigMap = [];
let bigAt = null;
const bigStats = { files: 0, bytes: 0, found: 0 };

/** Bộ lọc riêng của tab (gọi từ passesFilter cho mục group "bigfile"). */
function bigFilePasses(it) {
  if (!bigQuery) return true;
  return [it.label, it.path].some((s) => String(s).toLowerCase().includes(bigQuery));
}

/** File từ backend → mục sổ cái. File hệ thống và ổ đĩa ảo WSL/Docker bị khoá tick nhưng vẫn hiện. */
function bigFileItem(f) {
  const name = String(f.path).split("\\").filter(Boolean).slice(-1)[0] || f.path;
  const extraTags = [];
  if (f.ext) extraTags.push(`<b class="tag tag--src">${esc(f.ext)}</b>`);
  const isVhdx = f.lock_kind === "vhdx";
  if (f.locked) {
    extraTags.push(`<b class="tag tag--component" title="${esc(t(isVhdx ? "big.vhdxTitle" : "big.lockedTitle"))}">${t(isVhdx ? "big.vhdx" : "big.locked")}</b>`);
  }
  const note = f.locked ? t(isVhdx ? "big.vhdxNote" : "big.lockedNote") : "";
  return {
    id: `big:${f.path}`, group: "bigfile", label: name, note, path: f.path, paths: [f.path],
    bytes: f.bytes, files: 1, safety: "review", keepRoot: false, needsAdmin: false, locked: f.locked,
    checked: false, gone: false, partial: false, age: f.modified, measuring: false, extraTags,
  };
}

/** Thư mục đang xem trong bản đồ: gốc thì là danh sách ổ, không thì là con của nó. */
function bigMapNodes() {
  return bigAt ? bigAt.children : bigMap;
}

function bigMapRow(node, max) {
  const li = document.createElement("li");
  const hasChildren = node.children.length > 0;
  li.className = `drift__row${hasChildren ? " drift__row--into" : ""}`;
  li.style.setProperty("--rel", `${max ? (node.bytes / max) * 100 : 0}%`);
  const name = String(node.path).replace(/\\$/, "").split("\\").filter(Boolean).slice(-1)[0] || node.path;
  li.innerHTML = `
    <span class="drift__label">${esc(name)}${hasChildren ? ` <small>${t("big.subfolders", { n: node.children.length })}</small>` : ""}</span>
    <span class="drift__span">${t("big.fileCount", { n: node.files })}</span>
    <span class="drift__delta">${fmtBytes(node.bytes)}</span>`;
  if (hasChildren) {
    li.tabIndex = 0;
    li.addEventListener("click", () => { bigAt = node; renderBigMap(); });
    li.addEventListener("keydown", (ev) => { if (ev.key === "Enter" || ev.key === " ") { ev.preventDefault(); bigAt = node; renderBigMap(); } });
  }
  return li;
}

/** Đường dẫn phân cấp để quay lên: "Mọi ổ / D:\ / Game". */
function renderBigCrumbs() {
  const nav = $("#big-crumbs");
  nav.innerHTML = "";
  const chain = [];
  // Cây chỉ có liên kết cha → con, nên lần ngược bằng cách tìm từ gốc xuống nút đang mở.
  const walk = (node, trail) => {
    if (node === bigAt) { chain.push(...trail, node); return true; }
    return node.children.some((c) => walk(c, [...trail, node]));
  };
  if (bigAt) bigMap.some((root) => walk(root, []));
  const crumb = (label, node) => {
    const b = document.createElement("button");
    b.className = "crumbs__item";
    b.textContent = label;
    b.disabled = node === bigAt;
    b.addEventListener("click", () => {
      bigAt = node;
      if (bigView === "treemap") renderBigTreemap();
      else renderBigMap();
    });
    nav.appendChild(b);
  };
  crumb(t("big.allDrives"), null);
  for (const node of chain) {
    const name = String(node.path).replace(/\\$/, "").split("\\").filter(Boolean).slice(-1)[0] || node.path;
    nav.appendChild(Object.assign(document.createElement("span"), { className: "crumbs__sep", textContent: "/" }));
    crumb(name, node);
  }
}

function renderBigMap() {
  const list = $("#big-map");
  list.innerHTML = "";
  const nodes = [...bigMapNodes()].sort((a, b) => b.bytes - a.bytes);
  const max = Math.max(1, ...nodes.map((n) => n.bytes));
  for (const n of nodes) list.appendChild(bigMapRow(n, max));
  renderBigCrumbs();
  $("#big-empty").hidden = nodes.length > 0 || bigScanning;
  $("#big-empty").textContent = t("big.emptyMap");
}

/** Recursive proportional treemap partitioning in percentage space (0..100%). */
function layoutTreemap(nodes, x, y, w, h, out) {
  if (!nodes.length || w <= 0 || h <= 0) return;
  if (nodes.length === 1) {
    out.push({ node: nodes[0], x, y, w, h });
    return;
  }
  const total = nodes.reduce((s, n) => s + Math.max(1, n.bytes), 0);
  let acc = 0;
  let split = 1;
  for (let i = 0; i < nodes.length - 1; i++) {
    acc += Math.max(1, nodes[i].bytes);
    split = i + 1;
    if (acc >= total * 0.5) break;
  }
  const left = nodes.slice(0, split);
  const right = nodes.slice(split);
  const ratio = left.reduce((s, n) => s + Math.max(1, n.bytes), 0) / total;
  if (w >= h) {
    const lw = w * ratio;
    layoutTreemap(left, x, y, lw, h, out);
    layoutTreemap(right, x + lw, y, w - lw, h, out);
  } else {
    const lh = h * ratio;
    layoutTreemap(left, x, y, w, lh, out);
    layoutTreemap(right, x, y + lh, w, h - lh, out);
  }
}

function renderBigTreemap() {
  const host = $("#big-treemap");
  if (!host) return;
  host.innerHTML = "";
  const nodes = [...bigMapNodes()].filter((n) => n.bytes > 0).sort((a, b) => b.bytes - a.bytes);
  renderBigCrumbs();
  $("#big-empty").hidden = nodes.length > 0 || bigScanning;
  $("#big-empty").textContent = t("big.emptyMap");
  if (!nodes.length) return;
  const total = nodes.reduce((s, n) => s + n.bytes, 0) || 1;
  // Cap to top 24 visible tiles so tiny 0.1% slices remain legible; fold remainder into last tile if needed.
  const visible = nodes.slice(0, 24);
  const rects = [];
  layoutTreemap(visible, 0, 0, 100, 100, rects);
  rects.forEach((r, idx) => {
    const node = r.node;
    const hasChildren = node.children && node.children.length > 0;
    const name = String(node.path).replace(/\\$/, "").split("\\").filter(Boolean).slice(-1)[0] || node.path;
    const pct = ((node.bytes / total) * 100).toFixed(1);
    const tile = document.createElement(hasChildren ? "button" : "div");
    if (hasChildren) tile.type = "button";
    tile.className = `treemap__tile treemap__tile--t${idx % 6}${hasChildren ? " treemap__tile--into" : ""}`;
    tile.style.left = `${r.x.toFixed(3)}%`;
    tile.style.top = `${r.y.toFixed(3)}%`;
    tile.style.width = `${r.w.toFixed(3)}%`;
    tile.style.height = `${r.h.toFixed(3)}%`;
    tile.title = `${node.path}\n${fmtBytes(node.bytes)} (${pct}%) · ${t("big.fileCount", { n: node.files })}`;
    if (r.w >= 8 && r.h >= 12) {
      tile.innerHTML = `<span class="treemap__name">${esc(name)}</span><span class="treemap__meta">${fmtBytes(node.bytes)} · ${pct}%</span>`;
    } else if (r.w >= 5 && r.h >= 8) {
      tile.innerHTML = `<span class="treemap__meta">${esc(name)}</span>`;
    }
    if (hasChildren) {
      tile.addEventListener("click", () => { bigAt = node; renderBigTreemap(); });
    }
    host.appendChild(tile);
  });
}

/** Đổi giữa danh sách file, bản đồ thư mục và bản đồ khối treemap. */
function renderBigView() {
  const isList = bigView === "list";
  const isMap = bigView === "map";
  const isTree = bigView === "treemap";
  $("#big-list").hidden = !isList;
  $("#big-map").hidden = !isMap;
  if ($("#big-treemap")) $("#big-treemap").hidden = !isTree;
  $("#big-crumbs").hidden = isList;
  for (const b of document.querySelectorAll("[data-big-view]")) b.classList.toggle("is-active", b.dataset.bigView === bigView);
  if (isMap) renderBigMap();
  else if (isTree) renderBigTreemap();
  else renderAll();
}

function refreshBigTotals() {
  let n = 0, bytes = 0, hidden = 0;
  for (const it of items.values()) {
    if (it.group !== "bigfile" || it.gone) continue;
    n++;
    bytes += it.bytes;
    if (!passesFilter(it)) hidden++;
  }
  $("#big-hero").textContent = n ? fmtBytes(bytes) : "—";
  if (!bigScanning) $("#big-sub").textContent = n ? t("big.summary", { n, b: fmtBytes(bytes) }) : (items.size ? t("big.none", { b: fmtBytes(bigThreshold) }) : t("big.subIdle"));
  $("#big-hint").textContent = hidden ? t("filter.hintCount", { n: hidden }) : "";
  if (bigView === "list") {
    $("#big-empty").hidden = $("#big-list").children.length > 0 || bigScanning;
    $("#big-empty").textContent = n ? t("flat.empty") : t("big.none", { b: fmtBytes(bigThreshold) });
  }
  setTabBadge("files", n, false);
  renderBigExtStrip();
}

function renderBigExtStrip() {
  const host = $("#big-ext-strip");
  if (!host) return;
  const totals = new Map();
  for (const it of items.values()) {
    if (it.group !== "bigfile" || it.gone) continue;
    const m = /\.([a-z0-9]{1,12})$/i.exec(it.label || "");
    const ext = m ? `.${m[1].toLowerCase()}` : "(none)";
    const cur = totals.get(ext) || { count: 0, bytes: 0 };
    cur.count += 1;
    cur.bytes += it.bytes;
    totals.set(ext, cur);
  }
  const top = [...totals.entries()].sort((a, b) => b[1].bytes - a[1].bytes).slice(0, 8);
  host.innerHTML = "";
  host.hidden = bigView !== "list" || top.length === 0;
  for (const [ext, stat] of top) {
    const btn = document.createElement("button");
    btn.type = "button";
    btn.className = `quick__btn${bigQuery === ext ? " is-active" : ""}`;
    btn.textContent = `${ext} · ${fmtBytes(stat.bytes)} (${stat.count})`;
    btn.addEventListener("click", () => {
      bigQuery = bigQuery === ext ? "" : ext;
      const inp = $("#big-search");
      if (inp) inp.value = bigQuery;
      renderAll();
    });
    host.appendChild(btn);
  }
}

function paintBigProgress() {
  const bar = $("#big-scanbar");
  bar.hidden = !bigScanning;
  $("#btn-big-cancel").hidden = !bigScanning;
  if (!bigScanning) return;
  // Không biết trước tổng số file nên thanh chạy vô định; con số mới là thứ cho biết đang tới đâu.
  $("#big-scanbar-fill").style.setProperty("--pct", "100%");
  $("#big-sub").textContent = t("big.scanning", { n: bigStats.files.toLocaleString(), b: fmtBytes(bigStats.bytes), f: bigStats.found });
}

async function scanBigFiles() {
  if (bigScanning) return;
  bigScanning = true;
  for (const [k, it] of items) if (it.group === "bigfile") items.delete(k);
  Object.assign(bigStats, { files: 0, bytes: 0, found: 0 });
  bigMap = [];
  bigAt = null;
  $("#btn-big-scan").disabled = true;
  renderAll();
  paintBigProgress();
  const unFile = await listen("big-file", (e) => {
    upsert(bigFileItem(e.payload));
    bigStats.found++;
  });
  const unProg = await listen("big-progress", (e) => {
    Object.assign(bigStats, e.payload);
    paintBigProgress();
  });
  try {
    const out = await invoke("scan_big_files", { threshold: bigThreshold });
    bigMap = out.map;
    // Kết quả cuối là nguồn sự thật (quét huỷ giữa chừng có thể thiếu vài event). Thay cả tập
    // rồi vẽ lại một lần, thay vì upsert từng file vào danh sách còn hàng cũ.
    for (const [k, it] of items) if (it.group === "bigfile") items.delete(k);
    for (const f of out.files) items.set(`big:${f.path}`, bigFileItem(f));
    renderAll();
    if (out.denied) toast(t("big.deniedNote", { n: out.denied }));
  } catch (err) {
    toast(String(err));
  } finally {
    unFile();
    unProg();
    bigScanning = false;
    $("#btn-big-scan").disabled = false;
    paintBigProgress();
    renderBigView();
    refreshTotals();
  }
}

$("#btn-big-scan").addEventListener("click", scanBigFiles);
if ($("#btn-big-export-ext-html")) {
  $("#btn-big-export-ext-html").addEventListener("click", () => {
    const agg = new Map();
    let totalBytes = 0;
    let totalFiles = 0;
    for (const it of items.values()) {
      if (it.group !== "bigfile" || it.gone) continue;
      const parts = String(it.label || "").split(".");
      const ext = parts.length > 1 ? ("." + parts.pop().toLowerCase()) : "(no ext)";
      const cur = agg.get(ext) || { ext, count: 0, bytes: 0 };
      cur.count += 1;
      cur.bytes += Number(it.bytes || 0);
      agg.set(ext, cur);
      totalBytes += Number(it.bytes || 0);
      totalFiles += 1;
    }
    const rows = [...agg.values()].sort((a, b) => b.bytes - a.bytes).slice(0, 40);
    const tr = rows.map((r) => {
      const pct = totalBytes > 0 ? ((r.bytes * 100) / totalBytes).toFixed(1) : "0.0";
      return `<tr><td><code>${esc(r.ext)}</code></td><td>${r.count}</td><td>${esc(fmtBytes(r.bytes))}</td><td>${pct}%</td></tr>`;
    }).join("\n");
    const html = `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>SLClean File Extension Breakdown</title><style>body{margin:0;padding:28px;background:#0f1411;color:#e6efe9;font-family:system-ui,sans-serif}.wrap{max-width:900px;margin:0 auto}.card{background:#161d19;border:1px solid #2a3730;border-radius:10px;padding:18px;margin-bottom:16px}table{width:100%;border-collapse:collapse;font-size:13px}th,td{padding:8px 10px;border-bottom:1px solid #24302a;text-align:left}th{color:#93a89b}code{font-family:ui-monospace,monospace;color:#4ade80}</style></head><body><div class="wrap"><div class="card"><h1>SLClean Big Files Extension Breakdown</h1><p>Files: <b>${totalFiles}</b> &middot; Total Size: <b>${esc(fmtBytes(totalBytes))}</b> &middot; Extensions: <b>${agg.size}</b></p></div><div class="card"><table><thead><tr><th>Extension</th><th>Count</th><th>Total Size</th><th>Share</th></tr></thead><tbody>${tr}</tbody></table></div></div></body></html>`;
    if (typeof downloadBlob === "function") {
      downloadBlob("slclean-extensions-breakdown.html", "text/html;charset=utf-8", html);
      toast(lang === "vi" ? "Đã xuất slclean-extensions-breakdown.html" : "Exported slclean-extensions-breakdown.html");
    }
  });
}
$("#btn-big-cancel").addEventListener("click", () => invoke("cancel_scan"));
for (const b of document.querySelectorAll("[data-big-view]")) {
  b.addEventListener("click", () => { bigView = b.dataset.bigView; renderBigView(); });
}
$("#big-threshold").addEventListener("change", (e) => {
  bigThreshold = Number(e.target.value);
  localStorage.setItem("slclean-big-threshold", String(bigThreshold));
  scanBigFiles();
});
let bigSearchTimer;
$("#big-search").addEventListener("input", (e) => {
  clearTimeout(bigSearchTimer);
  bigSearchTimer = setTimeout(() => { bigQuery = e.target.value.trim().toLowerCase(); renderAll(); }, 80);
});
$("#big-search").addEventListener("keydown", (e) => { if (e.key === "Escape") { e.target.value = ""; bigQuery = ""; renderAll(); } });

$("#big-threshold").value = String(bigThreshold);
onTabFirstShow("files", scanBigFiles);
