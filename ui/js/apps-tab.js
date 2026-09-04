// Tab Ứng dụng: danh sách app desktop + Store với lần mở cuối (Windows ghi nhận), ngày cài,
// dung lượng thư mục cài; hành động gỡ (trình gỡ của hãng / Store) và xoá mục đăng ký chết.
// Dữ liệu ở `apps` (Map id → AppInfo + measuring); DOM vẽ lại từ đó.
"use strict";

/** id → AppInfo từ backend, thêm `measuring`, `gone`. */
const apps = new Map();
let appsScanning = false;
let appsSort = localStorage.getItem("slclean-apps-sort") || "unused";
let appsFilter = "all";
let appsQuery = "";
const appsStats = { total: 0, done: 0 };

const AVATAR_COLORS = ["#8fb996", "#e2a83b", "#d9a5a5", "#9fb7d9", "#c9b6e0", "#b9c98f", "#e0b48f", "#8fc9c2"];
function avatarColor(name) {
  let h = 0;
  for (const c of name) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return AVATAR_COLORS[h % AVATAR_COLORS.length];
}

function appMatches(a) {
  if (!appsQuery) return true;
  return [a.name, a.publisher, a.install_dir || ""].some((s) => String(s).toLowerCase().includes(appsQuery));
}

/** App bình thường mà người dùng có thể cân nhắc gỡ (không phải mục chết, không phải thành phần nền). */
function isUninstallCandidate(a) {
  return !a.dead && !a.system_component;
}

function appPasses(a) {
  if (a.gone) return false;
  if (!appMatches(a)) return false;
  switch (appsFilter) {
    case "dead": return a.dead;
    case "never": return isUninstallCandidate(a) && a.usage_known && a.last_used === 0 && !a.running;
    case "component": return a.system_component;
    case "desktop": return a.kind === "desktop";
    case "store": return a.kind === "store";
    default: return true;
  }
}

function appCompare(x, y) {
  switch (appsSort) {
    case "size": return y.bytes - x.bytes || x.name.localeCompare(y.name);
    case "installed": return y.installed - x.installed || x.name.localeCompare(y.name);
    case "name": return x.name.localeCompare(y.name, undefined, { sensitivity: "base" });
    default: {
      // Lâu không mở: mục chết trước, rồi chưa từng mở, rồi không rõ (không có thư mục để đối
      // chiếu), rồi lần mở cũ nhất, rồi đang chạy; thành phần nền xuống cuối vì không phải ứng
      // viên để gỡ. Cùng hạng thì lớn trước.
      const rank = (a) => (a.dead ? 0 : a.system_component ? 5 : a.running ? 4 : !a.usage_known ? 2 : a.last_used === 0 ? 1 : 3);
      return rank(x) - rank(y) || (x.last_used - y.last_used) || (y.bytes - x.bytes);
    }
  }
}

function lastUsedCol(a) {
  if (a.running) return `<span class="app__col app__col--fresh"><small>${t("apps.lastUsedLabel")}</small><b>${t("apps.running")}</b></span>`;
  // Thành phần nền không có gì để "mở", nên "chưa từng" ở đây là vô nghĩa.
  if (a.system_component && !a.last_used) return `<span class="app__col app__col--unknown"><small>${t("apps.lastUsedLabel")}</small><b>—</b></span>`;
  if (!a.usage_known) return `<span class="app__col app__col--unknown" title="${esc(t("apps.unknownTitle"))}"><small>${t("apps.lastUsedLabel")}</small><b>${t("apps.unknown")}</b></span>`;
  if (!a.last_used) return `<span class="app__col app__col--never"><small>${t("apps.lastUsedLabel")}</small><b>${t("apps.never")}</b></span>`;
  const stale = isStale(a.last_used);
  return `<span class="app__col ${stale ? "app__col--stale" : ""}"><small>${t("apps.lastUsedLabel")}</small><b>${esc(fmtAge(a.last_used))}</b></span>`;
}

function installedCol(a) {
  return `<span class="app__col"><small>${t("apps.installedLabel")}</small><b>${a.installed ? esc(fmtAge(a.installed)) : "—"}</b></span>`;
}

function appSizeText(a) {
  if (a.measuring) return a.bytes ? `${fmtBytes(a.bytes)}…` : "…";
  if (!a.folder_exists) return a.dead ? "—" : a.bytes ? fmtBytes(a.bytes) : "—";
  return fmtBytes(a.bytes);
}

