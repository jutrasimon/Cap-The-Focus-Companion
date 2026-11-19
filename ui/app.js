const T = window.__TAURI__;
const invoke = (cmd, args) => T.core.invoke(cmd, args);
const $ = (id) => document.getElementById(id);
const I = window.ICON;

const PRESETS = [15, 25, 45, 60, 90];
const PC = ["#ffe58a", "#ffb8c9", "#a8ecd0", "#aad5ff", "#ffcb9a", "#d6c6ff"];
const GDEF = { enabled: true, max_min: 15, drain: 1, refill_per_hour: 15 };
const BNAME = { chrome: "Chrome", edge: "Edge", firefox: "Firefox", brave: "Brave", opera: "Opera", vivaldi: "Vivaldi" };
const C = 2 * Math.PI * 98;
const DAY = 86400000;

let S = null;
// Textes de la langue courante (fichiers locales/*.json)
let L = {};
const t = (k, v = {}) => { let s = L[k] ?? k; for (const [a, b] of Object.entries(v)) s = s.split(`{${a}}`).join(b); return s; };
const LOC = () => L["_meta.locale"] || undefined;
function applyI18n(root = document) {
  root.querySelectorAll("[data-t]").forEach((el) => (el.textContent = t(el.dataset.t)));
  root.querySelectorAll("[data-t-ph]").forEach((el) => (el.placeholder = t(el.dataset.tPh)));
  root.querySelectorAll("[data-t-tip]").forEach((el) => { el.dataset.tip = t(el.dataset.tTip); el.removeAttribute("title"); });
  document.documentElement.lang = L["_meta.code"] || "fr";
}
let tab = "dossiers", lastTab = "dossiers";
let hadSession = false;
let minutes = 25, primed = false;
let archOpen = false, archMode = "done", archExpanded = null;
let openSite = null, openMenu = null, openRem = null;
let period = 7, stats = null, statsAt = 0;
let updBusy = false;
let perfTimer = null, perfHist = [], scListen = null;

// ------------------------------------------------------------ utilitaires
const fmt = (ms) => {
  const s = Math.max(0, Math.ceil(ms / 1000));
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), r = s % 60;
  return h ? `${h}:${String(m).padStart(2, "0")}:${String(r).padStart(2, "0")}` : `${m}:${String(r).padStart(2, "0")}`;
};
const dur = (min) => {
  min = Math.round(min);
  if (min < 60) return t("fmt.min", { n: min });
  const h = Math.floor(min / 60), m = min % 60;
  return m ? t("fmt.hm", { h, m: String(m).padStart(2, "0") }) : t("fmt.h", { h });
};
const clock = (x) => new Date(x).toLocaleTimeString(LOC(), { hour: "2-digit", minute: "2-digit" });
const dateS = (x) => new Date(x).toLocaleDateString(LOC(), { day: "numeric", month: "short" });
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
const short = (d) => d.replace(/\.(com|ca|net|org|tv|io|co)$/, "");
const activeSites = () => S.settings.sites.filter((x) => !S.settings.sites_off.includes(x));
// Vrai seulement si l'utilisateur écrit dans un champ de cette zone (un bouton cliqué ne compte pas)
const typingEl = () => { const a = document.activeElement; return !!a && (a.isContentEditable || a.tagName === "INPUT" || a.tagName === "TEXTAREA"); };
const editingIn = (el) => typingEl() && el.contains(document.activeElement);
const setHTML = (el, html) => { if (el.dataset.k !== html) { el.innerHTML = html; el.dataset.k = html; } };
const dayStart = (t) => { const d = new Date(t); d.setHours(0, 0, 0, 0); return d.getTime(); };

function icons(root = document) {
  root.querySelectorAll("[data-i]").forEach((el) => { if (!el.dataset.iok) { el.innerHTML = I[el.dataset.i] || ""; el.dataset.iok = 1; } });
}

// Corbeille en deux temps : un clic arme, le deuxième confirme
// L'état « armé » est retenu par clé, pas par élément : il survit aux rafraîchissements de la liste.
let armKey = null, armAt = 0;
const keyOf = (b) => Object.entries(b.dataset).filter(([k]) => k !== "tip").map(([k, v]) => `${k}=${v}`).join("&") || b.id;
function armed(btn) {
  const k = keyOf(btn);
  if (armKey === k && Date.now() - armAt < 3000) { armKey = null; restoreArmed(); return true; }
  armKey = k; armAt = Date.now();
  restoreArmed();
  setTimeout(restoreArmed, 3050);
  return false;
}
function restoreArmed() {
  const live = armKey && Date.now() - armAt < 3000;
  document.querySelectorAll(".del").forEach((b) => b.classList.toggle("armed", !!live && keyOf(b) === armKey));
}

function toast(text) {
  const el = document.createElement("div");
  el.className = "toast";
  el.innerHTML = I.check;
  el.appendChild(document.createTextNode(text));
  $("card").appendChild(el);
  setTimeout(() => el.remove(), 1800);
}

// ------------------------------------------------------------ navigation
function showTab(name) {
  const t_ = name;
  if (t_ !== "settings" && t_ !== "perf") lastTab = t_;
  tab = t_;
  document.querySelectorAll(".view").forEach((v) => {
    const on = v.dataset.view === t_;
    v.classList.toggle("hidden", !on);
    if (on) { v.classList.remove("enter"); void v.offsetWidth; v.classList.add("enter"); }
  });
  document.querySelectorAll("#tabs button").forEach((b) => b.classList.toggle("sel", b.dataset.tab === t_));
  $("gearBtn").classList.toggle("on", t_ === "settings");
  $("perfBtn").classList.toggle("on", t_ === "perf");
  clearInterval(perfTimer);
  if (t_ === "perf") { pollPerf(); perfTimer = setInterval(pollPerf, 1000); }
  if (t_ === "stats") loadStats();
  if (t_ === "settings") refresh();
  render();
}

// ------------------------------------------------------------ rendu
function render() {
  if (!S) return;
  $("tabRappels").classList.toggle("alert", !S.settings.reminders_on);
  if (tab === "focus") renderFocus();
  if (tab === "dossiers") renderDossiers();
  if (tab === "jauges") renderSites();
  if (tab === "rappels") renderRappels();
  if (tab === "stats" && Date.now() - statsAt > 30000) loadStats();
  if (tab === "settings") renderSettings();
  $("gearBtn").classList.toggle("dot", !!S.update);
  restoreArmed();
}

// ---------------------------------------------------- focus
function renderFocus() {
  const s = S.session;
  const wrap = $("ringWrap");
  $("ring").style.strokeDasharray = C;
  wrap.classList.toggle("idle", !s);
  wrap.classList.toggle("running", !!s && !s.paused);
  wrap.classList.toggle("paused", !!s && s.paused);
  $("idleCtl").classList.toggle("hidden", !!s);
  $("runCtl").classList.toggle("hidden", !s);
  $("minInput").classList.toggle("hidden", !!s);
  $("minUnit").classList.toggle("hidden", !!s);
  $("time").classList.toggle("hidden", !s);
  // Le texte du focus se saisit avant ou pendant ; il se vide à la fin de la session
  if (s && document.activeElement !== $("focusNote")) $("focusNote").value = s.goal || "";
  if (!s && hadSession) $("focusNote").value = "";
  hadSession = !!s;
  if (s) {
    $("time").textContent = fmt(s.left);
    $("ring").style.strokeDashoffset = C * (1 - Math.max(0, Math.min(1, s.left / s.total)));
    $("pauseBtn").innerHTML = s.paused ? I.play : I.pause;
  } else {
    if (document.activeElement !== $("minInput")) $("minInput").value = minutes;
    // anneau proportionnel à la durée choisie (plein à 60 min)
    $("ring").style.strokeDashoffset = C * (1 - Math.min(1, minutes / 60));
  }
}

