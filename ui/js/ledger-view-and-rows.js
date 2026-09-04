// Vẽ sổ cái: tạo hàng, hai cách xem (phẳng theo dung lượng / theo nhóm), bộ lọc an toàn,
// ô tìm kiếm, và cập nhật tổng. Dữ liệu nguồn là `items` (Map id → mục); DOM luôn vẽ lại từ đó.
// File này nạp trước slclean-app.js; hai file dùng chung biến toàn cục.
"use strict";

const $ = (sel) => document.querySelector(sel);
const GROUPS = ["artifacts", "ai", "package", "editor", "browser", "app", "game", "system", "temp"];

/** id → { id, group, label, note, path, paths, bytes, safety, keepRoot, needsAdmin, checked, gone, partial, age, measuring } */
const items = new Map();
let scanning = false;
/** Tab Thư mục thừa đang quét (mục group "leftover" nằm chung trong `items`). */
let leftScanning = false;
/** App có đang chạy với quyền admin không; set một lần lúc khởi động. */
let elevated = false;

/** Cách xem: "size" (mọi mục, lớn nhất trước) hoặc "group" (theo nhóm). */
let view = localStorage.getItem("slclean-view") || "size";
/** Bộ lọc an toàn: "all" | "safe" | "safe-rebuild". */
let filter = localStorage.getItem("slclean-filter") || "all";
/** Chuỗi tìm kiếm (chữ thường); rỗng = không lọc. */
let query = "";

function matchesQuery(it) {
  if (!query) return true;
  return L(it.label).toLowerCase().includes(query) || String(it.path).toLowerCase().includes(query) || L(it.note).toLowerCase().includes(query);
}

/** Mục có qua được bộ lọc hiện tại không (mục đã tick luôn hiện để không "mất dấu" lựa chọn). */
function passesFilter(it) {
  if (it.checked) return true;
  // Thư mục thừa có bộ lọc/tìm kiếm riêng của tab nó; mục rỗng vẫn hiện vì tự thân thư mục là rác.
  if (it.group === "leftover") return leftoverPasses(it);
  // Mục rỗng không dọn được gì: ẩn cho gọn, trừ khi rỗng vì không đọc được (admin có thể thấy thêm).
  if (it.bytes === 0 && !it.measuring && !it.partial) return false;
  if (!matchesQuery(it)) return false;
  if (filter === "safe") return it.safety === "safe";
  if (filter === "safe-rebuild") return it.safety === "safe" || it.safety === "rebuild";
  return true;
}

/** Mục bị khoá tick vì cần admin mà app đang chạy thường. */
function lockedByAdmin(it) {
  return it.needsAdmin && !elevated;
}

/** Không tick được: đang đo, rỗng (trừ thư mục thừa), khoá admin. */
function unselectable(it) {
  return it.measuring || (it.bytes === 0 && it.group !== "leftover") || lockedByAdmin(it);
}

function containerFor(it) {
  if (it.group === "leftover") return $("#left-list");
  return view === "size" ? $("#rows-flat") : $(`#rows-${it.group}`);
}

const REVEAL_SVG = `<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5"><path d="M2 4.5A1.5 1.5 0 0 1 3.5 3h3l1.5 1.5h4.5A1.5 1.5 0 0 1 14 6v6.5a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 12.5z"/></svg>`;

function sizeText(it) {
  if (it.measuring) return it.bytes ? `${fmtBytes(it.bytes)}…` : "…";
  return fmtBytes(it.bytes);
}