function appRow(a) {
  const li = document.createElement("li");
  li.className = "app";
  li.dataset.app = a.id;
  li.classList.toggle("app--dead", a.dead);
  li.classList.toggle("app--measuring", !!a.measuring);
  li.style.setProperty("--av", avatarColor(a.name));
  const tags = [];
  if (a.dead) tags.push(`<b class="tag tag--dead" title="${esc(t("apps.deadTitle"))}">${t("apps.dead")}</b>`);
  if (a.system_component) tags.push(`<b class="tag tag--component" title="${esc(t("apps.componentTitle"))}">${t("apps.component")}</b>`);
  if (a.kind === "store") tags.push(`<b class="tag tag--store">Store</b>`);
  if (a.running) tags.push(`<b class="tag tag--running">${t("apps.running")}</b>`);
  if (a.denied > 0) tags.push(`<b class="tag tag--partial" title="${esc(t("tag.partialTitle"))}">${t("tag.partial")}</b>`);
  const meta = [a.publisher, a.version ? `<span class="ver">${esc(a.version)}</span>` : ""].filter(Boolean).map((s, i) => (i === 0 ? esc(s) : s)).join(" · ");
  const pathText = a.install_dir ? shortPath(a.install_dir) : "";
  const path = a.install_dir
    ? `<div class="app__path" title="${esc(a.install_dir)}"><bdi>${esc(pathText)}</bdi>${a.dead && !a.folder_exists ? ` <span class="row__multi">· ${t("apps.folderMissing")}</span>` : ""}</div>`
    : "";
  let action;
  if (a.dead) {
    action = a.needs_admin
      ? `<button class="btn btn--sm btn--line" disabled title="${esc(t("tag.adminTitle"))}">${t("apps.needsAdmin")}</button>`
      : `<button class="btn btn--sm btn--danger-soft" data-act="dead">${t("apps.removeEntry")}</button>`;
  } else {
    action = `<button class="btn btn--sm btn--line" data-act="uninstall" ${a.running ? `disabled title="${esc(t("apps.runningTitle"))}"` : ""}>${t("apps.uninstall")}</button>`;
  }
  const sizeTitle = !a.measured && !a.measuring && a.bytes ? t("apps.estimate") : "";
  li.innerHTML = `
    <span class="app__avatar" aria-hidden="true">${esc((a.name.trim()[0] || "?").toUpperCase())}</span>
    <div class="app__main">
      <div class="app__title"><span class="app__name">${esc(a.name)}</span>${tags.join("")}</div>
      ${meta ? `<div class="app__meta">${meta}</div>` : ""}
      ${path}
    </div>
    ${lastUsedCol(a)}
    ${installedCol(a)}
    <span class="app__size ${sizeTitle ? "app__size--estimate" : ""}" title="${esc(sizeTitle)}">${appSizeText(a)}</span>
    <span class="app__actions">
      ${a.folder_exists ? `<button class="row__open" title="${esc(t("row.open"))}" aria-label="${esc(t("row.open"))}">${REVEAL_SVG}</button>` : ""}
      ${action}
    </span>`;
  li.querySelector(".row__open")?.addEventListener("click", () => revealItemInDir(a.install_dir).catch(() => toast(t("toast.openFail"))));
  li.querySelector('[data-act="uninstall"]')?.addEventListener("click", () => uninstallApp(a));
  li.querySelector('[data-act="dead"]')?.addEventListener("click", () => removeDeadApp(a));
  return li;
}

function renderApps() {
  const list = $("#apps-list");
  list.innerHTML = "";
  const rows = [...apps.values()].filter(appPasses).sort(appCompare);
  for (const a of rows) list.appendChild(appRow(a));
  $("#apps-empty").hidden = rows.length > 0 || appsScanning;
  $("#apps-empty").textContent = apps.size ? t("apps.emptyFilter") : t("apps.empty");
  for (const b of document.querySelectorAll("[data-apps-sort]")) b.classList.toggle("is-active", b.dataset.appsSort === appsSort);
  for (const b of document.querySelectorAll("[data-apps-filter]")) b.classList.toggle("is-active", b.dataset.appsFilter === appsFilter);
  refreshAppsTotals();
}

