// Tab Thư mục thừa: thư mục trong AppData / ProgramData / Program Files / Local\Packages mà
// không app đã cài nào nhận. Mỗi thư mục là một mục trong `items` (group "leftover") nên tick
// ở đây cộng vào tổng "Đã chọn để dọn" và được dọn bằng cùng nút Dọn ở thanh trái.
"use strict";

let leftFilter = "all";
let leftQuery = "";
const leftStats = { total: 0, done: 0 };

/** Bộ lọc riêng của tab (gọi từ passesFilter cho mục group "leftover"). */
function leftoverPasses(it) {
  if (leftFilter === "safe_only") {
    if (it.verdict !== "safe_delete" && it.safety !== "safe") return false;
  } else if (leftFilter !== "all" && it.area !== leftFilter) {
    return false;
  }
  if (!leftQuery) return true;
  return [L(it.label), it.path, L(it.note)].some((s) => String(s).toLowerCase().includes(leftQuery));
}

function areaTag(area) {
  return `<b class="tag tag--area">${t(`left.area.${area}`)}</b>`;
}

function verdictTag(verdict, confidence) {
  const conf = confidence ? ` ${confidence}%` : "";
  if (verdict === "safe_delete") {
    return `<b class="tag tag--hygiene" style="background:rgba(74,222,128,.16);color:#4ade80">✓ ${esc(t("left.verdictSafe"))}${conf}</b>`;
  }
  if (verdict === "keep") {
    return `<b class="tag tag--rebuild" style="background:rgba(96,165,250,.16);color:#60a5fa">🛡 ${esc(t("left.verdictKeep"))}${conf}</b>`;
  }
  return `<b class="tag tag--review">⚠ ${esc(t("left.verdictReview"))}${conf}</b>`;
}

/** Leftover từ backend → mục sổ cái. */
function leftoverItem(l) {
  const name = String(l.path).split("\\").filter(Boolean).slice(-1)[0] || l.path;
  const verdict = l.verdict || "review";
  const extraTags = [areaTag(l.area), verdictTag(verdict, verdict === "safe_delete" ? 90 : null)];
  if (l.has_exe) extraTags.push(`<b class="tag tag--exe" title="${esc(t("left.hasExeTitle"))}">${t("left.hasExe")}</b>`);
  return {
    id: l.id, group: "leftover", area: l.area, verdict, label: name, note: l.reason || l.note, path: l.path, paths: [l.path],
    bytes: 0, files: 0, safety: verdict === "safe_delete" ? "safe" : "review", keepRoot: false, needsAdmin: l.needs_admin,
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
    // Protect items explicitly advised as KEEP from accidental bulk selection!
    if (it.verdict === "keep") continue;
    if (mode === "safe") {
      if (it.verdict === "safe_delete" || it.safety === "safe") it.checked = true;
      continue;
    }
    if (passesFilter(it)) it.checked = true;
  }
  syncVisibleRowChecks();
  refreshTotals();
}

async function scanAiJunk() {
  const btn = $("#btn-ai-junk-scan");
  if (btn) btn.disabled = true;
  try {
    const rep = await invoke("scan_ai_junk", { roots: [], staleDays: 7 });
    for (const [k, it] of items) {
      if (it.group === "leftover" && (it.area === "ai_junk" || it.area === "installers")) items.delete(k);
    }
    for (const f of (rep && rep.findings) || []) {
      const area = f.category === "leftover_installer" ? "installers" : "ai_junk";
      const extraTags = [areaTag(area), verdictTag(f.verdict, f.confidence)];
      if (f.git_dirty) {
        extraTags.push(`<b class="tag tag--exe" title="Uncommitted git changes detected">git dirty</b>`);
      }
      items.set(f.id, {
        id: f.id,
        group: "leftover",
        area,
        verdict: f.verdict,
        label: f.name,
        note: {
          vi: `${f.reason_vi} — ${f.impact_vi}`,
          en: `${f.reason_en} — ${f.impact_en}`,
        },
        path: f.path,
        paths: [f.path],
        bytes: f.bytes || 0,
        files: f.files || 0,
        safety: f.verdict === "safe_delete" ? "safe" : (f.verdict === "keep" ? "rebuild" : "review"),
        keepRoot: false,
        needsAdmin: false,
        locked: f.verdict === "keep" && f.git_dirty,
        checked: false,
        gone: false,
        partial: false,
        age: f.modified || null,
        measuring: false,
        extraTags,
        lastUsed: 0,
      });
    }
    leftFilter = rep && rep.safe_delete_count > 0 ? "safe_only" : "ai_junk";
    renderAll();
    toast(t("left.aiJunkFound", {
      safe: rep?.safe_delete_count || 0,
      review: rep?.review_count || 0,
      keep: rep?.keep_count || 0,
    }));
  } catch (err) {
    toast(String(err));
  } finally {
    if (btn) btn.disabled = false;
  }
}