function setMinutes(m) {
  minutes = Math.max(1, Math.min(600, Math.round(m) || 1));
  renderFocus();
}

function startFocus() {
  const v = parseInt($("minInput").value, 10);
  if (v > 0) minutes = Math.min(600, v);
  $("minInput").blur();
  invoke("start", { minutes, goal: $("focusNote").value.trim() });
  $("focusNote").blur();
}

// ---------------------------------------------------- dossiers
const rot = (id) => (((id * 37) % 7) - 3) * 0.5;

function postit(x) {
  const lvl = Math.round(x.level);
  const style = `--pc:${PC[x.color % 6]};--rot:${rot(x.id)}deg`;
  if (x.state === "lost") {
    return `<div class="postit lost" style="${style}" data-id="${x.id}">
      <span class="ghost">${I.skull}</span>
      <div class="pt">${esc(x.title)}</div>
      <div class="pn">${esc(x.notes)}</div>
      <div class="pf"><button class="bury" data-bury="${x.id}" title="${t("dos.bury")}">${I.skull}<span>${t("dos.buryBtn")}</span></button></div>
    </div>`;
  }
  const menu = openMenu === x.id ? `
      <div class="pmenu">
        <div class="stepper"><button data-decay="${x.id}" data-d="-1" title="${t("dos.decaySlower")}">${I.minus}</button><span title="${t("dos.decayPerDay")}">${t("fmt.perDay", { n: x.decay_eff })}</span><button data-decay="${x.id}" data-d="1" title="${t("dos.decayFaster")}">${I.plus}</button></div>
        <button class="mini-btn" data-color="${x.id}" title="${t("dos.color")}">${I.palette}</button>
        <button class="mini-btn del" data-del="${x.id}" title="${t("dos.delete")}">${I.trash}</button>
      </div>` : "";
  const days = x.ms_left < 9e15 ? Math.ceil(x.ms_left / DAY) : "∞";
  return `<div class="postit" style="${style}" data-id="${x.id}">
      <div class="pt" contenteditable="plaintext-only" data-f="title" spellcheck="false">${esc(x.title)}</div>
      <div class="pn" contenteditable="plaintext-only" data-f="notes" spellcheck="false">${esc(x.notes)}</div>
      <div class="pf">
        <button class="actif ${lvl < 25 ? "low" : ""}" data-act="${x.id}" title="${t("dos.activeTip", { n: days })}"><span class="af" style="width:${lvl}%"></span>${I.flame}<span>${lvl}%</span></button>
        <button class="mini-btn" data-done="${x.id}" title="${t("dos.complete")}">${I.check}</button>
        <button class="mini-btn" data-menu="${x.id}" title="${t("dos.options")}">${I.more}</button>
      </div>${menu}
    </div>`;
}

function renderDossiers() {
  $("board").classList.toggle("hidden", archOpen);
  $("archives").classList.toggle("hidden", !archOpen);
  $("archBtn").classList.toggle("on", archOpen);
  $("archBtn").innerHTML = archOpen ? I.folder : I.archive;
  $("archBtn").dataset.tip = archOpen ? t("dos.current") : t("dos.archives");
  $("archBtn").removeAttribute("title");
  if (!archOpen) {
    if (editingIn($("board"))) return;
    const live = S.dossiers.filter((x) => x.state === "open" || x.state === "lost");
    setHTML($("board"), live.length ? live.map(postit).join("") : `<div class="empty-ico">${I.folder}</div>`);
  } else {
    const done = S.dossiers.filter((x) => x.state === "done");
    const dead = S.dossiers.filter((x) => x.state === "dead");
    $("archDoneN").textContent = done.length;
    $("archDeadN").textContent = dead.length;
    document.querySelectorAll("#archSeg button").forEach((b) => b.classList.toggle("sel", b.dataset.a === archMode));
    const list = (archMode === "done" ? done : dead).slice().sort((a, b) => (b.ended || 0) - (a.ended || 0));
    setHTML($("archList"), list.length ? list.map((x) => {
      const life = Math.max(1, Math.round(((x.ended || S.now) - x.created) / DAY));
      return `<div class="arch">
        <div class="arch-h" data-exp="${x.id}"><span class="dotc" style="background:${PC[x.color % 6]}"></span><b>${esc(x.title)}</b>
          ${archMode === "done" ? `<button class="icon-btn" data-reopen="${x.id}" title="${t("dos.reopen")}">${I.undo}</button>` : ""}
          <button class="icon-btn del" data-del="${x.id}" title="${t("dos.delete")}">${I.trash}</button></div>
        <div class="arch-meta"><span title="${t("dos.createdEnded")}">${I.clock}${dateS(x.created)} → ${dateS(x.ended || S.now)}</span><span title="${t("dos.lifespan")}">${t("fmt.days", { n: life })}</span><span title="${t("dos.activations")}">${I.flame}${x.activations}</span></div>
        ${archExpanded === x.id && x.notes ? `<div class="arch-notes">${esc(x.notes)}</div>` : ""}
      </div>`;
    }).join("") : `<div class="empty-ico" title="${t("dos.empty")}">${archMode === "done" ? I.check : I.skull}</div>`);
  }
}

// ---------------------------------------------------- jauges
const gaugeOf = (site) => (S.gauges || {})[site] || { level: 0, max: 900, locked: false, using: false, cfg: GDEF, off: false, refill_in: 0 };

let addFocusOnly = false;
function renderSiteMode() {
  const b = $("siteMode");
  b.innerHTML = addFocusOnly ? I.timer : I.gauge;
  b.classList.toggle("accent", addFocusOnly);
  b.dataset.tip = t(addFocusOnly ? "site.modeFocus" : "site.modeGauge");
}

function renderSites() {
  const st = S.settings;
  const html = st.sites.map((site, i) => {
    const g = gaugeOf(site);
    const c = { ...GDEF, ...(st.gauge_cfg || {})[site] };
    const off = st.sites_off.includes(site);
    const pct = Math.max(0, Math.min(100, (100 * g.level) / g.max));
    const label = off ? "" : !c.enabled ? `<b class="ftag" data-tip="${esc(t("site.focusTip"))}">${I.timer}${t("site.focusTag")}</b>` : g.locked ? `${I.lock}${t("fmt.min", { n: Math.ceil(g.refill_in / 60) })}` : `${Math.floor(g.level / 60)} / ${t("fmt.min", { n: c.max_min })}`;
    const body = openSite === site ? `
      <div class="site-b">
        <i>${I.gauge}</i><span>${t("site.gauge")}</span><div class="switch end ${c.enabled ? "on" : ""}" data-gon="${i}" data-tip="${esc(t(c.enabled ? "site.modeGauge" : "site.modeFocus"))}"></div>
        ${c.enabled ? `<i>${I.size}</i><span>${t("site.size")}</span><div class="stepper end"><button data-g="${i}" data-k="max_min" data-d="-5">${I.minus}</button><span>${t("fmt.min", { n: c.max_min })}</span><button data-g="${i}" data-k="max_min" data-d="5">${I.plus}</button></div>
        <i>${I.down}</i><span>${t("site.drain")}</span><div class="stepper end"><button data-g="${i}" data-k="drain" data-d="-0.5">${I.minus}</button><span>×${c.drain}</span><button data-g="${i}" data-k="drain" data-d="0.5">${I.plus}</button></div>
        <i>${I.up}</i><span>${t("site.refill")}</span><div class="stepper end"><button data-g="${i}" data-k="refill_per_hour" data-d="-5">${I.minus}</button><span>${t("fmt.perHour", { n: c.refill_per_hour })}</span><button data-g="${i}" data-k="refill_per_hour" data-d="5">${I.plus}</button></div>` : ""}
        <div class="foot"><button class="icon-btn del" data-del="${i}" title="${t("site.remove")}">${I.trash}</button></div>
      </div>` : "";
    return `<div class="site ${off ? "off" : ""}">
      <div class="site-h" data-open="${i}">
        <div class="site-m"><div class="site-n">${esc(site)}<span>${label}</span></div>
          ${!off && c.enabled ? `<div class="gbar"><div class="gfill ${pct < 25 ? "low" : ""} ${g.using ? "using" : ""}" style="width:${pct}%"></div></div>` : ""}</div>
        <div class="switch ${off ? "" : "on"}" data-tog="${i}" title="${t("site.active")}"></div>
      </div>${body}</div>`;
  }).join("");
  setHTML($("siteList"), html);
}

