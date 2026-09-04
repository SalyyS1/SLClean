// Tab Khởi động: mục tự chạy cùng Windows (Run, thư mục Startup, scheduled task), kèm lần mở
// cuối của exe để trả lời câu không tool nào trả lời: "tự chạy cùng máy nhưng bạn không mở nó
// tám tháng rồi". Công tắc là tắt/bật, không xoá — trừ mục hỏng (lệnh trỏ tới file không còn),
// thứ duy nhất xoá được ở đây.
"use strict";

/** id → StartupEntry từ backend, thêm `gone` (đã xoá trong phiên này). */
const startups = new Map();
let startScanning = false;
let startFilter = "all";
let startQuery = "";

function startMatches(e) {
  if (!startQuery) return true;
  return [e.name, e.command].some((s) => String(s).toLowerCase().includes(startQuery));
}

/** Đang bật, có bản ghi mở, và lần mở gần nhất đã quá 30 ngày. */
function isStaleStartup(e) {
  return e.enabled && !e.broken && e.usage_known && isStale(e.last_used);
}

function startPasses(e) {
  if (e.gone) return false;
  if (!startMatches(e)) return false;
  switch (startFilter) {
    case "on": return e.enabled;
    case "stale": return isStaleStartup(e);
    case "broken": return e.broken;
    default: return true;
  }
}

const SOURCE_KEY = { run: "start.srcRun", runmachine: "start.srcRunMachine", folder: "start.srcFolder", foldermachine: "start.srcFolderMachine", task: "start.srcTask" };

/** Cột "lần mở cuối" của exe mục này chạy. */
function startUsedCol(e) {
  if (e.running) return `<span class="app__col app__col--fresh"><small>${t("start.lastUsedLabel")}</small><b>${t("apps.running")}</b></span>`;
  if (e.broken) return `<span class="app__col app__col--unknown"><small>${t("start.lastUsedLabel")}</small><b>—</b></span>`;
  if (!e.usage_known) return `<span class="app__col app__col--unknown" title="${esc(t("start.unknownTitle"))}"><small>${t("start.lastUsedLabel")}</small><b>${t("apps.unknown")}</b></span>`;
  if (!e.last_used) return `<span class="app__col app__col--never"><small>${t("start.lastUsedLabel")}</small><b>${t("apps.never")}</b></span>`;
  return `<span class="app__col ${isStale(e.last_used) ? "app__col--stale" : ""}"><small>${t("start.lastUsedLabel")}</small><b>${esc(fmtAge(e.last_used))}</b></span>`;
}

function startRow(e) {
  const li = document.createElement("li");
  li.className = "boot";
  li.dataset.start = e.id;
  li.classList.toggle("boot--off", !e.enabled);
  const tags = [`<b class="tag tag--src">${t(SOURCE_KEY[e.source] || "start.srcRun")}</b>`];
  if (e.broken) tags.push(`<b class="tag tag--dead" title="${esc(t("start.brokenTitle"))}">${t("start.broken")}</b>`);
  if (e.system) tags.push(`<b class="tag tag--component" title="${esc(t("start.systemTitle"))}">${t("start.system")}</b>`);
  if (e.needs_admin) tags.push(`<b class="tag tag--admin" title="${esc(t("tag.adminTitle"))}">${t("tag.admin")}</b>`);
  // Người dùng phải thấy đúng dòng lệnh sẽ chạy trước khi quyết định tắt.
  const cmd = shortPath(e.command);
  li.innerHTML = `
    <div class="boot__main">
      <div class="app__title"><span class="app__name">${esc(e.name)}</span>${tags.join("")}</div>
      <div class="boot__cmd" title="${esc(e.command)}"><bdi>${esc(cmd)}</bdi></div>
    </div>
    ${startUsedCol(e)}
    <span class="boot__actions">
      ${e.broken ? `<button class="btn btn--sm btn--danger-soft" data-act="remove">${t("start.remove")}</button>` : ""}
      <label class="switch" title="${esc(t(e.enabled ? "start.toggleOff" : "start.toggleOn"))}">
        <input type="checkbox" data-act="toggle" ${e.enabled ? "checked" : ""}>
        <span class="switch__track" aria-hidden="true"></span>
        <span class="switch__label">${t(e.enabled ? "start.on" : "start.off")}</span>
      </label>
    </span>`;
  li.querySelector('[data-act="toggle"]').addEventListener("change", (ev) => toggleStartup(e, ev.target.checked));
  li.querySelector('[data-act="remove"]')?.addEventListener("click", () => removeStartup(e));
  return li;
}

