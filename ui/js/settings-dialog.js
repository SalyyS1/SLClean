// Hộp cài đặt: ngôn ngữ, thư mục dự án thêm/loại trừ (qua hộp chọn thư mục của Windows),
// chế độ Thùng rác mặc định. Lưu qua lệnh save_settings; đổi thư mục thì nhắc quét lại.
"use strict";

/** Bản nháp đang sửa trong hộp thoại; chỉ ghi ra khi bấm Lưu. */
let draft = null;
let rootsInfo = { discovered: [], extra: [], excluded: [] };

function chip(path, removable, onRemove) {
  const li = document.createElement("li");
  li.className = "chip";
  li.innerHTML = `<span class="chip__path mono" title="${esc(path)}">${esc(path)}</span>`;
  if (removable) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "chip__remove";
    b.title = t("settings.remove");
    b.setAttribute("aria-label", t("settings.remove"));
    b.textContent = "×";
    b.addEventListener("click", onRemove);
    li.appendChild(b);
  }
  return li;
}

function renderList(sel, paths, removable, onRemove) {
  const ul = $(sel);
  ul.innerHTML = "";
  if (!paths.length) {
    const li = document.createElement("li");
    li.className = "chip chip--none";
    li.textContent = t("settings.none");
    ul.appendChild(li);
    return;
  }
  for (const p of paths) ul.appendChild(chip(p, removable, () => onRemove(p)));
}

function renderSettings() {
  for (const r of document.querySelectorAll('input[name="lang"]')) r.checked = r.value === (draft.language || "system");
  renderList("#roots-discovered", rootsInfo.discovered, false);
  renderList("#roots-extra", draft.extra_roots, true, (p) => { draft.extra_roots = draft.extra_roots.filter((x) => x !== p); renderSettings(); });
  renderList("#roots-excluded", draft.excluded_roots, true, (p) => { draft.excluded_roots = draft.excluded_roots.filter((x) => x !== p); renderSettings(); });
  $("#set-trash").checked = !!draft.to_trash;
}

async function openSettings() {
  const [s, r] = await Promise.all([invoke("get_settings"), invoke("project_roots")]);
  draft = { language: s.language ?? null, extra_roots: [...(s.extra_roots || [])], excluded_roots: [...(s.excluded_roots || [])], to_trash: !!s.to_trash };
  rootsInfo = r;
  renderSettings();
  $("#settings").showModal();
}

async function pickInto(listKey, titleKey) {
  const path = await invoke("pick_folder", { title: t(titleKey) });
  if (!path) return;
  if (!draft[listKey].some((p) => p.toLowerCase() === path.toLowerCase())) draft[listKey].push(path);
  renderSettings();
}

async function saveSettings() {
  const before = await invoke("get_settings");
  const rootsChanged =
    JSON.stringify(before.extra_roots || []) !== JSON.stringify(draft.extra_roots) ||
    JSON.stringify(before.excluded_roots || []) !== JSON.stringify(draft.excluded_roots);
  await invoke("save_settings", { settings: draft });
  const next = await invoke("ui_language");
  if (next !== lang) setLanguage(next);
  $("#to-trash").checked = draft.to_trash;
  toast(rootsChanged ? t("toast.rescanHint") : t("toast.settingsSaved"));
}

$("#btn-settings").addEventListener("click", () => openSettings().catch((e) => toast(String(e))));
$("#set-add-root").addEventListener("click", () => pickInto("extra_roots", "settings.pickRootTitle").catch((e) => toast(String(e))));
$("#set-add-excluded").addEventListener("click", () => pickInto("excluded_roots", "settings.pickExcludedTitle").catch((e) => toast(String(e))));
for (const r of document.querySelectorAll('input[name="lang"]')) {
  r.addEventListener("change", () => { draft.language = r.value === "system" ? null : r.value; });
}
$("#set-trash").addEventListener("change", (e) => { draft.to_trash = e.target.checked; });
$("#settings").addEventListener("close", () => {
  if ($("#settings").returnValue === "save") saveSettings().catch((e) => toast(String(e)));
});