function saveSettings(mut) {
  const cfg = structuredClone(S.settings);
  mut(cfg);
  S.settings = cfg;
  render();
  invoke("save_settings", { settings: cfg });
}

// ---------------------------------------------------- rappels
function renderRappels() {
  const on = S.settings.reminders_on;
  $("masterRow").classList.toggle("off", !on);
  $("masterIco").innerHTML = on ? I.bell : I.bellOff;
  $("masterSwitch").classList.toggle("on", on);
  // 1 raté : avertissement. 2 ratés : tout est en pause.
  const warn = S.missed > 0;
  $("missedTag").classList.toggle("hidden", !warn);
  $("missedTag").innerHTML = on ? `${I.bellOff}${t("rem.missedN", { n: S.missed })}` : `${I.bellOff}${t("rem.paused")}`;
  $("missedTag").dataset.tip = on ? t("rem.missedTip", { n: Math.max(1, (S.settings.misses_to_pause || 2) - S.missed) }) : t("rem.pausedTip");
  setHTML($("missStep"), `<button data-ms="-1">${I.minus}</button><span>${t("rem.missesN", { n: S.settings.misses_to_pause || 2 })}</span><button data-ms="1">${I.plus}</button>`);
  // Rappels en attente : réclamables jusqu'au prochain du même type
  const pend = (S.pending || []).slice().sort((a, b) => b.at - a.at);
  $("pendHead").classList.toggle("hidden", !pend.length);
  setHTML($("pendList"), pend.map((p) => `
    <div class="rem pend"><span class="ri">${I[p.icon] || I.bell}</span>
      <div class="rt"><span class="rl" style="cursor:default">${esc(p.label)}</span><span class="rn">${I.clock}${esc(agoTxt(S.now - p.at))}</span></div>
      <button class="icon-btn del" data-pdrop="${esc(p.rid)}" title="${esc(t("rem.lose"))}">${I.x}</button>
      <button class="icon-btn solid sm2" data-pclaim="${esc(p.rid)}" title="${esc(t("rem.claim"))}">${I.check}</button>
    </div>`).join(""));


  const once = S.settings.reminders.filter((r) => r.once_at).sort((a, b) => a.once_at - b.once_at);
  setHTML($("onceList"), once.map((r) => `
    <div class="rem"><span class="ri">${I.bell}</span><span class="rl" style="cursor:default">${esc(r.label)}</span>
      <span class="when" title="${t("fmt.at", { t: clock(r.once_at) })}">${I.clock}${soon(r.once_at - S.now)}</span>
      <button class="icon-btn del" data-rdel="${r.id}" title="${t("rem.delete")}">${I.trash}</button></div>`).join(""));

  if (editingIn($("remList"))) return;
  const rec = S.settings.reminders.filter((r) => !r.once_at);
  setHTML($("remList"), rec.map((r) => `
    <div class="rem ${r.enabled ? "" : "off"}" data-id="${r.id}">
      <button class="ri" data-ico="${r.id}" title="${t("rem.icon")}">${I[r.icon] || I.bell}</button>
      <div class="rt"><span class="rl" contenteditable="plaintext-only" data-label="${r.id}" spellcheck="false">${esc(r.label)}</span>
        <span class="rn">${!r.enabled || !on ? "—" : S.due && S.due[r.id] != null ? `${I.clock}${soon(S.due[r.id])}` : ""}</span></div>
      <div class="stepper" title="${t("rem.interval")}"><button data-every="${r.id}" data-d="-1">${I.minus}</button><span>${dur(r.every_min)}</span><button data-every="${r.id}" data-d="1">${I.plus}</button></div>
      <div class="switch ${r.enabled ? "on" : ""}" data-ron="${r.id}"></div>
      <button class="icon-btn del" data-rdel="${r.id}" title="${t("rem.delete")}">${I.trash}</button>
    </div>`).join(""));
}

const agoTxt = (ms) => (ms < 60000 ? t("rem.now") : t("rem.ago", { d: soon(ms) }));

// "45 s", "12 min", "1 h 05"
const soon = (ms) => {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return t("fmt.s", { n: s });
  const m = Math.ceil(s / 60);
  return m < 60 ? t("fmt.min", { n: m }) : t("fmt.hm", { h: Math.floor(m / 60), m: String(m % 60).padStart(2, "0") });
};

function quickAt(at, src = "qInput") {
  const label = $(src).value.trim();
  $(src).value = "";
  invoke("reminder_once", { label, at });
  closeQuick();
  toast(`${label || t("rem.default")} · ${clock(at)}`);
}

// Quand : "15" = dans 15 min, "1h30" = dans 1 h 30, "14:30" = à 14 h 30. Vide = 15 min.
function parseWhen(str) {
  const v = String(str || "").trim().toLowerCase().replace(/\s+/g, "");
  if (!v) return Date.now() + 15 * 60000;
  let m = v.match(/^(\d{1,2}):(\d{2})$/);
  if (m) {
    const d = new Date(); d.setHours(+m[1], +m[2], 0, 0);
    if (+m[1] > 23 || +m[2] > 59) return null;
    if (d.getTime() <= Date.now()) d.setDate(d.getDate() + 1);
    return d.getTime();
  }
  m = v.match(/^(\d{1,3})(?:m|min)?$/);
  if (m && +m[1] > 0) return Date.now() + +m[1] * 60000;
  m = v.match(/^(\d{1,2})h(\d{0,2})(?:m|min)?$/);
  if (m) return Date.now() + (+m[1] * 60 + +(m[2] || 0)) * 60000;
  return null;
}

// Champ « quand » : aperçu de l'heure, molette ±5 min, rouge si illisible
function bindWhen(input, onSubmit) {
  const upd = () => {
    const at = parseWhen(input.value);
    input.classList.toggle("bad", at === null);
    input.dataset.tip = at === null ? t("rem.whenBad") : `${t("fmt.at", { t: clock(at) })} · ${t("rem.whenTip")}`;
  };
  input.addEventListener("input", upd);
  input.addEventListener("focus", upd);
  input.addEventListener("wheel", (e) => {
    e.preventDefault();
    const cur = /^\d+$/.test(input.value.trim()) ? +input.value.trim() : 15;
    input.value = Math.max(5, Math.min(600, cur + (e.deltaY < 0 ? 5 : -5)));
    upd();
  }, { passive: false });
  input.addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); onSubmit(); } });
}

// Pas de 1 min jusqu'à 10, puis 5 min jusqu'à 1 h, puis 15 min
const stepEvery = (m, d) => {
  const size = (x) => (x < 10 ? 1 : x < 60 ? 5 : 15);
  let n = d > 0 ? m + size(m) : m - size(m - 1);
  const k = size(n);
  n = Math.round(n / k) * k;
  return Math.max(1, Math.min(1440, n));
};

