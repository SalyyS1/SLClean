// Tab Nhật ký: mỗi lần quét xong app ghi lại cỡ của từng mục; tab này so mốc mới nhất với một
// mốc cũ để trả lời "cái gì đã ăn ổ đĩa của tôi từ lần trước". Chỉ đọc, không xoá gì.
"use strict";

/** Danh sách mốc (mới nhất ở cuối) và kết quả so hiện tại. */
let histSnaps = [];
let histDiff = null;
let histFilter = "all";
let histLoading = false;
/** Đã nạp nhật ký lần nào chưa; trước đó tab chỉ có chuỗi tĩnh nên không cần vẽ lại. */
let histLoaded = false;
/** Clean Run Ledger (lịch sử dọn dẹp & tổng dung lượng đã giải phóng). */
let cleanLedger = { runs: [] };
let cleanSummary = null;

function histPasses(c) {
  switch (histFilter) {
    case "grew": return c.delta > 0 && c.before !== null;
    case "shrank": return c.delta < 0;
    case "new": return c.before === null;
    default: return true;
  }
}

/** "+1.2 GB" / "-300 MB"; 0 thì không có dấu. */
function fmtDelta(n) {
  if (!n) return fmtBytes(0);
  return `${n > 0 ? "+" : "−"}${fmtBytes(Math.abs(n))}`;
}

/** Giây epoch → "20:45 4/9" theo giờ máy, đủ để phân biệt hai mốc. */
function fmtStamp(epochSec) {
  const d = new Date(epochSec * 1000);
  const two = (n) => String(n).padStart(2, "0");
  return `${two(d.getHours())}:${two(d.getMinutes())} ${d.getDate()}/${d.getMonth() + 1}`;
}

function histLabel(c) {
  return lang === "vi" ? c.vi : c.en;
}

/** Một dòng thay đổi: nhãn, nhóm, cỡ trước → sau, và thanh tỉ lệ theo thay đổi lớn nhất. */
function histRow(c, max) {
  const li = document.createElement("li");
  const up = c.delta > 0;
  li.className = `drift__row ${up ? "drift__row--up" : "drift__row--down"}`;
  li.style.setProperty("--rel", `${max ? (Math.abs(c.delta) / max) * 100 : 0}%`);
  const tags = [];
  if (c.before === null) tags.push(`<b class="tag tag--new">${t("hist.tagNew")}</b>`);
  if (c.after === null) tags.push(`<b class="tag tag--gone">${t("hist.tagGone")}</b>`);
  const from = c.before === null ? "—" : fmtBytes(c.before);
  const to = c.after === null ? "—" : fmtBytes(c.after);
  li.innerHTML = `
    <span class="drift__label">${esc(histLabel(c))}${tags.join("")}</span>
    <span class="drift__span">${from} → ${to}</span>
    <span class="drift__delta">${fmtDelta(c.delta)}</span>`;
  return li;
}

/** Ổ đĩa: trống ra hay đầy thêm bao nhiêu giữa hai mốc. */
function histDriveRow(d) {
  const li = document.createElement("li");
  // Ổ trống ra là tin tốt, nên đảo màu so với dòng mục (mục phình ra là tin xấu). Không đổi thì
  // để trung tính: thanh xanh kín chiều ngang sẽ đọc nhầm thành "vừa dọn được nhiều".
  const tone = d.delta === 0 ? "" : d.delta > 0 ? "drift__row--good" : "drift__row--bad";
  li.className = `drift__row drift__drive ${tone}`;
  li.innerHTML = `
    <span class="drift__label">${esc(d.mount)} <small>${t("hist.freeSpace")}</small></span>
    <span class="drift__span">${fmtBytes(d.before)} → ${fmtBytes(d.after)}</span>
    <span class="drift__delta">${fmtDelta(d.delta)}</span>`;
  return li;
}