function rowFor(it) {
  const li = document.createElement("li");
  li.className = "row";
  li.dataset.id = it.id;
  const locked = lockedByAdmin(it);
  const tag =
    it.safety === "review" ? `<b class="tag tag--review">${t("tag.review")}</b>` :
    it.safety === "rebuild" ? `<b class="tag tag--rebuild">${t("tag.rebuild")}</b>` : "";
  const admin = locked ? `<b class="tag tag--admin" title="${esc(t("tag.adminTitle"))}">${t("tag.admin")}</b>` : "";
  const partial = it.partial ? `<b class="tag tag--partial" title="${esc(t("tag.partialTitle"))}">${t("tag.partial")}</b>` : "";
  const measuring = it.measuring ? `<b class="tag tag--measuring">${t("tag.measuring")}</b>` : "";
  const multi = it.paths.length > 1 ? `<span class="row__multi">· ${t("row.paths", { n: it.paths.length })}</span>` : "";
  const extra = (it.extraTags || []).join("");
  // Thư mục thừa: cột tuổi hiện lần mở cuối nếu Windows từng thấy exe trong đó chạy, không thì lần ghi cuối.
  const age = it.group === "leftover"
    ? (it.lastUsed
      ? `<span class="row__age row__age--used" title="${esc(t("left.lastUsedTitle"))}">${t("left.lastUsed", { a: fmtAge(it.lastUsed) })}</span>`
      : `<span class="row__age ${isStale(it.age) ? "row__age--stale" : ""}" title="${esc(t("left.modifiedTitle"))}">${fmtAge(it.age) || ""}</span>`)
    : it.age != null ? `<span class="row__age ${isStale(it.age) ? "row__age--stale" : ""}">${fmtAge(it.age)}</span>` : "<span></span>";
  const note = L(it.note);
  li.innerHTML = `
    <label class="check"><input type="checkbox" ${it.checked ? "checked" : ""} ${unselectable(it) ? "disabled" : ""}><span class="check__box"></span></label>
    <div class="row__main">
      <div class="row__title"><span class="row__label">${esc(L(it.label))}</span>${tag}${extra}${admin}${partial}${measuring}</div>
      <div class="row__path" title="${esc(it.paths.join("\n"))}"><bdi>${esc(shortPath(it.path))}</bdi>${multi}</div>
      ${note ? `<div class="row__note">${esc(note)}</div>` : ""}
      <div class="row__bar"></div>
    </div>
    ${age}
    <span class="row__size">${sizeText(it)}</span>
    <button class="row__open" title="${esc(t("row.open"))}" aria-label="${esc(t("row.open"))}">${REVEAL_SVG}</button>`;
  li.classList.toggle("row--checked", it.checked);
  li.classList.toggle("row--zero", it.bytes === 0 && !it.measuring && it.group !== "leftover");
  li.classList.toggle("row--measuring", !!it.measuring);
  li.classList.toggle("row--gone", !!it.gone);
  const input = li.querySelector("input");
  input.addEventListener("change", () => {
    it.checked = input.checked;
    li.classList.toggle("row--checked", it.checked);
    refreshTotals();
  });
  // Bấm bất kỳ đâu trên hàng cũng tick, trừ nút mở Explorer và chính ô tick.
  li.addEventListener("click", (e) => {
    if (e.target.closest(".row__open") || e.target.closest(".check")) return;
    if (input.disabled) return;
    input.checked = !input.checked;
    input.dispatchEvent(new Event("change"));
  });
  li.querySelector(".row__open").addEventListener("click", () => revealItemInDir(it.path).catch(() => toast(t("toast.openFail"))));
  return li;
}

/** Thêm/cập nhật một mục và đặt nó vào đúng chỗ theo cách xem + bộ lọc hiện tại. */
function upsert(it) {
  items.set(it.id, it);
  const existing = $("#board").querySelector(`[data-id="${CSS.escape(it.id)}"]`);
  if (existing) existing.remove();
  if (passesFilter(it)) {
    const list = containerFor(it);
    const row = rowFor(it);
    // Chèn theo thứ tự giảm dần dung lượng để mục lớn luôn ở trên.
    const after = [...list.children].find((c) => (items.get(c.dataset.id)?.bytes ?? 0) < it.bytes);
    list.insertBefore(row, after || null);
  }
  refreshTotals();
}

/** Cập nhật số đang đo của một hàng tại chỗ, không vẽ lại/sắp xếp lại. */
function updateMeasuring(id, bytes, files) {
  const it = items.get(id);
  if (!it || !it.measuring) return;
  it.bytes = bytes;
  it.files = files;
  const li = $("#board").querySelector(`[data-id="${CSS.escape(id)}"]`);
  if (li) li.querySelector(".row__size").textContent = sizeText(it);
}