// ---------------------------------------------------- réglages
function renderSettings() {
  $("autoSwitch").classList.toggle("on", !!S.autostart);
  $("soundSwitch").classList.toggle("on", S.settings.sound);
  setHTML($("decayStep"), `<button data-dd="-1">${I.minus}</button><span>${t("fmt.perDay", { n: S.settings.dossier_decay })}</span><button data-dd="1">${I.plus}</button>`);
  const sc = (a) => a.split("+").map((k) => (["ctrl", "alt", "shift", "super", "space"].includes(k) ? t(`key.${k}`) : k.replace(/^key/, "").toUpperCase())).join(" ");
  // Langue
  const langs = S.langs || [];
  setHTML($("langSel"), [`<option value="auto">${esc(t("set.langAuto"))}</option>`, ...langs.map((l) => `<option value="${esc(l.code)}">${esc(l.name)}</option>`)].join(""));
  if (document.activeElement !== $("langSel")) $("langSel").value = S.settings.lang || "auto";
  const ok = (S.about && S.about.sc_ok) || [true, true];
  if (scListen !== "open") $("scOpen").textContent = sc(S.settings.sc_open);
  if (scListen !== "quick") $("scQuick").textContent = sc(S.settings.sc_quick);
  for (const [id, i] of [["scOpenOk", 0], ["scQuickOk", 1]]) {
    $(id).innerHTML = ok[i] ? I.check : I.x;
    $(id).className = `scs ${ok[i] ? "okc" : "warn"}`;
    $(id).dataset.tip = ok[i] ? t("set.scOk") : t("set.scTaken");
  }
  if (!updBusy) {
    $("updRow").classList.toggle("avail", !!S.update);
    $("updTxt").textContent = S.update ? t("set.updAvail", { v: S.update }) : t("set.upToDate");
    $("updBtn").innerHTML = S.update ? I.down : I.repeat;
    $("updBtn").dataset.tip = S.update ? t("set.updInstall") : t("set.updCheck");
  }
  const ab = S.about;
  if (!ab) return;
  const list = ab.browsers || [];
  setHTML($("browserList"), list.length ? list.map((b) => {
    const old = b.version !== ab.ext_version; // une extension sans version est forcément ancienne
    return `<div class="row"><b>${esc(BNAME[b.name] || b.name)}</b><span class="muted">${esc(b.version || "?")}</span>
      <span class="${old ? "warn" : "okc"}" title="${old ? esc(t("set.extOld", { v: b.version || t("set.extOldName"), n: ab.ext_version })) : t("set.upToDate")}">${old ? I.repeat : I.check}</span></div>`;
  }).join("") : `<div class="row"><b class="muted">${t("set.noBrowser")}</b></div>`);
  setHTML($("about"), `
    <div class="ab"><span>${t("set.version")}</span><b>${esc(ab.version)}</b></div>
    <div class="ab"><span>${t("set.built")}</span><b>${new Date(ab.built).toLocaleString(LOC(), { dateStyle: "medium", timeStyle: "short" })}</b></div>
    <div class="ab"><span>${t("set.extShipped")}</span><b>${esc(ab.ext_version)}</b></div>
    <div class="ab"><span>${t("set.data")}</span><b class="path" title="${esc(ab.data_dir)}">${esc(ab.data_dir)}</b><button class="icon-btn sm" id="openData" title="${t("set.openFolder")}">${I.folder}</button></div>`);
}

// ---------------------------------------------------- stats
async function loadStats() {
  statsAt = Date.now();
  try { stats = await invoke("get_stats"); } catch { stats = null; }
  renderStats();
}

