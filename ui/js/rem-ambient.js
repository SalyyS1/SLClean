// Rem Aqua Ambient Anime Canvas + Theme Switcher + Ctrl+K Quick Command Palette for SLClean.
// Lightweight (<1% CPU): auto-pauses when window is hidden and respects prefers-reduced-motion.
"use strict";

(function () {
  const THEMES = ["rem-aqua", "rem-twilight", "glacier-mist"];
  const THEME_LABELS = {
    "rem-aqua": { vi: "Rem Aqua", en: "Rem Aqua" },
    "rem-twilight": { vi: "Rem Twilight", en: "Rem Twilight" },
    "glacier-mist": { vi: "Glacier Sáng", en: "Glacier Light" },
  };
  const FX_MODES = ["calm", "dreamy", "off"];
  const FX_LABELS = {
    calm: { vi: "✦ Nền động: Nhẹ", en: "✦ Anime FX: Calm" },
    dreamy: { vi: "✦ Nền động: Mơ màng", en: "✦ Anime FX: Dreamy" },
    off: { vi: "✧ Nền động: Tắt", en: "✧ Anime FX: Off" },
  };

  let currentTheme = localStorage.getItem("slclean-theme") || "rem-aqua";
  if (!THEMES.includes(currentTheme)) currentTheme = "rem-aqua";

  let currentFx = localStorage.getItem("slclean-fx") || "calm";
  if (!FX_MODES.includes(currentFx)) currentFx = "calm";
  if (window.matchMedia && window.matchMedia("(prefers-reduced-motion: reduce)").matches && !localStorage.getItem("slclean-fx")) {
    currentFx = "off";
  }

  let currentDensity = localStorage.getItem("slclean-density") || "comfortable";
  if (currentDensity !== "compact" && currentDensity !== "comfortable") currentDensity = "comfortable";

  function curLang() {
    return typeof lang !== "undefined" && lang === "en" ? "en" : "vi";
  }

  function applyTheme(name) {
    currentTheme = THEMES.includes(name) ? name : "rem-aqua";
    document.documentElement.dataset.theme = currentTheme;
    localStorage.setItem("slclean-theme", currentTheme);
    syncSettingsRadios();
    updatePills();
  }

  function applyFx(mode) {
    currentFx = FX_MODES.includes(mode) ? mode : "calm";
    document.documentElement.dataset.fx = currentFx;
    localStorage.setItem("slclean-fx", currentFx);
    const canvas = document.getElementById("rem-ambient-canvas");
    if (canvas) canvas.hidden = currentFx === "off";
    rebuildParticles();
    syncSettingsRadios();
    updatePills();
  }

  function applyDensity(mode) {
    currentDensity = mode === "compact" ? "compact" : "comfortable";
    document.documentElement.dataset.density = currentDensity;
    localStorage.setItem("slclean-density", currentDensity);
    syncSettingsRadios();
  }

  function syncSettingsRadios() {
    const t = document.querySelector(`input[name="ui-theme"][value="${currentTheme}"]`);
    if (t) t.checked = true;
    const f = document.querySelector(`input[name="ui-fx"][value="${currentFx}"]`);
    if (f) f.checked = true;
    const d = document.querySelector(`input[name="ui-density"][value="${currentDensity}"]`);
    if (d) d.checked = true;
  }

  function updatePills() {
    const l = curLang();
    const tb = document.getElementById("btn-theme-cycle");
    if (tb) tb.textContent = `❄ ${THEME_LABELS[currentTheme][l]}`;
    const fb = document.getElementById("btn-fx-cycle");
    if (fb) fb.textContent = FX_LABELS[currentFx][l];
  }

  // --- Animated Anime Ice-Crystal, Aqua Bokeh & Sakura Ribbon Petal Canvas ---
  let canvas, ctx, w = 0, h = 0, dpr = 1, rafId = 0;
  let particles = [];

  function spawnParticle(randomY) {
    const roll = Math.random();
    // 45% aqua bokeh orb, 38% delicate 6-point Rem ice crystal, 17% Rem ribbon sakura-pink petal
    const kind = roll < 0.45 ? "orb" : roll < 0.83 ? "crystal" : "petal";
    return {
      kind,
      x: Math.random() * (w || 1280),
      y: randomY ? Math.random() * (h || 800) : -24 - Math.random() * 60,
      r: kind === "orb" ? 6 + Math.random() * 18 : kind === "crystal" ? 3.5 + Math.random() * 5.5 : 4 + Math.random() * 4.5,
      vx: (Math.random() - 0.5) * 0.28,
      vy: 0.16 + Math.random() * 0.36,
      rot: Math.random() * Math.PI * 2,
      vrot: (Math.random() - 0.5) * 0.012,
      phase: Math.random() * Math.PI * 2,
      alpha: kind === "orb" ? 0.05 + Math.random() * 0.08 : 0.18 + Math.random() * 0.28,
    };
  }

  function rebuildParticles() {
    const count = currentFx === "dreamy" ? 42 : currentFx === "calm" ? 24 : 0;
    particles = [];
    for (let i = 0; i < count; i++) particles.push(spawnParticle(true));
  }

  function resize() {
    if (!canvas) return;
    dpr = Math.min(window.devicePixelRatio || 1, 2);
    w = window.innerWidth;
    h = window.innerHeight;
    canvas.width = Math.floor(w * dpr);
    canvas.height = Math.floor(h * dpr);
    canvas.style.width = `${w}px`;
    canvas.style.height = `${h}px`;
    if (ctx) ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  }

  function drawCrystal(p, color) {
    ctx.save();
    ctx.translate(p.x, p.y);
    ctx.rotate(p.rot);
    ctx.strokeStyle = color;
    ctx.lineWidth = 1;
    ctx.beginPath();
    for (let i = 0; i < 3; i++) {
      const a = (i * Math.PI) / 3;
      ctx.moveTo(Math.cos(a) * -p.r, Math.sin(a) * -p.r);
      ctx.lineTo(Math.cos(a) * p.r, Math.sin(a) * p.r);
    }
    ctx.stroke();
    ctx.fillStyle = color;
    ctx.beginPath();
    ctx.arc(0, 0, Math.max(1.2, p.r * 0.24), 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }

  function drawPetal(p, color) {
    ctx.save();
    ctx.translate(p.x, p.y);
    ctx.rotate(p.rot);
    ctx.fillStyle = color;
    ctx.beginPath();
    ctx.ellipse(0, 0, p.r * 1.15, p.r * 0.55, 0, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }

  function tick(ts) {
    rafId = requestAnimationFrame(tick);
    if (!ctx || currentFx === "off" || document.hidden) return;
    ctx.clearRect(0, 0, w, h);

    const isLight = currentTheme === "glacier-mist";
    const isTwilight = currentTheme === "rem-twilight";
    const aquaRgb = isLight ? "2, 132, 199" : isTwilight ? "103, 232, 249" : "95, 212, 244";
    const pinkRgb = isLight ? "219, 39, 119" : isTwilight ? "240, 171, 252" : "244, 143, 177";
    const t = ts * 0.0005;

    for (const p of particles) {
      p.y += p.vy;
      p.x += p.vx + Math.sin(t + p.phase) * 0.22;
      p.rot += p.vrot;
      if (p.y > h + 30) {
        p.y = -20;
        p.x = Math.random() * w;
      }
      if (p.x < -30) p.x = w + 20;
      if (p.x > w + 30) p.x = -20;

      const twinkle = 0.78 + 0.22 * Math.sin(t * 2.2 + p.phase);
      const a = p.alpha * twinkle * (currentFx === "dreamy" ? 1.25 : 1);
      if (p.kind === "orb") {
        const g = ctx.createRadialGradient(p.x, p.y, 0, p.x, p.y, p.r);
        g.addColorStop(0, `rgba(${aquaRgb}, ${a.toFixed(3)})`);
        g.addColorStop(1, `rgba(${aquaRgb}, 0)`);
        ctx.fillStyle = g;
        ctx.beginPath();
        ctx.arc(p.x, p.y, p.r, 0, Math.PI * 2);
        ctx.fill();
      } else if (p.kind === "crystal") {
        drawCrystal(p, `rgba(${aquaRgb}, ${a.toFixed(3)})`);
      } else {
        drawPetal(p, `rgba(${pinkRgb}, ${(a * 0.85).toFixed(3)})`);
      }
    }
  }

  // --- Universal Quick Command Palette (Ctrl+K) ---
  function commandItems() {
    const l = curLang();
    const vi = l === "vi";
    return [
      { id: "smart-all", icon: "✦", label: vi ? "Quét Toàn Diện (Cache + Build Artifacts + Rác AI/Codex Documents)" : "All-in-One Smart Scan (Caches + Artifacts + AI/Codex Documents)", hint: "1-Click", run: () => document.getElementById("btn-smart-all")?.click() },
      { id: "scan-ai", icon: "❄", label: vi ? "Quét Rác AI, Codex (Documents) & Bộ cài thừa" : "Scan AI, Codex (Documents) Junk & Old Installers", hint: "SLC016–SLC019", run: () => { switchTabSafe("leftovers"); setTimeout(() => document.getElementById("btn-ai-junk-scan")?.click(), 60); } },
      { id: "pick-safe", icon: "✓", label: vi ? "Chọn tất cả mục Xóa An Toàn 100%" : "Select all 100% Safe-to-Delete items", hint: "Safe only", run: () => { const active = document.querySelector(".tab.is-active")?.dataset.tab; if (active === "leftovers") document.querySelector('[data-left-pick="safe"]')?.click(); else document.querySelector('[data-pick="safe"]')?.click(); } },
      { id: "tab-clean", icon: "🧹", label: vi ? "Mở tab Dọn dẹp (Cache & Build Artifacts)" : "Open Clean tab (Caches & Build Artifacts)", hint: "Alt+1", run: () => switchTabSafe("clean") },
      { id: "tab-left", icon: "📁", label: vi ? "Mở tab Thư mục thừa & Rác Codex/Documents" : "Open Leftovers & Codex/Documents Junk", hint: "Alt+3", run: () => switchTabSafe("leftovers") },
      { id: "tab-files", icon: "🧊", label: vi ? "Mở tab File lớn & Bản đồ khối Treemap" : "Open Large Files & Disk Treemap", hint: "Alt+4", run: () => switchTabSafe("files") },
      { id: "tab-apps", icon: "📦", label: vi ? "Mở tab Ứng dụng (Gỡ app ít dùng)" : "Open Installed Apps", hint: "Alt+2", run: () => switchTabSafe("apps") },
      { id: "tab-startup", icon: "⚡", label: vi ? "Mở tab Khởi động & Điểm Boot Grade" : "Open Startup & Boot Health Grade", hint: "Alt+6", run: () => switchTabSafe("startup") },
      { id: "theme", icon: "❄", label: vi ? "Đổi giao diện (Rem Aqua / Rem Twilight / Glacier)" : "Switch Theme (Rem Aqua / Rem Twilight / Glacier)", hint: currentTheme, run: () => applyTheme(THEMES[(THEMES.indexOf(currentTheme) + 1) % THEMES.length]) },
      { id: "fx", icon: "✨", label: vi ? "Chỉnh hiệu ứng nền động Anime (Nhẹ / Mơ màng / Tắt)" : "Cycle Anime Ambient FX (Calm / Dreamy / Off)", hint: currentFx, run: () => applyFx(FX_MODES[(FX_MODES.indexOf(currentFx) + 1) % FX_MODES.length]) },
      { id: "density", icon: "☰", label: vi ? "Đổi mật độ hàng (Thoải mái / Gọn nhẹ)" : "Toggle Row Density (Comfortable / Compact)", hint: currentDensity, run: () => applyDensity(currentDensity === "compact" ? "comfortable" : "compact") },
      { id: "shortcuts", icon: "⌨", label: vi ? "Xem bảng phím tắt nhanh" : "Show Keyboard Shortcuts Cheatsheet", hint: "?", run: () => document.getElementById("shortcuts-dialog")?.showModal() },
      { id: "export", icon: "📄", label: vi ? "Xuất báo cáo kiểm toán HTML" : "Export HTML Audit Report", hint: "HTML", run: () => document.getElementById("btn-export-html")?.click() },
    ];
  }

  function switchTabSafe(tabId) {
    document.querySelector(`.tab[data-tab="${tabId}"]`)?.click();
  }

  function openPalette() {
    const dlg = document.getElementById("cmd-palette");
    const inp = document.getElementById("cmd-input");
    if (!dlg || !inp) return;
    inp.value = "";
    renderPalette("");
    if (!dlg.open) dlg.showModal();
    setTimeout(() => inp.focus(), 20);
  }

  function renderPalette(q) {
    const list = document.getElementById("cmd-list");
    if (!list) return;
    const term = q.trim().toLowerCase();
    const matched = commandItems().filter((c) => !term || c.label.toLowerCase().includes(term) || c.hint.toLowerCase().includes(term));
    list.innerHTML = "";
    for (const c of matched) {
      const li = document.createElement("li");
      li.className = "cmd__item";
      li.tabIndex = 0;
      li.innerHTML = `<span class="cmd__icon">${c.icon}</span><span class="cmd__label"></span><span class="cmd__hint"></span>`;
      li.querySelector(".cmd__label").textContent = c.label;
      li.querySelector(".cmd__hint").textContent = c.hint;
      const exec = () => {
        document.getElementById("cmd-palette")?.close();
        c.run();
      };
      li.addEventListener("click", exec);
      li.addEventListener("keydown", (e) => { if (e.key === "Enter") exec(); });
      list.appendChild(li);
    }
  }

  function init() {
    canvas = document.getElementById("rem-ambient-canvas");
    if (canvas) {
      ctx = canvas.getContext("2d");
      resize();
      window.addEventListener("resize", resize);
    }
    applyTheme(currentTheme);
    applyFx(currentFx);
    applyDensity(currentDensity);
    if (canvas) rafId = requestAnimationFrame(tick);

    document.getElementById("btn-theme-cycle")?.addEventListener("click", () => {
      applyTheme(THEMES[(THEMES.indexOf(currentTheme) + 1) % THEMES.length]);
    });
    document.getElementById("btn-fx-cycle")?.addEventListener("click", () => {
      applyFx(FX_MODES[(FX_MODES.indexOf(currentFx) + 1) % FX_MODES.length]);
    });
    document.getElementById("btn-cmd-open")?.addEventListener("click", openPalette);

    // 1-Click All-in-One Smart Scan (Clean Caches + AI/Codex Documents Junk)
    document.getElementById("btn-smart-all")?.addEventListener("click", () => {
      document.getElementById("btn-scan")?.click();
      if (typeof scanAiJunk === "function") {
        setTimeout(() => scanAiJunk(), 120);
      }
      if (typeof toast === "function") {
        toast(curLang() === "vi" ? "✦ Đang quét toàn diện: Cache, Artifact & Rác AI/Codex (Documents)…" : "✦ Running All-in-One Smart Scan (Caches + Artifacts + AI/Codex Documents)…");
      }
    });

    // Live Appearance & Density radios inside Settings dialog
    for (const r of document.querySelectorAll('input[name="ui-theme"]')) {
      r.addEventListener("change", () => applyTheme(r.value));
    }
    for (const r of document.querySelectorAll('input[name="ui-fx"]')) {
      r.addEventListener("change", () => applyFx(r.value));
    }
    for (const r of document.querySelectorAll('input[name="ui-density"]')) {
      r.addEventListener("change", () => applyDensity(r.value));
    }

    // Quick-action launcher cards on Clean tab
    document.getElementById("qa-codex-docs")?.addEventListener("click", () => {
      switchTabSafe("leftovers");
      setTimeout(() => document.getElementById("btn-ai-junk-scan")?.click(), 70);
    });
    document.getElementById("qa-large-files")?.addEventListener("click", () => switchTabSafe("files"));
    document.getElementById("qa-startup-grade")?.addEventListener("click", () => switchTabSafe("startup"));
    document.getElementById("qa-uninstalled")?.addEventListener("click", () => switchTabSafe("leftovers"));

    const inp = document.getElementById("cmd-input");
    if (inp) {
      inp.addEventListener("input", () => renderPalette(inp.value));
      inp.addEventListener("keydown", (e) => {
        if (e.key === "Enter") {
          const first = document.querySelector("#cmd-list .cmd__item");
          if (first) first.click();
        }
      });
    }

    document.addEventListener("keydown", (e) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        openPalette();
      } else if (e.key === "?" && !e.ctrlKey && !e.metaKey && !e.altKey) {
        const tag = (e.target && e.target.tagName) || "";
        if (tag !== "INPUT" && tag !== "TEXTAREA" && tag !== "SELECT") {
          e.preventDefault();
          const sd = document.getElementById("shortcuts-dialog");
          if (sd && !sd.open) sd.showModal();
        }
      }
    });

    // Keep pills synced if language button is clicked
    document.getElementById("btn-lang")?.addEventListener("click", () => setTimeout(updatePills, 30));
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
