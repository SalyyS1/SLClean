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

function renderHistory() {
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
  await loadHistoryDiff();
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
for (const b of document.querySelectorAll("[data-hist-filter]")) {
  b.addEventListener("click", () => { histFilter = b.dataset.histFilter; renderHistory(); });
}

onTabFirstShow("history", loadHistory);