function renderStats() {
  if (!stats || !S) return;
  const now = stats.now;
  const first = stats.events.reduce((a, e) => Math.min(a, e.t), Object.keys(stats.usage).reduce((a, h) => Math.min(a, +h), now));
  const days = period || Math.max(1, Math.ceil((now - first) / DAY) + 1);
  const from = period === 1 ? dayStart(now) : dayStart(now) - (days - 1) * DAY;
  const ev = stats.events.filter((e) => e.t >= from);
  const of = (k) => ev.filter((e) => e.k === k);

  // Focus
  const done = of("focus_done"), stop = of("focus_stop");
  const sess = [...done, ...stop];
  const total = sess.reduce((a, e) => a + e.n, 0);
  const rate = sess.length ? Math.round((100 * done.length) / sess.length) : 0;
  const longest = sess.reduce((a, e) => Math.max(a, e.n), 0);
  // série de jours consécutifs avec du focus, jusqu'à aujourd'hui
  const focusDays = new Set(stats.events.filter((e) => e.k === "focus_done" || (e.k === "focus_stop" && e.n > 0)).map((e) => dayStart(e.t)));
  let streak = 0;
  for (let d = dayStart(now); focusDays.has(d); d -= DAY) streak++;

  // barres : par heure (Jour) ou par jour
  let buckets, labels;
  if (period === 1) {
    buckets = Array(24).fill(0);
    sess.forEach((e) => { buckets[new Date(e.t - e.n * 60000).getHours()] += e.n; });
    labels = [0, 6, 12, 18, 23].map((h) => t("fmt.h", { h }));
  } else {
    const nb = Math.min(days, 60);
    const start = dayStart(now) - (nb - 1) * DAY;
    buckets = Array(nb).fill(0);
    sess.forEach((e) => { const i = Math.floor((dayStart(e.t) - start) / DAY); if (i >= 0 && i < nb) buckets[i] += e.n; });
    labels = [dateS(start), dateS(now)];
  }
  const bmax = Math.max(1, ...buckets);
  const barTip = (i) => period === 1 ? `${t("fmt.h", { h: i })} · ${dur(buckets[i])}` : `${dateS(dayStart(now) - (buckets.length - 1 - i) * DAY)} · ${dur(buckets[i])}`;

  // heures productives (toute la période)
  const hours = Array(24).fill(0);
  sess.forEach((e) => { hours[new Date(e.t - e.n * 60000).getHours()] += e.n; });
  const hmax = Math.max(1, ...hours);

  // Sites
  const usage = {};
  Object.entries(stats.usage).forEach(([h, m]) => { if (+h >= from) Object.entries(m).forEach(([s, sec]) => (usage[s] = (usage[s] || 0) + sec)); });
  const lock = {}, hitF = {}, hitG = {};
  of("lockout").forEach((e) => (lock[e.a] = (lock[e.a] || 0) + 1));
  of("block_hit").forEach((e) => { const [s, r] = e.a.split("|"); (r === "gauge" ? hitG : hitF)[s] = ((r === "gauge" ? hitG : hitF)[s] || 0) + 1; });
  const siteNames = [...new Set([...Object.keys(usage), ...Object.keys(lock), ...Object.keys(hitF), ...Object.keys(hitG)])].sort((a, b) => (usage[b] || 0) - (usage[a] || 0));
  const umax = Math.max(1, ...siteNames.map((s) => usage[s] || 0));
  const usageTotal = Object.values(usage).reduce((a, b) => a + b, 0);
  const hits = Object.values(hitF).reduce((a, b) => a + b, 0) + Object.values(hitG).reduce((a, b) => a + b, 0);

  // Rappels
  const ack = of("rem_ack"), miss = of("rem_missed"), later = of("rem_later");
  const answered = ack.length + miss.length;
  const ackRate = answered ? Math.round((100 * ack.length) / answered) : 0;
  const perRem = {};
  [...ack.map((e) => [e.a, "a"]), ...miss.map((e) => [e.a, "m"])].forEach(([a, k]) => { perRem[a] = perRem[a] || { a: 0, m: 0 }; perRem[a][k]++; });
  const remNames = Object.keys(perRem).sort((x, y) => (perRem[y].a + perRem[y].m) - (perRem[x].a + perRem[x].m));

  // Dossiers
  const dNew = of("dos_new").length, dAct = of("dos_active").length, dDone = of("dos_done").length, dLost = of("dos_lost").length;
  const open = S.dossiers.filter((x) => x.state === "open");
  const avgLvl = open.length ? Math.round(open.reduce((a, x) => a + x.level, 0) / open.length) : 0;
  const ended = S.dossiers.filter((x) => (x.state === "done" || x.state === "dead") && x.ended);
  const life = ended.length ? Math.round(ended.reduce((a, x) => a + (x.ended - x.created), 0) / ended.length / DAY) : 0;

  const kpi = (v, l, t = "") => `<div class="kpi" ${t ? `data-tip="${esc(t)}"` : ""}><b>${v}</b><span>${l}</span></div>`;
  const sec = (ico, title, body) => `<div class="st-sec"><div class="st-h">${I[ico]}${title}</div>${body}</div>`;

  $("stats").innerHTML = [
    sec("timer", t("st.focus"), `
      <div class="kpis">${kpi(dur(total), t("st.total"))}${kpi(sess.length, t("st.sessions"), t("st.sessionsTip", { a: done.length, b: stop.length }))}${kpi(rate + "%", t("st.completeRate"))}
      ${kpi(sess.length ? dur(total / sess.length) : dur(0), t("st.avg"))}${kpi(dur(longest), t("st.longest"))}${kpi(t("fmt.days", { n: streak }), t("st.streak"))}</div>
      <div class="chart"><div class="bars">${buckets.map((v, i) => `<div class="${v ? "" : "z"}" style="height:${(100 * v) / bmax}%" data-tip="${esc(barTip(i))}"></div>`).join("")}</div>
      <div class="axis">${labels.map((l) => `<span>${l}</span>`).join("")}</div></div>
      ${period !== 1 ? `<div class="chart"><div class="heat">${hours.map((v, h) => `<div style="opacity:${v ? 0.15 + 0.85 * (v / hmax) : 0.06}" data-tip="${t("fmt.h", { h })} · ${dur(v)}"></div>`).join("")}</div>
      <div class="axis">${[0, 6, 12, 18, 23].map((h) => `<span>${t("fmt.h", { h })}</span>`).join("")}</div></div>` : ""}`),
    sec("clock", t("st.history"), sess.length ? `<div class="hist">${sess.slice().sort((a, b) => b.t - a.t).slice(0, 15).map((e) => `
      <div class="hi"><span class="hs ${e.k === "focus_done" ? "ok" : ""}">${e.k === "focus_done" ? I.check : I.stop}</span>
        <span class="hd">${dateS(e.t)} · ${clock(e.t - e.n * 60000)}</span><b class="${e.a ? "" : "none"}">${esc(e.a || "—")}</b><span class="hm">${dur(e.n)}</span></div>`).join("")}</div>` : `<div class="st-empty">—</div>`),
    sec("gauge", t("st.sites"), `
      <div class="kpis">${kpi(dur(usageTotal / 60), t("st.onSites"))}${kpi(of("lockout").length, t("st.lockouts"))}${kpi(hits, t("st.blocked"), t("st.blockedTip"))}</div>
      ${siteNames.length ? `<div class="chart hbars">${siteNames.slice(0, 8).map((s) => `
        <div class="hb-row" data-tip="${esc(t("st.siteTip", { s, d: dur((usage[s] || 0) / 60), l: lock[s] || 0, b: (hitF[s] || 0) + (hitG[s] || 0) }))}"><b>${esc(short(s))}</b>
          <div class="track"><div style="width:${(100 * (usage[s] || 0)) / umax}%"></div></div>
          <span>${dur((usage[s] || 0) / 60)}</span></div>`).join("")}</div>` : `<div class="st-empty">—</div>`}`),
    sec("bell", t("st.reminders"), `
      <div class="kpis">${kpi(ack.length, t("st.acked"))}${kpi(miss.length, t("st.missed"))}${kpi(ackRate + "%", t("st.rate"))}</div>
      ${remNames.length ? `<div class="chart hbars">${remNames.slice(0, 8).map((r) => {
        const v = perRem[r], tot = v.a + v.m;
        return `<div class="hb-row" data-tip="${esc(t("st.remTip", { r, a: v.a, m: v.m }))}"><b>${esc(r)}</b><div class="track"><div style="width:${(100 * v.a) / Math.max(1, tot)}%"></div></div><span>${v.a}/${tot}</span></div>`;
      }).join("")}</div>` : ""}
      ${later.length ? `<div class="st-empty">${t("st.snoozed", { n: later.length })}</div>` : ""}`),
    sec("folder", t("st.dossiers"), `
      <div class="kpis">${kpi(open.length, t("st.open"))}${kpi(avgLvl + "%", t("st.avgActivity"))}${kpi(dAct, t("st.activations"))}
      ${kpi(dNew, t("st.created"))}${kpi(dDone, t("st.completed"))}${kpi(dLost, t("st.lost"))}</div>
      ${ended.length ? `<div class="st-empty">${t("st.lifespan", { n: life })}</div>` : ""}`),
  ].join("");
}