function renderCleanLedger() {
  const statsEl = $("#ledger-stats");
  const runsEl = $("#ledger-runs");
  const sumEl = $("#ledger-summary-line");
  const emptyEl = $("#ledger-empty");
  if (!statsEl || !runsEl) return;
  const s = cleanSummary || { total_runs: 0, lifetime_freed_bytes: 0, trash_runs: 0, permanent_runs: 0 };
  if (sumEl) sumEl.textContent = t("ledger.summaryLine", { runs: s.total_runs, freed: fmtBytes(s.lifetime_freed_bytes) });
  statsEl.innerHTML = `
    <div class="ledger-stat"><div class="ledger-stat__k">${esc(t("ledger.lifetimeFreed"))}</div><div class="ledger-stat__v">${fmtBytes(s.lifetime_freed_bytes)}</div></div>
    <div class="ledger-stat"><div class="ledger-stat__k">${esc(t("ledger.totalRuns"))}</div><div class="ledger-stat__v">${s.total_runs}</div></div>
    <div class="ledger-stat"><div class="ledger-stat__k">${esc(t("ledger.trashRuns"))}</div><div class="ledger-stat__v">${s.trash_runs}</div></div>
    <div class="ledger-stat"><div class="ledger-stat__k">${esc(t("ledger.permRuns"))}</div><div class="ledger-stat__v">${s.permanent_runs}</div></div>`;
  runsEl.innerHTML = "";
  const runs = (cleanLedger && Array.isArray(cleanLedger.runs) ? cleanLedger.runs : []).slice(-10).reverse();
  if (emptyEl) emptyEl.hidden = runs.length > 0;
  for (const r of runs) {
    const li = document.createElement("li");
    li.className = "ledger-row";
    const modeTag = r.to_trash
      ? `<b class="tag tag--git">${esc(t("ledger.modeTrash"))}</b>`
      : `<b class="tag tag--review">${esc(t("ledger.modePerm"))}</b>`;
    li.innerHTML = `
      <span class="ledger-row__time">${fmtStamp(r.timestamp)}</span>
      <span class="ledger-row__meta">${esc(t("ledger.itemsCount", { n: (r.items || []).length, r: r.total_removed || 0 }))} · ${esc(r.source || "gui")}</span>
      ${modeTag}
      <span class="ledger-row__freed">${fmtBytes(r.total_freed || 0)}</span>`;
    runsEl.appendChild(li);
  }
}

async function loadCleanLedger() {
  try {
    const [ledger, summary] = await Promise.all([invoke("list_clean_ledger"), invoke("clean_ledger_summary")]);
    cleanLedger = ledger || { runs: [] };
    cleanSummary = summary || null;
  } catch {
    cleanLedger = { runs: [] };
    cleanSummary = null;
  }
  renderCleanLedger();
}

function renderHistory() {
  renderCleanLedger();
  if (!histLoaded && !histLoading) return;
  const list = $("#hist-list");
  const drives = $("#hist-drives");
  list.innerHTML = "";
  drives.innerHTML = "";
  for (const b of document.querySelectorAll("[data-hist-filter]")) b.classList.toggle("is-active", b.dataset.histFilter === histFilter);

  if (histLoading) {
    $("#hist-empty").hidden = false;
    $("#hist-empty").textContent = t("hist.loading");
    return;
  }
  if (histSnaps.length < 2) {
    $("#hist-hero").textContent = "—";
    $("#hist-sub").textContent = t("hist.needTwo", { n: histSnaps.length });
    $("#hist-empty").hidden = false;
    $("#hist-empty").textContent = t("hist.emptyOne");
    return;
  }
  if (!histDiff) return;

  for (const d of histDiff.drives) drives.appendChild(histDriveRow(d));
  const rows = histDiff.changes.filter(histPasses);
  const max = Math.max(1, ...rows.map((c) => Math.abs(c.delta)));
  for (const c of rows) list.appendChild(histRow(c, max));

  const grew = histDiff.changes.filter((c) => c.delta > 0).reduce((s, c) => s + c.delta, 0);
  $("#hist-hero").textContent = fmtDelta(histDiff.total_delta);
  $("#hist-hero").classList.toggle("hero__value--up", histDiff.total_delta > 0);
  $("#hist-sub").textContent = t("hist.summary", { from: fmtStamp(histDiff.from), to: fmtStamp(histDiff.to), grew: fmtBytes(grew), n: histDiff.changes.length });
  const hidden = histDiff.changes.length - rows.length;
  $("#hist-hint").textContent = hidden ? t("hist.hiddenHint", { n: hidden }) : "";
  $("#hist-empty").hidden = rows.length > 0;
  $("#hist-empty").textContent = histDiff.changes.length ? t("hist.emptyFilter") : t("hist.noChange");
}