function renderStartup() {
  const list = $("#start-list");
  list.innerHTML = "";
  const rows = [...startups.values()].filter(startPasses);
  for (const e of rows) list.appendChild(startRow(e));
  $("#start-empty").hidden = rows.length > 0 || startScanning;
  $("#start-empty").textContent = startups.size ? t("start.emptyFilter") : t("start.empty");
  for (const b of document.querySelectorAll("[data-start-filter]")) b.classList.toggle("is-active", b.dataset.startFilter === startFilter);
  refreshStartupTotals();
}

function refreshStartupTotals() {
  let on = 0, stale = 0, broken = 0;
  for (const e of startups.values()) {
    if (e.gone) continue;
    if (e.enabled) on++;
    if (isStaleStartup(e)) stale++;
    if (e.broken) broken++;
  }
  $("#start-hero").textContent = startups.size ? String(on) : "—";
  if (!startScanning) {
    $("#start-sub").textContent = startups.size ? t("start.summary", { on, stale, broken }) : t("start.subIdle");
  }
  const hidden = [...startups.values()].filter((e) => !e.gone && !startPasses(e)).length;
  $("#start-hint").textContent = hidden ? t("start.hiddenHint", { n: hidden }) : "";
  // Huy hiệu đếm mục hỏng: đó là thứ duy nhất ở tab này cần dọn hẳn.
  setTabBadge("startup", broken, true);
}

async function scanStartup() {
  if (startScanning) return;
  startScanning = true;
  startups.clear();
  $("#btn-start-scan").disabled = true;
  $("#start-scanbar").hidden = false;
  $("#start-sub").textContent = t("start.scanning");
  renderStartup();
  try {
    for (const e of await invoke("scan_startup")) startups.set(e.id, { ...e, gone: false });
  } catch (err) {
    toast(String(err));
  } finally {
    startScanning = false;
    $("#btn-start-scan").disabled = false;
    $("#start-scanbar").hidden = true;
    renderStartup();
  }
}

/** Lỗi từ backend là mã ngắn; dịch sang câu người đọc được. */
function startupErrorText(err) {
  const code = String(err);
  if (code === "needs-admin") return t("start.errAdmin");
  if (code === "task-failed") return t("start.errTask");
  if (code === "bad-id") return t("start.errBadId");
  return code;
}

/** Vẽ lại một hàng tại chỗ, hoặc bỏ khỏi danh sách nếu bộ lọc không còn nhận nó. */
function repaintStartRow(e) {
  const li = document.querySelector(`[data-start="${CSS.escape(e.id)}"]`);
  if (!li) return;
  if (startPasses(e)) li.replaceWith(startRow(e));
  else li.remove();
}

async function toggleStartup(e, want) {
  try {
    await invoke("set_startup_enabled", { id: e.id, enabled: want });
    e.enabled = want;
    toast(t(want ? "start.toastOn" : "start.toastOff", { name: e.name }));
  } catch (err) {
    toast(startupErrorText(err));
  }
  // Dù thành hay bại đều vẽ lại: thất bại thì công tắc phải bật về trạng thái thật.
  repaintStartRow(e);
  refreshStartupTotals();
}

async function removeStartup(e) {
  const r = await ask({
    title: t("start.askRemoveTitle", { name: e.name }),
    body: t("start.askRemoveBody"),
    path: e.command,
    ok: t("start.askRemoveOk"),
  });
  if (!r.ok) return;
  try {
    await invoke("remove_startup_entry", { id: e.id });
    e.gone = true;
    document.querySelector(`[data-start="${CSS.escape(e.id)}"]`)?.remove();
    toast(t("start.toastRemoved", { name: e.name }));
    refreshStartupTotals();
  } catch (err) {
    toast(startupErrorText(err));
  }
}

$("#btn-start-scan").addEventListener("click", scanStartup);
$("#start-search").addEventListener("input", (ev) => { startQuery = ev.target.value.trim().toLowerCase(); renderStartup(); });
for (const b of document.querySelectorAll("[data-start-filter]")) {
  b.addEventListener("click", () => { startFilter = b.dataset.startFilter; renderStartup(); });
}

onTabFirstShow("startup", scanStartup);