// ------------------------------------------------------------ événements
function bind() {
  icons();
  $("minBtn").innerHTML = I.minimize;
  $("closeBtn").innerHTML = I.close;
  $("gearBtn").innerHTML = I.gear;
  $("stopBtn").innerHTML = I.stop;
  $("startBtn").innerHTML = I.play;
  $("minusBtn").innerHTML = I.minus;
  $("plusBtn").innerHTML = I.plus;
  $("fabBtn").innerHTML = I.plus;

  $("minBtn").onclick = () => invoke("set_mode", { mode: "mini" });
  // Barre du haut : glisser déplace, double-clic réduit
  let lastDown = 0, lastX = 0, lastY = 0;
  $("hdr").addEventListener("mousedown", (e) => {
    if (e.button !== 0 || e.target.closest("button")) return;
    const now = Date.now();
    if (now - lastDown < 400 && Math.hypot(e.screenX - lastX, e.screenY - lastY) < 6) {
      lastDown = 0;
      return invoke("set_mode", { mode: "mini" });
    }
    lastDown = now; lastX = e.screenX; lastY = e.screenY;
    T.window.getCurrentWindow().startDragging();
  });
  $("closeBtn").onclick = () => invoke("set_mode", { mode: "closed" });
  $("gearBtn").onclick = () => showTab(tab === "settings" ? lastTab : "settings");
  $("perfBtn").innerHTML = I.activity;
  $("perfBtn").onclick = () => showTab(tab === "perf" ? lastTab : "perf");
  $("tabs").onclick = (e) => { const b = e.target.closest("button"); if (b) showTab(b.dataset.tab); };

  // Focus
  $("startBtn").onclick = startFocus;
  const fn = $("focusNote");
  const saveNote = () => { if (S.session && fn.value.trim() !== (S.session.goal || "")) invoke("set_goal", { goal: fn.value.trim() }); };
  fn.addEventListener("blur", saveNote);
  fn.addEventListener("keydown", (e) => {
    if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); fn.blur(); }
    // Entrée : avant le focus, le lance ; pendant, enregistre
    if (e.key === "Enter") { e.preventDefault(); e.stopPropagation(); if (S.session) fn.blur(); else startFocus(); }
  });
  const mi = $("minInput");
  // Au clic, tout le nombre est sélectionné : taper remplace
  let fresh = false;
  mi.addEventListener("focus", () => { fresh = true; mi.select(); });
  mi.addEventListener("mouseup", (e) => { if (fresh) { e.preventDefault(); mi.select(); fresh = false; } });
  mi.addEventListener("input", () => { mi.value = mi.value.replace(/\D/g, "").slice(0, 3); const v = parseInt(mi.value, 10); if (v > 0) { minutes = Math.min(600, v); $("ring").style.strokeDashoffset = C * (1 - Math.min(1, minutes / 60)); } });
  mi.addEventListener("blur", () => setMinutes(parseInt(mi.value, 10) || minutes));
  mi.addEventListener("keydown", (e) => {
    if (e.key === "Enter") { e.preventDefault(); startFocus(); }
    if (e.key === "ArrowUp") { e.preventDefault(); setMinutes(minutes + (e.shiftKey ? 5 : 1)); mi.value = minutes; }
    if (e.key === "ArrowDown") { e.preventDefault(); setMinutes(minutes - (e.shiftKey ? 5 : 1)); mi.value = minutes; }
  });
  $("ringWrap").addEventListener("wheel", (e) => { if (S.session) return; e.preventDefault(); setMinutes(minutes + (e.deltaY < 0 ? 1 : -1)); }, { passive: false });
  // Maintenir − ou + : ça accélère
  for (const [id, d] of [["minusBtn", -1], ["plusBtn", 1]]) {
    const b = $(id);
    let tmr = null, n = 0;
    const step = () => { setMinutes(minutes + d * (n > 12 ? 5 : 1)); n++; tmr = setTimeout(step, n === 1 ? 380 : 70); };
    const end = () => { clearTimeout(tmr); tmr = null; n = 0; };
    b.addEventListener("pointerdown", (e) => { e.preventDefault(); step(); });
    b.addEventListener("pointerup", end);
    b.addEventListener("pointerleave", end);
  }
  $("pauseBtn").onclick = () => invoke(S.session.paused ? "resume" : "pause");
  $("stopBtn").onclick = () => invoke("stop");
  $("plus5").onclick = () => invoke("add_minutes", { minutes: 5 });

  // Dossiers
  $("addDossier").onsubmit = (e) => {
    e.preventDefault();
    const v = $("dossierInput").value.trim();
    if (v) { invoke("dossier_add", { title: v }); $("dossierInput").value = ""; }
  };
  $("archBtn").onclick = () => { archOpen = !archOpen; renderDossiers(); };
  $("archSeg").onclick = (e) => { const b = e.target.closest("button"); if (b) { archMode = b.dataset.a; renderDossiers(); } };
  const board = $("board");
  board.onclick = (e) => {
    const t = e.target;
    const act = t.closest("[data-act]");
    if (act) { act.classList.add("pop"); setTimeout(() => act.classList.remove("pop"), 500); return invoke("dossier_activate", { id: +act.dataset.act }); }
    const dn = t.closest("[data-done]");
    if (dn) return invoke("dossier_set_state", { id: +dn.dataset.done, state: "done" });
    const bu = t.closest("[data-bury]");
    if (bu) return invoke("dossier_set_state", { id: +bu.dataset.bury, state: "dead" });
    const mn = t.closest("[data-menu]");
    if (mn) { openMenu = openMenu === +mn.dataset.menu ? null : +mn.dataset.menu; board.dataset.k = ""; return renderDossiers(); }
    const dc = t.closest("[data-decay]");
    if (dc) {
      const x = S.dossiers.find((y) => y.id === +dc.dataset.decay);
      return invoke("dossier_update", { id: x.id, decay: Math.max(0, Math.min(100, x.decay_eff + +dc.dataset.d)) });
    }
    const co = t.closest("[data-color]");
    if (co) { const x = S.dossiers.find((y) => y.id === +co.dataset.color); return invoke("dossier_update", { id: x.id, color: (x.color + 1) % 6 }); }
    const del = t.closest("[data-del]");
    if (del && armed(del)) { openMenu = null; return invoke("dossier_delete", { id: +del.dataset.del }); }
  };
  board.addEventListener("focusout", (e) => {
    const f = e.target.dataset && e.target.dataset.f;
    if (!f) return;
    const id = +e.target.closest(".postit").dataset.id;
    const x = S.dossiers.find((y) => y.id === id);
    const v = f === "title" ? e.target.textContent.trim() : e.target.innerText.replace(/\n$/, "");
    if (x && v !== x[f] && (f !== "title" || v)) invoke("dossier_update", { id, [f]: v });
    board.dataset.k = "";
  });
  board.addEventListener("keydown", (e) => {
    if (e.target.dataset.f === "title" && e.key === "Enter") { e.preventDefault(); e.target.blur(); }
    if (e.key === "Escape") { e.stopPropagation(); e.target.blur(); }
  });
  $("archList").onclick = (e) => {
    const ro = e.target.closest("[data-reopen]");
    if (ro) return invoke("dossier_set_state", { id: +ro.dataset.reopen, state: "open" });
    const del = e.target.closest("[data-del]");
    if (del) { if (armed(del)) invoke("dossier_delete", { id: +del.dataset.del }); return; }
    const ex = e.target.closest("[data-exp]");
    if (ex) { archExpanded = archExpanded === +ex.dataset.exp ? null : +ex.dataset.exp; renderDossiers(); }
  };

  // Jauges. Bouton à gauche du champ : ajouter avec jauge ou en focus seulement.
  $("siteMode").onclick = () => { addFocusOnly = !addFocusOnly; renderSiteMode(); $("siteInput").focus(); };
  renderSiteMode();
  $("addSite").onsubmit = (e) => {
    e.preventDefault();
    const v = $("siteInput").value.trim().toLowerCase().replace(/^https?:\/\//, "").replace(/^www\./, "").split(/[/?#:]/)[0];
    if (v && v.includes(".")) saveSettings((c) => {
      if (!c.sites.includes(v)) c.sites.push(v);
      if (addFocusOnly) c.gauge_cfg[v] = { ...GDEF, ...c.gauge_cfg[v], enabled: false };
    });
    $("siteInput").value = "";
  };
  $("siteList").onclick = (e) => {
    const t = e.target;
    const del = t.closest("[data-del]");
    if (del) { if (armed(del)) { openSite = null; saveSettings((c) => { const [x] = c.sites.splice(+del.dataset.del, 1); c.sites_off = c.sites_off.filter((o) => o !== x); }); } return; }
    const tg = t.closest("[data-tog]");
    if (tg) return saveSettings((c) => { const x = c.sites[+tg.dataset.tog]; c.sites_off = c.sites_off.includes(x) ? c.sites_off.filter((o) => o !== x) : [...c.sites_off, x]; });
    const gon = t.closest("[data-gon]");
    if (gon) return saveSettings((c) => { const x = c.sites[+gon.dataset.gon]; const g = { ...GDEF, ...c.gauge_cfg[x] }; g.enabled = !g.enabled; c.gauge_cfg[x] = g; });
    const st = t.closest("[data-g]");
    if (st) return saveSettings((c) => {
      const x = c.sites[+st.dataset.g]; const g = { ...GDEF, ...c.gauge_cfg[x] }; const k = st.dataset.k;
      const lim = { max_min: [5, 240], drain: [0.5, 5], refill_per_hour: [5, 120] }[k];
      g[k] = Math.round(Math.max(lim[0], Math.min(lim[1], g[k] + +st.dataset.d)) * 10) / 10;
      c.gauge_cfg[x] = g;
    });
    const o = t.closest("[data-open]");
    if (o) { const x = S.settings.sites[+o.dataset.open]; openSite = openSite === x ? null : x; renderSites(); }
  };

  // Rappels
  $("masterRow").onclick = () => invoke("reminders_set_on", { on: !S.settings.reminders_on });
  const remClick = (e) => {
    const t = e.target;
    const del = t.closest("[data-rdel]");
    if (del) { if (armed(del)) invoke("reminder_delete", { id: del.dataset.rdel }); return; }
    const on = t.closest("[data-ron]");
    if (on) { const r = S.settings.reminders.find((x) => x.id === on.dataset.ron); return invoke("reminder_update", { id: r.id, enabled: !r.enabled }); }
    const ev = t.closest("[data-every]");
    if (ev) { const r = S.settings.reminders.find((x) => x.id === ev.dataset.every); return invoke("reminder_update", { id: r.id, everyMin: stepEvery(r.every_min, +ev.dataset.d) }); }
    const ic = t.closest("[data-ico]");
    if (ic) return openPicker(ic, S.settings.reminders.find((x) => x.id === ic.dataset.ico));
  };
  $("remList").onclick = remClick;
  $("onceList").onclick = remClick;
  $("remList").addEventListener("focusout", (e) => {
    const id = e.target.dataset && e.target.dataset.label;
    if (!id) return;
    const v = e.target.textContent.trim();
    const r = S.settings.reminders.find((x) => x.id === id);
    if (r && v && v !== r.label) invoke("reminder_update", { id, label: v });
    $("remList").dataset.k = "";
  });
  $("remList").addEventListener("keydown", (e) => { if (e.target.dataset.label && e.key === "Enter") { e.preventDefault(); e.target.blur(); } });
  // Rappel unique : quoi + quand, Entrée ou +
  const addOnce = () => {
    const at = parseWhen($("onceWhen").value);
    if (at === null) { $("onceWhen").focus(); return; }
    quickAt(at, "onceInput");
    $("onceWhen").value = "";
  };
  $("onceAdd").onsubmit = (e) => { e.preventDefault(); addOnce(); };
  bindWhen($("onceWhen"), addOnce);
  $("missStep").onclick = (e) => {
    const b = e.target.closest("[data-ms]"); if (!b) return;
    saveSettings((c) => (c.misses_to_pause = Math.max(1, Math.min(20, (c.misses_to_pause || 2) + +b.dataset.ms))));
  };
  $("pendList").onclick = (e) => {
    const c = e.target.closest("[data-pclaim]");
    if (c) return invoke("pending_claim", { rid: c.dataset.pclaim });
    const d = e.target.closest("[data-pdrop]");
    if (d && armed(d)) invoke("pending_drop", { rid: d.dataset.pdrop });
  };
  $("addRem").onsubmit = (e) => {
    e.preventDefault();
    const v = $("remInput").value.trim();
    if (v) { invoke("reminder_add", { label: v, everyMin: 30 }); $("remInput").value = ""; }
  };

  // Stats
  $("period").onclick = (e) => {
    const b = e.target.closest("button"); if (!b) return;
    period = +b.dataset.p;
    document.querySelectorAll("#period button").forEach((x) => x.classList.toggle("sel", x === b));
    renderStats();
  };

  // Réglages
  $("autoRow").onclick = async () => { S.autostart = await invoke("set_autostart", { on: !S.autostart }); renderSettings(); };
  $("soundRow").onclick = () => saveSettings((c) => (c.sound = !c.sound));
  $("decayStep").onclick = (e) => { const b = e.target.closest("[data-dd]"); if (b) saveSettings((c) => (c.dossier_decay = Math.max(1, Math.min(50, c.dossier_decay + +b.dataset.dd)))); };
  $("openExt").onclick = () => invoke("open_extension_folder");
  // Raccourcis : clic sur la ligne, puis la combinaison voulue
  document.querySelectorAll(".row.sc").forEach((row) => {
    row.onclick = () => {
      scListen = row.dataset.sc;
      document.querySelectorAll(".row.sc").forEach((r) => r.classList.toggle("listening", r === row));
      row.querySelector("kbd").textContent = "…";
    };
  });
  document.addEventListener("keydown", async (e) => {
    if (!scListen) return;
    e.preventDefault(); e.stopPropagation();
    if (e.key === "Escape") { scListen = null; document.querySelectorAll(".row.sc").forEach((r) => r.classList.remove("listening")); return renderSettings(); }
    if (["Control", "Alt", "Shift", "Meta", "AltGraph"].includes(e.key)) return;
    const mods = [e.ctrlKey && "ctrl", e.altKey && "alt", e.shiftKey && "shift", e.metaKey && "super"].filter(Boolean);
    if (!mods.length) return;
    const code = e.code.startsWith("Key") ? e.code.slice(3) : e.code.startsWith("Digit") ? e.code.slice(5) : e.code;
    const kind = scListen, accel = [...mods, code.toLowerCase()].join("+");
    scListen = null;
    document.querySelectorAll(".row.sc").forEach((r) => r.classList.remove("listening"));
    try { await invoke("set_shortcut", { kind, accel }); toast(accel.replace(/\+/g, " ")); S = await invoke("get_state"); }
    catch (err) { toast(String(err)); }
    renderSettings();
  }, true);
  $("updBtn").onclick = async () => {
    if (updBusy) return;
    updBusy = true;
    if (S.update) {
      $("updTxt").textContent = t("set.updInstalling", { v: S.update });
      try { await invoke("install_update"); } catch (e) { $("updTxt").textContent = String(e); }
    } else {
      $("updTxt").textContent = t("set.updChecking");
      try { S.update = await invoke("check_update"); } catch { S.update = null; }
      if (!S.update) $("updTxt").textContent = t("set.upToDate");
    }
    updBusy = false;
    renderSettings();
  };
  $("about").onclick = (e) => { if (e.target.closest("#openData")) invoke("open_data_folder"); };
  $("quitBtn").onclick = (e) => { if (armed(e.currentTarget)) invoke("quit"); };
  $("quitBtn").classList.add("del");

  // Infobulles maison : quasi instantanées. Les title natifs sont convertis au survol.
  let tipEl = null, tipTimer = null;
  const tip = $("tip");
  tip.classList.remove("hidden");
  const placeTip = () => {
    if (!tipEl || !document.body.contains(tipEl)) return hideTip();
    const r = tipEl.getBoundingClientRect(), t = tip.getBoundingClientRect();
    let x = r.left + r.width / 2 - t.width / 2;
    x = Math.max(16, Math.min(window.innerWidth - t.width - 16, x));
    let y = r.top - t.height - 9, below = false;
    if (y < 14) { y = r.bottom + 9; below = true; }
    tip.style.left = `${x}px`; tip.style.top = `${y}px`;
    tip.style.setProperty("--ax", `${r.left + r.width / 2 - x}px`);
    tip.classList.toggle("below", below);
  };
  const hideTip = () => { clearTimeout(tipTimer); tipEl = null; tip.classList.remove("show"); };
  document.addEventListener("mouseover", (e) => {
    let el = e.target.closest("[title], [data-tip]");
    if (el && el.hasAttribute("title")) { if (el.title) el.dataset.tip = el.title; el.removeAttribute("title"); }
    if (el && !el.dataset.tip) el = null;
    if (el === tipEl) return;
    hideTip();
    if (!el) return;
    tipEl = el;
    tipTimer = setTimeout(() => { tip.textContent = el.dataset.tip; placeTip(); tip.classList.add("show"); }, 120);
  });
  document.addEventListener("mousedown", hideTip);
  document.addEventListener("scroll", hideTip, true);

  // Actions rapides (bas droite)
  $("fabBtn").onclick = () => { if (!$("qpanel").classList.contains("hidden")) return closeQuick(); $("fab").classList.toggle("open"); };
  $("fabMenu").onclick = (e) => {
    const b = e.target.closest("[data-qa]"); if (!b) return;
    $("fab").classList.remove("open");
    if (b.dataset.qa === "focus") { if (!S.session) startFocus(); showTab("focus"); }
    else openQuick(b.dataset.qa);
  };
  const submitQuick = () => {
    if (quickKind === "dos") {
      const v = $("qInput").value.trim();
      if (v) { invoke("dossier_add", { title: v }); toast(v); }
      return closeQuick();
    }
    const at = parseWhen($("qWhen").value);
    if (at === null) return $("qWhen").focus();
    quickAt(at);
  };
  $("qpanel").onsubmit = (e) => { e.preventDefault(); submitQuick(); };
  $("qpanel").addEventListener("keydown", (e) => { if (e.key === "Escape") { e.stopPropagation(); closeQuick(); } });
  bindWhen($("qWhen"), submitQuick);

  // Langue
  $("langSel").onchange = async () => {
    const r = await invoke("set_lang", { lang: $("langSel").value });
    setLang(r.i18n, r.langs);
  };
  $("langImport").onclick = () => $("langFile").click();
  $("langFile").onchange = async () => {
    const f = $("langFile").files[0];
    $("langFile").value = "";
    if (!f) return;
    try {
      const name = await invoke("import_locale", { content: await f.text() });
      await refresh();
      toast(t("set.langImported", { n: name }));
    } catch (err) { toast(String(err)); }
  };
  $("langFolder").onclick = () => invoke("open_locales_folder");
  document.addEventListener("mousedown", (e) => {
    if (!e.target.closest("#fab")) $("fab").classList.remove("open");
    if (!e.target.closest("#picker") && !e.target.closest("[data-ico]")) $("picker").classList.add("hidden");
  });
  $("picker").onclick = (e) => {
    const b = e.target.closest("[data-pick]"); if (!b) return;
    invoke("reminder_update", { id: $("picker").dataset.rid, icon: b.dataset.pick });
    $("picker").classList.add("hidden");
  };

  // Clavier
  document.addEventListener("keydown", (e) => {
    const typing = /INPUT|TEXTAREA/.test(document.activeElement.tagName) || document.activeElement.isContentEditable;
    if (e.key === "Escape" && !typing) {
      if (!$("qpanel").classList.contains("hidden")) closeQuick();
      else if (tab === "settings") showTab(lastTab);
      else invoke("set_mode", { mode: "mini" });
    }
    if (e.key === " " && !typing && tab === "focus" && S.session) { e.preventDefault(); $("pauseBtn").click(); }
  });
}