/** Ô chọn mốc để so: mọi mốc trừ mốc mới nhất (mốc mới nhất luôn là vế "đến"). */
function renderHistPicker() {
  const sel = $("#hist-from");
  const prev = sel.value;
  sel.innerHTML = "";
  for (const s of histSnaps.slice(0, -1).reverse()) {
    const o = document.createElement("option");
    o.value = String(s.at);
    o.textContent = `${fmtStamp(s.at)} · ${fmtBytes(s.bytes)}`;
    sel.appendChild(o);
  }
  sel.value = prev && [...sel.options].some((o) => o.value === prev) ? prev : sel.options[0]?.value || "";
  sel.disabled = histSnaps.length < 2;
}

async function loadHistoryDiff() {
  if (histSnaps.length < 2) return;
  const from = Number($("#hist-from").value);
  const to = histSnaps[histSnaps.length - 1].at;
  try {
    histDiff = await invoke("history_diff", { from, to });
  } catch (err) {
    histDiff = null;
    toast(String(err) === "snapshot-missing" ? t("hist.errMissing") : String(err));
  }
  renderHistory();
}

async function loadHistory() {
  histLoading = true;
  renderHistory();
  try {
    histSnaps = await invoke("history_list");
  } catch (err) {
    histSnaps = [];
    toast(String(err));
  }
  histLoading = false;
  histLoaded = true;
  renderHistPicker();
  setTabBadge("history", 0);
  await Promise.all([loadHistoryDiff(), loadCleanLedger()]);
}

/** Ghi một mốc sau khi quét xong trọn vẹn. Gọi từ slclean-app.js, lỗi thì im lặng bỏ qua. */
async function recordSnapshot() {
  const items = [...snapshotItems()].map((i) => ({
    id: i.id,
    vi: typeof i.label === "string" ? i.label : i.label.vi,
    en: typeof i.label === "string" ? i.label : i.label.en,
    group: i.group,
    bytes: i.bytes,
  }));
  if (!items.length) return;
  try {
    await invoke("record_snapshot", { items });
    // Tab có thể đang mở: nạp lại để thấy mốc vừa ghi.
    if (tabShown.has("history")) await loadHistory();
  } catch {
    /* nhật ký hỏng không được làm hỏng lần quét */
  }
}

/** Mục để ghi vào mốc: cache + artifact đã đo xong, bỏ mục của tab khác và mục rỗng. */
function snapshotItems() {
  return [...items.values()].filter((i) => i.group !== "leftover" && !i.measuring && !i.gone && i.bytes > 0);
}

$("#hist-from").addEventListener("change", loadHistoryDiff);
if ($("#btn-ledger-csv")) {
  $("#btn-ledger-csv").addEventListener("click", () => {
    const runs = (cleanLedger && cleanLedger.runs) || [];
    const escCsv = (v) => `"${String(v ?? "").replaceAll('"', '""')}"`;
    const lines = ["run_id,timestamp,mode,source,total_freed_bytes,total_removed,total_skipped,recoverable"];
    for (const r of runs) {
      lines.push([r.run_id, r.timestamp, r.to_trash ? "trash" : "permanent", r.source || "ui", r.total_freed || 0, r.total_removed || 0, r.total_skipped || 0, !!r.recoverable_via_trash].map(escCsv).join(","));
    }
    if (typeof downloadBlob === "function") {
      downloadBlob("slclean-clean-ledger.csv", "text/csv;charset=utf-8", lines.join("\n"));
      toast("Exported slclean-clean-ledger.csv");
    }
  });
}
for (const b of document.querySelectorAll("[data-hist-filter]")) {
  b.addEventListener("click", () => { histFilter = b.dataset.histFilter; renderHistory(); });
}

onTabFirstShow("history", loadHistory);
