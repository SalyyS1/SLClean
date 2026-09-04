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

/** File từ backend → mục sổ cái. File hệ thống bị khoá tick nhưng vẫn hiện. */
function bigFileItem(f) {
  const name = String(f.path).split("\\").filter(Boolean).slice(-1)[0] || f.path;
  const extraTags = [];
  if (f.ext) extraTags.push(`<b class="tag tag--src">${esc(f.ext)}</b>`);
  if (f.locked) extraTags.push(`<b class="tag tag--component" title="${esc(t("big.lockedTitle"))}">${t("big.locked")}</b>`);
  return {
    id: `big:${f.path}`, group: "bigfile", label: name, note: f.locked ? t("big.lockedNote") : "", path: f.path, paths: [f.path],
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
    b.addEventListener("click", () => { bigAt = node; renderBigMap(); });
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

/** Đổi giữa danh sách file và bản đồ thư mục. */
function renderBigView() {
  const isMap = bigView === "map";
  $("#big-list").hidden = isMap;
  $("#big-map").hidden = !isMap;
  $("#big-crumbs").hidden = !isMap;
  for (const b of document.querySelectorAll("[data-big-view]")) b.classList.toggle("is-active", b.dataset.bigView === bigView);
  if (isMap) renderBigMap();
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