// ---------------------------------------------------- performance
const mb = (b) => (b >= 1073741824 ? `${(b / 1073741824).toFixed(1)} ${t("unit.gb")}` : `${Math.round(b / 1048576)} ${t("unit.mb")}`);
const kb = (b) => (b >= 1048576 ? `${(b / 1048576).toFixed(1)} ${t("unit.mb")}` : `${Math.max(1, Math.round(b / 1024))} ${t("unit.kb")}`);
function spark(svg, vals, max) {
  const n = 60, w = 300, h = 60;
  const pts = vals.slice(-n).map((v, i, a) => [(w * (i + n - a.length)) / (n - 1), h - 4 - ((h - 8) * v) / max]);
  if (pts.length < 2) { svg.innerHTML = ""; return; }
  const line = pts.map((p) => p.map((x) => x.toFixed(1)).join(",")).join(" ");
  svg.innerHTML = `<polygon points="${pts[0][0]},${h} ${line} ${pts[pts.length - 1][0]},${h}" class="sa"/><polyline points="${line}" class="sl"/>`;
}
async function pollPerf() {
  let p;
  try { p = await invoke("get_perf"); } catch { return; }
  perfHist.push(p);
  if (perfHist.length > 60) perfHist.shift();
  $("pfCpu").textContent = `${p.cpu.toFixed(1)} %`;
  $("pfMem").textContent = mb(p.mem);
  $("pfProcs").textContent = p.procs;
  const cpus = perfHist.map((x) => x.cpu), mems = perfHist.map((x) => x.mem / 1048576);
  const cmax = Math.max(5, ...cpus), mmax = Math.max(50, ...mems) * 1.15;
  spark($("pfCpuSpark"), cpus, cmax);
  spark($("pfMemSpark"), mems, mmax);
  $("pfCpuMax").textContent = t("pf.max", { v: `${Math.max(...cpus).toFixed(1)} %` });
  $("pfMemMax").textContent = t("pf.max", { v: mb(Math.max(...perfHist.map((x) => x.mem))) });
  const up = Math.floor(p.uptime / 1000);
  setHTML($("pfDetails"), `
    <div class="ab"><span>${t("pf.capOnly")}</span><b>${p.cap_cpu.toFixed(1)} % · ${mb(p.cap_mem)}</b></div>
    <div class="ab"><span>${t("pf.display")}</span><b>${(p.cpu - p.cap_cpu).toFixed(1)} % · ${mb(p.mem - p.cap_mem)}</b></div>
    <div class="ab"><span>${t("pf.windows")}</span><b>${(p.windows || []).map((w) => t(`pf.win.${w}`)).join(" · ") || "—"}</b></div>
    <div class="ab" title="${t("pf.workingTip")}"><span>${t("pf.working")}</span><b>${mb(p.working || 0)}</b></div>
    <div class="ab"><span>${t("pf.uptime")}</span><b>${t("fmt.hm", { h: Math.floor(up / 3600), m: String(Math.floor((up % 3600) / 60)).padStart(2, "0") })}</b></div>
    <div class="ab"><span>${t("pf.data")}</span><b>${t("pf.events", { s: kb(p.data), n: p.events })}</b></div>
    <div class="ab"><span>${t("pf.cores")}</span><b>${p.cores}</b></div>`);
}

