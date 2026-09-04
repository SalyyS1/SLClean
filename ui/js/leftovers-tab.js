// Tab Thư mục thừa: thư mục trong AppData / ProgramData / Program Files / Local\Packages mà
// không app đã cài nào nhận. Mỗi thư mục là một mục trong `items` (group "leftover") nên tick
// ở đây cộng vào tổng "Đã chọn để dọn" và được dọn bằng cùng nút Dọn ở thanh trái.
"use strict";

let leftFilter = "all";
let leftQuery = "";
const leftStats = { total: 0, done: 0 };

/** Bộ lọc riêng của tab (gọi từ passesFilter cho mục group "leftover"). */
function leftoverPasses(it) {
  if (leftFilter !== "all" && it.area !== leftFilter) return false;
  if (!leftQuery) return true;
  return [L(it.label), it.path, L(it.note)].some((s) => String(s).toLowerCase().includes(leftQuery));
}

function areaTag(area) {
  return `<b class="tag tag--area">${t(`left.area.${area}`)}</b>`;
}

/** Leftover từ backend → mục sổ cái. */
function leftoverItem(l) {
  const name = String(l.path).split("\\").filter(Boolean).slice(-1)[0] || l.path;
  const extraTags = [areaTag(l.area)];
  if (l.has_exe) extraTags.push(`<b class="tag tag--exe" title="${esc(t("left.hasExeTitle"))}">${t("left.hasExe")}</b>`);
  return {
    id: l.id, group: "leftover", area: l.area, label: name, note: l.note, path: l.path, paths: [l.path],
    bytes: 0, files: 0, safety: "review", keepRoot: false, needsAdmin: l.needs_admin,
    checked: false, gone: false, partial: false, age: l.modified, measuring: true, extraTags,
    lastUsed: l.last_used,
  };
}

function paintLeftProgress() {
  const bar = $("#left-scanbar");
  bar.hidden = !leftScanning;
  if (!leftScanning) return;
  const pct = leftStats.total ? (leftStats.done / leftStats.total) * 100 : 0;
  $("#left-scanbar-fill").style.setProperty("--pct", `${pct.toFixed(1)}%`);
  $("#left-sub").textContent = t("left.measuring", { done: leftStats.done, total: leftStats.total });
}

function refreshLeftTotals() {
  let n = 0, bytes = 0, hidden = 0;
  for (const it of items.values()) {
    if (it.group !== "leftover" || it.gone) continue;
    n++;
    bytes += it.bytes;
    if (!passesFilter(it) && it.bytes > 0) hidden++;
  }
  $("#left-hero").textContent = n ? fmtBytes(bytes) : "—";
  if (!leftScanning) $("#left-sub").textContent = n ? t("left.summary", { n }) : t("left.subIdle");
  $("#left-hint").textContent = hidden ? t("filter.hintCount", { n: hidden }) : "";
  const list = $("#left-list");
  $("#left-empty").hidden = list.children.length > 0 || leftScanning;
  $("#left-empty").textContent = n ? t("flat.empty") : t("left.empty");
  for (const b of document.querySelectorAll("[data-left-filter]")) b.classList.toggle("is-active", b.dataset.leftFilter === leftFilter);
  setTabBadge("leftovers", n, false);
}

function applyLeftoverSize(d) {
  const it = items.get(d.id);
  if (!it) return;
  if (!d.done) {
    updateMeasuring(d.id, d.bytes, d.files);
    return;
  }
  it.bytes = d.bytes;
  it.files = d.files;
  it.partial = d.denied > 0;
  it.measuring = false;
  leftStats.done++;
  paintLeftProgress();
  upsert(it);
}

async function scanLeftovers() {
  if (leftScanning) return;
  leftScanning = true;
  for (const [k, it] of items) if (it.group === "leftover") items.delete(k);
  Object.assign(leftStats, { total: 0, done: 0 });
  $("#btn-left-scan").disabled = true;
  renderAll();
  paintLeftProgress();
  const unList = await listen("leftovers-list", (e) => {
    leftStats.total = e.payload.length;
    for (const l of e.payload) upsert(leftoverItem(l));
    paintLeftProgress();
  });
  const unSize = await listen("leftover-size", (e) => applyLeftoverSize(e.payload));
  try {
    await invoke("scan_leftovers");
  } catch (err) {
    toast(String(err));
  } finally {
    unList();
    unSize();
    leftScanning = false;
    for (const it of items.values()) if (it.group === "leftover" && it.measuring) it.measuring = false;
    $("#btn-left-scan").disabled = false;
    paintLeftProgress();
    renderAll();
  }
}

function pickLeftovers(mode) {
  for (const it of items.values()) {
    if (it.group !== "leftover" || it.gone || unselectable(it)) continue;
    if (mode === "none") { it.checked = false; continue; }
    if (passesFilter(it)) it.checked = true;
  }
  syncVisibleRowChecks();
  refreshTotals();
}

$("#btn-left-scan").addEventListener("click", scanLeftovers);
for (const b of document.querySelectorAll("[data-left-filter]")) {
  b.addEventListener("click", () => { leftFilter = b.dataset.leftFilter; renderAll(); });
}
for (const b of document.querySelectorAll("[data-left-pick]")) b.addEventListener("click", () => pickLeftovers(b.dataset.leftPick));
let leftSearchTimer;
$("#left-search").addEventListener("input", (e) => {
  clearTimeout(leftSearchTimer);
  leftSearchTimer = setTimeout(() => { leftQuery = e.target.value.trim().toLowerCase(); renderAll(); }, 80);
});
$("#left-search").addEventListener("keydown", (e) => { if (e.key === "Escape") { e.target.value = ""; leftQuery = ""; renderAll(); } });

onTabFirstShow("leftovers", scanLeftovers);