async function scanHygiene() {
  const btn = $("#btn-hygiene-scan");
  if (btn) btn.disabled = true;
  try {
    const rep = await invoke("scan_hygiene", { roots: [] });
    for (const [k, it] of items) if (it.group === "leftover" && it.area === "hygiene") items.delete(k);
    for (const f of (rep && rep.findings) || []) {
      const p = String(f.path);
      const name = p.split(/[\\/]/).filter(Boolean).slice(-1)[0] || p;
      const isBroken = f.kind === "broken_symlink";
      items.set(`hyg:${p}`, {
        id: `hyg:${p}`,
        group: "leftover",
        area: "hygiene",
        label: name,
        note: f.detail || (isBroken ? "Broken symlink (SLC010)" : "Empty directory (SLC010)"),
        path: p,
        paths: [p],
        bytes: 0,
        files: 0,
        safety: "safe",
        keepRoot: false,
        needsAdmin: false,
        checked: false,
        gone: false,
        partial: false,
        age: null,
        measuring: false,
        extraTags: [areaTag("hygiene"), `<b class="tag tag--hygiene">${isBroken ? "symlink" : "empty"}</b>`],
        lastUsed: 0,
      });
    }
    leftFilter = "hygiene";
    renderAll();
    toast(t("left.hygieneFound", { e: rep?.empty_dir_count || 0, b: rep?.broken_link_count || 0 }));
  } catch (err) {
    toast(String(err));
  } finally {
    if (btn) btn.disabled = false;
  }
}

$("#btn-left-scan").addEventListener("click", scanLeftovers);
if ($("#btn-ai-junk-scan")) $("#btn-ai-junk-scan").addEventListener("click", scanAiJunk);
if ($("#btn-hygiene-scan")) $("#btn-hygiene-scan").addEventListener("click", scanHygiene);
if ($("#btn-hygiene-export-html")) {
  $("#btn-hygiene-export-html").addEventListener("click", () => {
    const list = [...items.values()].filter((it) => it.group === "leftover" && !it.gone);
    const totalBytes = list.reduce((s, it) => s + Number(it.bytes || 0), 0);
    const tr = list.slice(0, 200).map((it) =>
      `<tr><td><b>${esc(it.label)}</b></td><td><code>${esc(it.area || "leftover")}</code></td><td>${esc(fmtBytes(it.bytes || 0))}</td><td><code>${esc(it.path || "")}</code></td></tr>`
    ).join("\n");
    const html = `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>SLClean Leftovers &amp; Hygiene Report</title><style>body{margin:0;padding:28px;background:#0f1411;color:#e6efe9;font-family:system-ui,sans-serif}.wrap{max-width:1020px;margin:0 auto}.card{background:#161d19;border:1px solid #2a3730;border-radius:10px;padding:18px;margin-bottom:16px}table{width:100%;border-collapse:collapse;font-size:12.5px}th,td{padding:8px 10px;border-bottom:1px solid #24302a;text-align:left}th{color:#93a89b}code{font-family:ui-monospace,monospace;color:#4ade80}</style></head><body><div class="wrap"><div class="card"><h1>SLClean Leftovers &amp; Filesystem Hygiene Report</h1><p>Entries: <b>${list.length}</b> &middot; Reclaimable: <b>${esc(fmtBytes(totalBytes))}</b></p></div><div class="card"><table><thead><tr><th>Name</th><th>Area</th><th>Size</th><th>Path</th></tr></thead><tbody>${tr}</tbody></table></div></div></body></html>`;
    if (typeof downloadBlob === "function") {
      downloadBlob("slclean-leftovers-hygiene.html", "text/html;charset=utf-8", html);
      toast(lang === "vi" ? "Đã xuất slclean-leftovers-hygiene.html" : "Exported slclean-leftovers-hygiene.html");
    }
  });
}
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