function refreshAppsTotals() {
  let bytes = 0, dead = 0, never = 0, deadRemovable = 0, live = 0, components = 0;
  for (const a of apps.values()) {
    if (a.gone) continue;
    live++;
    bytes += a.folder_exists ? a.bytes : 0;
    if (a.dead) { dead++; if (!a.needs_admin) deadRemovable++; }
    else if (a.system_component) components++;
    else if (a.usage_known && a.last_used === 0 && !a.running) never++;
  }
  $("#apps-hero").textContent = apps.size ? fmtBytes(bytes) : "—";
  if (!appsScanning) {
    const sum = t("apps.summary", { n: live, dead, never });
    $("#apps-sub").textContent = apps.size ? (components ? `${sum} · ${t("apps.summaryComponents", { n: components })}` : sum) : t("apps.subIdle");
  }
  const hidden = [...apps.values()].filter((a) => !a.gone && !appPasses(a)).length;
  $("#apps-hint").textContent = hidden ? t("apps.hiddenHint", { n: hidden }) : "";
  const bulk = $("#btn-remove-dead");
  bulk.hidden = deadRemovable === 0;
  bulk.textContent = t("apps.removeDeadAll", { n: deadRemovable });
  setTabBadge("apps", dead, true);
}

function paintAppsProgress() {
  const bar = $("#apps-scanbar");
  bar.hidden = !appsScanning;
  if (!appsScanning) return;
  const pct = appsStats.total ? (appsStats.done / appsStats.total) * 100 : 0;
  $("#apps-scanbar-fill").style.setProperty("--pct", `${pct.toFixed(1)}%`);
  $("#apps-sub").textContent = t("apps.measuring", { done: appsStats.done, total: appsStats.total });
}

/** Cập nhật số đo một app tại chỗ; xong thì vẽ lại hàng (đổi thứ tự nếu sắp theo cỡ). */
function applyAppSize(d) {
  const a = apps.get(d.id);
  if (!a) return;
  a.bytes = d.bytes;
  a.files = d.files;
  a.denied = d.denied;
  if (d.done) {
    a.measuring = false;
    a.measured = true;
    appsStats.done++;
    paintAppsProgress();
    const li = document.querySelector(`[data-app="${CSS.escape(d.id)}"]`);
    if (li && appPasses(a)) li.replaceWith(appRow(a));
    refreshAppsTotals();
    return;
  }
  const li = document.querySelector(`[data-app="${CSS.escape(d.id)}"]`);
  if (li) li.querySelector(".app__size").textContent = appSizeText(a);
}

async function scanApps() {
  if (appsScanning) return;
  appsScanning = true;
  apps.clear();
  Object.assign(appsStats, { total: 0, done: 0 });
  $("#btn-apps-scan").disabled = true;
  renderApps();
  paintAppsProgress();
  const unList = await listen("apps-list", (e) => {
    for (const a of e.payload) apps.set(a.id, { ...a, measuring: a.folder_exists, gone: false });
    appsStats.total = e.payload.filter((a) => a.folder_exists).length;
    renderApps();
    paintAppsProgress();
  });
  const unSize = await listen("app-size", (e) => applyAppSize(e.payload));
  try {
    const final = await invoke("scan_apps");
    for (const a of final) {
      const cur = apps.get(a.id);
      if (cur) Object.assign(cur, a, { measuring: false });
    }
  } catch (err) {
    toast(String(err));
  } finally {
    unList();
    unSize();
    appsScanning = false;
    for (const a of apps.values()) a.measuring = false;
    $("#btn-apps-scan").disabled = false;
    paintAppsProgress();
    renderApps();
  }
}

function markAppGone(a) {
  a.gone = true;
  a.checked = false;
  const li = document.querySelector(`[data-app="${CSS.escape(a.id)}"]`);
  if (li) li.classList.add("app--gone");
  refreshAppsTotals();
}

/** Thư mục cài còn sót sau khi gỡ: hỏi rồi xoá thẳng qua lệnh clean. */
async function offerLeftoverDir(a, dir) {
  const r = await ask({ title: t("ask.leftoverTitle"), body: t("ask.leftoverBody", { name: a.name }), path: dir, ok: t("ask.leftoverOk") });
  if (!r.ok) return;
  try {
    const res = await invoke("clean", { items: [{ id: a.id, paths: [dir], keep_root: false }], toTrash: false });
    const one = res[0] || {};
    toast(one.error ? errorText(one.error) : t("toast.leftoverDeleted", { b: fmtBytes(one.freed || 0) }));
    loadDrives();
  } catch (err) {
    toast(String(err));
  }
}