/** Vẽ lại toàn bộ (khi đổi cách xem / bộ lọc / tìm kiếm / ngôn ngữ / trạng thái admin). */
function renderAll() {
  $("#flat-view").hidden = view !== "size";
  for (const sec of document.querySelectorAll(".group[data-group]")) sec.hidden = view === "size";
  $("#rows-flat").innerHTML = "";
  for (const g of GROUPS) $(`#rows-${g}`).innerHTML = "";
  $("#left-list").innerHTML = "";
  const sorted = [...items.values()].sort((a, b) => b.bytes - a.bytes);
  let shown = 0;
  for (const it of sorted) {
    if (!passesFilter(it)) continue;
    containerFor(it).appendChild(rowFor(it));
    if (it.group !== "leftover") shown++;
  }
  $("#empty-flat").hidden = view !== "size" || shown > 0 || scanning;
  refreshTotals();
}

/** Cập nhật thanh cỡ, tổng nhóm, ô tick nhóm, tổng đã chọn và gợi ý bộ lọc. */
function refreshTotals() {
  const lists = view === "size" ? ["flat"] : GROUPS;
  for (const g of lists) {
    const list = $(`#rows-${g}`);
    const rows = [...list.children].map((li) => items.get(li.dataset.id)).filter((i) => i && !i.gone);
    const max = Math.max(0, ...rows.map((i) => i.bytes));
    for (const li of list.children) {
      const it = items.get(li.dataset.id);
      li.querySelector(".row__bar").style.setProperty("--rel", max ? `${((it.bytes / max) * 100).toFixed(1)}%` : "0%");
    }
    const total = rows.reduce((s, i) => s + i.bytes, 0);
    $(`[data-group-size="${g}"]`).textContent = rows.length ? fmtBytes(total) : "—";
    const gc = $(`[data-group-check="${g}"]`);
    const sel = rows.filter((i) => i.checked).length;
    const selectable = rows.filter((i) => !unselectable(i)).length;
    gc.checked = selectable > 0 && sel === selectable;
    gc.indeterminate = sel > 0 && sel < selectable;
    gc.disabled = selectable === 0;
    if (g !== "flat") $(`[data-group="${g}"]`).classList.toggle("group--empty", rows.length === 0 && g !== "artifacts");
  }
  // Thanh cỡ tương đối của danh sách thư mục thừa (tab riêng, không thuộc GROUPS).
  {
    const list = $("#left-list");
    const rows = [...list.children].map((li) => items.get(li.dataset.id)).filter((i) => i && !i.gone);
    const max = Math.max(0, ...rows.map((i) => i.bytes));
    for (const li of list.children) {
      const it = items.get(li.dataset.id);
      li.querySelector(".row__bar").style.setProperty("--rel", max ? `${((it.bytes / max) * 100).toFixed(1)}%` : "0%");
    }
  }
  let checkedBytes = 0, checkedCount = 0, foundBytes = 0, hiddenBytes = 0, hiddenCount = 0, zeroHidden = 0;
  for (const it of items.values()) {
    if (it.gone) continue;
    if (it.checked) { checkedBytes += it.bytes; checkedCount++; }
    if (it.group === "leftover") continue;
    foundBytes += it.bytes;
    if (!passesFilter(it) && it.bytes > 0) { hiddenBytes += it.bytes; hiddenCount++; }
    if (it.bytes === 0 && !it.measuring && !it.partial) zeroHidden++;
  }
  $("#tally-bytes").textContent = fmtBytes(checkedBytes);
  $("#tally-count").textContent = t(checkedCount === 1 ? "rail.selectedMetaOne" : "rail.selectedMeta", { n: checkedCount });
  $("#found-bytes").textContent = fmtBytes(foundBytes);
  $("#hero-bytes").textContent = fmtBytes(foundBytes);
  const hints = [];
  if (hiddenCount) hints.push(t("filter.hint", { n: hiddenCount, b: fmtBytes(hiddenBytes) }));
  if (zeroHidden) hints.push(t("filter.zero", { n: zeroHidden }));
  $("#filter-hint").textContent = hints.join(" · ");
  $("#btn-clean").disabled = checkedCount === 0 || scanning || leftScanning;
  setTabBadge("clean", checkedCount, false);
  paintGains();
  refreshLeftTotals();
}

/** Đồng bộ ô tick của mọi hàng đang hiển thị với trạng thái trong `items`. */
function syncVisibleRowChecks() {
  for (const li of $("#board").querySelectorAll("[data-id]")) {
    const it = items.get(li.dataset.id);
    if (!it) continue;
    li.querySelector("input").checked = it.checked;
    li.classList.toggle("row--checked", it.checked);
  }
}