let quickKind = "rem";
function openQuick(kind = "rem") {
  quickKind = kind;
  $("fab").classList.remove("open");
  $("qInput").placeholder = kind === "dos" ? t("qa.dosPh") : t("qa.remPh");
  $("qWhen").classList.toggle("hidden", kind === "dos");
  $("qWhen").value = "";
  $("qpanel").classList.remove("hidden");
  $("fabBtn").innerHTML = I.close;
  setTimeout(() => $("qInput").focus(), 30);
}
function closeQuick() {
  $("qpanel").classList.add("hidden");
  $("qInput").value = "";
  $("fabBtn").innerHTML = I.plus;
}

function openPicker(btn, r) {
  const p = $("picker");
  p.dataset.rid = r.id;
  p.innerHTML = window.REM_ICONS.map((k) => `<button data-pick="${k}" class="${k === r.icon ? "sel" : ""}">${I[k]}</button>`).join("");
  p.classList.remove("hidden");
  const c = $("card").getBoundingClientRect(), b = btn.getBoundingClientRect(), pr = p.getBoundingClientRect();
  let top = b.bottom - c.top + 6;
  if (top + pr.height > c.height - 10) top = b.top - c.top - pr.height - 6;
  p.style.left = `${Math.min(c.width - pr.width - 10, b.left - c.left)}px`;
  p.style.top = `${top}px`;
}

function setLang(dict, langs) {
  if (!dict) return;
  L = dict;
  if (langs) S.langs = langs;
  applyI18n();
  // vider les caches de rendu pour tout redessiner dans la nouvelle langue
  document.querySelectorAll("[data-k]").forEach((el) => (el.dataset.k = ""));
  render();
}

async function refresh() {
  S = await invoke("get_state");
  $("verTag").textContent = S.about ? S.about.version : "";
  if (!primed) { minutes = S.settings.minutes || 25; primed = true; }
  if (S.i18n && JSON.stringify(S.i18n) !== JSON.stringify(L)) setLang(S.i18n, S.langs); else render();
  if (S.quick) openQuick("rem");
}

bind();
showTab("dossiers");
refresh();
T.event.listen("tick", (e) => { const keep = S ? { autostart: S.autostart, about: S.about, langs: S.langs, i18n: S.i18n } : {}; S = Object.assign(e.payload, keep); if (!primed) refresh(); else render(); });
// La langue du système sert quand le réglage est « Automatique »
invoke("set_system_lang", { lang: navigator.language || "en" }).then(refresh).catch(() => {});
T.event.listen("quick", () => openQuick("rem"));
document.addEventListener("visibilitychange", () => { if (!document.hidden && primed) refresh(); });