async function uninstallApp(a) {
  const r = await ask({
    title: t("ask.uninstallTitle", { name: a.name }),
    body: a.kind === "store" ? t("ask.uninstallStoreBody") : t("ask.uninstallBody"),
    path: a.install_dir || "",
    ok: t("ask.uninstallOk"),
  });
  if (!r.ok) return;
  const close = showBusy(t("progress.uninstalling", { name: a.name }));
  try {
    const out = await invoke("uninstall_app", { id: a.id });
    close();
    if (out.gone) {
      markAppGone(a);
      toast(t("toast.uninstalled", { name: a.name }));
      loadDrives();
      if (out.leftover_dir) await offerLeftoverDir(a, out.leftover_dir);
    } else {
      toast(t("toast.notGone", { name: a.name }));
    }
  } catch (err) {
    close();
    toast(actionErrorText(err));
  }
}

async function removeDeadApp(a) {
  const r = await ask({
    title: t("ask.deadTitle", { name: a.name }),
    body: t("ask.deadBody"),
    path: a.install_dir || "",
    option: a.folder_exists ? t("ask.deadOption") : "",
    optionDefault: a.folder_exists,
    ok: t("ask.deadOk"),
  });
  if (!r.ok) return;
  try {
    const out = await invoke("remove_dead_app", { id: a.id, deleteFolder: a.folder_exists && r.option });
    if (out.gone) markAppGone(a);
    toast(out.freed ? t("toast.deadRemovedFreed", { name: a.name, b: fmtBytes(out.freed) }) : t("toast.deadRemoved", { name: a.name }));
    if (out.freed) loadDrives();
  } catch (err) {
    toast(actionErrorText(err));
  }
}

/** Xoá mọi mục chết xoá được (không cần admin) trong một lượt. */
async function removeAllDead() {
  const targets = [...apps.values()].filter((a) => a.dead && !a.gone && !a.needs_admin);
  const skipped = [...apps.values()].filter((a) => a.dead && !a.gone && a.needs_admin).length;
  if (!targets.length) return;
  const r = await ask({
    title: t("ask.deadAllTitle", { n: targets.length }),
    body: t("ask.deadAllBody") + (skipped ? " " + t("ask.deadAllSkipped", { n: skipped }) : ""),
    option: t("ask.deadOption"),
    optionDefault: true,
    ok: t("ask.deadOk"),
  });
  if (!r.ok) return;
  const close = showBusy(t("progress.removingDead"));
  let done = 0, freed = 0, failed = 0;
  for (const a of targets) {
    try {
      const out = await invoke("remove_dead_app", { id: a.id, deleteFolder: a.folder_exists && r.option });
      if (out.gone) markAppGone(a);
      freed += out.freed || 0;
      done++;
    } catch {
      failed++;
    }
  }
  close();
  toast(t("toast.deadRemovedN", { n: done, b: fmtBytes(freed) }) + (failed ? " · " + t("toast.failedN", { n: failed }) : ""));
  if (freed) loadDrives();
}

$("#btn-apps-scan").addEventListener("click", scanApps);
$("#btn-remove-dead").addEventListener("click", removeAllDead);
for (const b of document.querySelectorAll("[data-apps-sort]")) {
  b.addEventListener("click", () => { appsSort = b.dataset.appsSort; localStorage.setItem("slclean-apps-sort", appsSort); renderApps(); });
}
for (const b of document.querySelectorAll("[data-apps-filter]")) {
  b.addEventListener("click", () => { appsFilter = b.dataset.appsFilter; renderApps(); });
}
let appsSearchTimer;
$("#apps-search").addEventListener("input", (e) => {
  clearTimeout(appsSearchTimer);
  appsSearchTimer = setTimeout(() => { appsQuery = e.target.value.trim().toLowerCase(); renderApps(); }, 80);
});
$("#apps-search").addEventListener("keydown", (e) => { if (e.key === "Escape") { e.target.value = ""; appsQuery = ""; renderApps(); } });

onTabFirstShow("apps", scanApps);
