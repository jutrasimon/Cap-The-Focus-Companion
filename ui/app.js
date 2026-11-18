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
let tab = "focus", lastTab = "focus";
let minutes = 25, primed = false;
let archOpen = false, archMode = "done", archExpanded = null;
let openSite = null, openMenu = null, openRem = null;
let period = 7, stats = null, statsAt = 0;
let updBusy = false;

// ------------------------------------------------------------ utilitaires
const fmt = (ms) => {
  const s = Math.max(0, Math.ceil(ms / 1000));
  const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), r = s % 60;
  return h ? `${h}:${String(m).padStart(2, "0")}:${String(r).padStart(2, "0")}` : `${m}:${String(r).padStart(2, "0")}`;
};
const dur = (min) => {
  min = Math.round(min);
  if (min < 60) return `${min} min`;
  const h = Math.floor(min / 60), m = min % 60;
  return m ? `${h} h ${String(m).padStart(2, "0")}` : `${h} h`;
};
const clock = (t) => new Date(t).toLocaleTimeString("fr-CA", { hour: "2-digit", minute: "2-digit" });
const dateS = (t) => new Date(t).toLocaleDateString("fr-CA", { day: "numeric", month: "short" });
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
  const t = document.createElement("div");
  t.className = "toast";
  t.innerHTML = I.check;
  t.appendChild(document.createTextNode(text));
  $("card").appendChild(t);
  setTimeout(() => t.remove(), 1800);
}

// ------------------------------------------------------------ navigation
function showTab(t) {
  if (t !== "settings") lastTab = t;
  tab = t;
  document.querySelectorAll(".view").forEach((v) => {
    const on = v.dataset.view === t;
    v.classList.toggle("hidden", !on);
    if (on) { v.classList.remove("enter"); void v.offsetWidth; v.classList.add("enter"); }
  });
  document.querySelectorAll("#tabs button").forEach((b) => b.classList.toggle("sel", b.dataset.tab === t));
  $("gearBtn").classList.toggle("on", t === "settings");
  if (t === "stats") loadStats();
  if (t === "settings") refresh();
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
  invoke("start", { minutes });
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
      <div class="pf"><button class="bury" data-bury="${x.id}" title="Envoyer au cimetière">${I.skull}<span>Archiver</span></button></div>
    </div>`;
  }
  const menu = openMenu === x.id ? `
      <div class="pmenu">
        <div class="stepper"><button data-decay="${x.id}" data-d="-1" title="Déclin plus lent">${I.minus}</button><span title="Déclin par jour">${x.decay_eff}%/j</span><button data-decay="${x.id}" data-d="1" title="Déclin plus rapide">${I.plus}</button></div>
        <button class="mini-btn" data-color="${x.id}" title="Couleur">${I.palette}</button>
        <button class="mini-btn del" data-del="${x.id}" title="Supprimer">${I.trash}</button>
      </div>` : "";
  const days = x.ms_left < 9e15 ? Math.ceil(x.ms_left / DAY) : "∞";
  return `<div class="postit" style="${style}" data-id="${x.id}">
      <div class="pt" contenteditable="plaintext-only" data-f="title" spellcheck="false">${esc(x.title)}</div>
      <div class="pn" contenteditable="plaintext-only" data-f="notes" spellcheck="false">${esc(x.notes)}</div>
      <div class="pf">
        <button class="actif ${lvl < 25 ? "low" : ""}" data-act="${x.id}" title="Actif · ${days} j avant de perdre ce dossier"><span class="af" style="width:${lvl}%"></span>${I.flame}<span>${lvl}%</span></button>
        <button class="mini-btn" data-done="${x.id}" title="Compléter">${I.check}</button>
        <button class="mini-btn" data-menu="${x.id}" title="Options">${I.more}</button>
      </div>${menu}
    </div>`;
}

function renderDossiers() {
  $("board").classList.toggle("hidden", archOpen);
  $("archives").classList.toggle("hidden", !archOpen);
  $("archBtn").classList.toggle("on", archOpen);
  $("archBtn").innerHTML = archOpen ? I.folder : I.archive;
  $("archBtn").title = archOpen ? "Dossiers en cours" : "Archives";
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
          ${archMode === "done" ? `<button class="icon-btn" data-reopen="${x.id}" title="Rouvrir">${I.undo}</button>` : ""}
          <button class="icon-btn del" data-del="${x.id}" title="Supprimer">${I.trash}</button></div>
        <div class="arch-meta"><span title="Créé → terminé">${I.clock}${dateS(x.created)} → ${dateS(x.ended || S.now)}</span><span title="Durée de vie">${life} j</span><span title="Activations">${I.flame}${x.activations}</span></div>
        ${archExpanded === x.id && x.notes ? `<div class="arch-notes">${esc(x.notes)}</div>` : ""}
      </div>`;
    }).join("") : `<div class="empty-ico" title="Vide">${archMode === "done" ? I.check : I.skull}</div>`);
  }
}

// ---------------------------------------------------- jauges
const gaugeOf = (site) => (S.gauges || {})[site] || { level: 0, max: 900, locked: false, using: false, cfg: GDEF, off: false, refill_in: 0 };

function renderSites() {
  const st = S.settings;
  const html = st.sites.map((site, i) => {
    const g = gaugeOf(site);
    const c = { ...GDEF, ...(st.gauge_cfg || {})[site] };
    const off = st.sites_off.includes(site);
    const pct = Math.max(0, Math.min(100, (100 * g.level) / g.max));
    const label = off ? "" : !c.enabled ? I.lock : g.locked ? `${I.lock}${Math.ceil(g.refill_in / 60)} min` : `${Math.floor(g.level / 60)} / ${c.max_min} min`;
    const body = openSite === site ? `
      <div class="site-b">
        <i>${I.gauge}</i><span>Jauge</span><div class="switch end ${c.enabled ? "on" : ""}" data-gon="${i}"></div>
        <i>${I.size}</i><span>Taille</span><div class="stepper end"><button data-g="${i}" data-k="max_min" data-d="-5">${I.minus}</button><span>${c.max_min} min</span><button data-g="${i}" data-k="max_min" data-d="5">${I.plus}</button></div>
        <i>${I.down}</i><span>Usure</span><div class="stepper end"><button data-g="${i}" data-k="drain" data-d="-0.5">${I.minus}</button><span>×${c.drain}</span><button data-g="${i}" data-k="drain" data-d="0.5">${I.plus}</button></div>
        <i>${I.up}</i><span>Recharge</span><div class="stepper end"><button data-g="${i}" data-k="refill_per_hour" data-d="-5">${I.minus}</button><span>${c.refill_per_hour} min/h</span><button data-g="${i}" data-k="refill_per_hour" data-d="5">${I.plus}</button></div>
        <div class="foot"><button class="icon-btn del" data-del="${i}" title="Retirer">${I.trash}</button></div>
      </div>` : "";
    return `<div class="site ${off ? "off" : ""}">
      <div class="site-h" data-open="${i}">
        <div class="site-m"><div class="site-n">${esc(site)}<span>${label}</span></div>
          ${!off && c.enabled ? `<div class="gbar"><div class="gfill ${pct < 25 ? "low" : ""} ${g.using ? "using" : ""}" style="width:${pct}%"></div></div>` : ""}</div>
        <div class="switch ${off ? "" : "on"}" data-tog="${i}" title="Actif"></div>
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
  $("missedTag").innerHTML = on ? `${I.bellOff}${S.missed} raté` : `${I.bellOff}en pause`;
  $("missedTag").dataset.tip = on ? "Encore un rappel raté et tous les rappels se mettent en pause" : `${S.missed} rappels ratés de suite`;


  const once = S.settings.reminders.filter((r) => r.once_at).sort((a, b) => a.once_at - b.once_at);
  if (!$("onceAdd").dataset.k) {
    $("onceAdd").dataset.k = 1;
    $("onceChips").innerHTML = [5, 15, 30, 60].map((m) => `<button type="button" data-q="${m}" title="Dans ${m < 60 ? m + " min" : "1 h"}">${m < 60 ? m + "<small>m</small>" : "1 h"}</button>`).join("") +
      `<label title="À une heure précise">${I.clock}<input type="time" id="onceTime" /></label>`;
  }
  setHTML($("onceList"), once.map((r) => `
    <div class="rem"><span class="ri">${I.bell}</span><span class="rl" style="cursor:default">${esc(r.label)}</span>
      <span class="when" title="À ${clock(r.once_at)}">${I.clock}${soon(r.once_at - S.now)}</span>
      <button class="icon-btn del" data-rdel="${r.id}" title="Supprimer">${I.trash}</button></div>`).join(""));

  if (editingIn($("remList"))) return;
  const rec = S.settings.reminders.filter((r) => !r.once_at);
  setHTML($("remList"), rec.map((r) => `
    <div class="rem ${r.enabled ? "" : "off"}" data-id="${r.id}">
      <button class="ri" data-ico="${r.id}" title="Icône">${I[r.icon] || I.bell}</button>
      <div class="rt"><span class="rl" contenteditable="plaintext-only" data-label="${r.id}" spellcheck="false">${esc(r.label)}</span>
        <span class="rn">${!r.enabled || !on ? "—" : S.due && S.due[r.id] != null ? `${I.clock}${soon(S.due[r.id])}` : ""}</span></div>
      <div class="stepper" title="Intervalle"><button data-every="${r.id}" data-d="-1">${I.minus}</button><span>${dur(r.every_min)}</span><button data-every="${r.id}" data-d="1">${I.plus}</button></div>
      <div class="switch ${r.enabled ? "on" : ""}" data-ron="${r.id}"></div>
      <button class="icon-btn del" data-rdel="${r.id}" title="Supprimer">${I.trash}</button>
    </div>`).join(""));
}

// "45 s", "12 min", "1 h 05"
const soon = (ms) => {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s} s`;
  const m = Math.ceil(s / 60);
  return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")}`;
};

function quickAt(at, src = "qInput") {
  const label = $(src).value.trim();
  $(src).value = "";
  invoke("reminder_once", { label, at });
  closeQuick();
  toast(`${label || "Rappel"} · ${clock(at)}`);
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
  setHTML($("decayStep"), `<button data-dd="-1">${I.minus}</button><span>${S.settings.dossier_decay}%/j</span><button data-dd="1">${I.plus}</button>`);
  if (!updBusy) {
    $("updRow").classList.toggle("avail", !!S.update);
    $("updTxt").textContent = S.update ? `Version ${S.update} disponible` : "À jour";
    $("updBtn").innerHTML = S.update ? I.down : I.repeat;
    $("updBtn").dataset.tip = S.update ? "Installer et relancer" : "Vérifier maintenant";
  }
  const ab = S.about;
  if (!ab) return;
  const list = ab.browsers || [];
  setHTML($("browserList"), list.length ? list.map((b) => {
    const old = b.version !== ab.ext_version; // une extension sans version est forcément ancienne
    return `<div class="row"><b>${esc(BNAME[b.name] || b.name)}</b><span class="muted">${esc(b.version || "?")}</span>
      <span class="${old ? "warn" : "okc"}" title="${old ? `Extension ${esc(b.version || "ancienne")} · recharge-la pour passer à ${esc(ab.ext_version)}` : "À jour"}">${old ? I.repeat : I.check}</span></div>`;
  }).join("") : `<div class="row"><b class="muted">Aucun navigateur connecté</b></div>`);
  setHTML($("about"), `
    <div class="ab"><span>Cap</span><b>${esc(ab.version)}</b></div>
    <div class="ab"><span>Compilé le</span><b>${new Date(ab.built).toLocaleString("fr-CA", { dateStyle: "medium", timeStyle: "short" })}</b></div>
    <div class="ab"><span>Extension livrée</span><b>${esc(ab.ext_version)}</b></div>
    <div class="ab"><span>Données</span><b class="path" title="${esc(ab.data_dir)}">${esc(ab.data_dir)}</b><button class="icon-btn sm" id="openData" title="Ouvrir le dossier">${I.folder}</button></div>`);
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
    labels = ["0 h", "6 h", "12 h", "18 h", "23 h"];
  } else {
    const nb = Math.min(days, 60);
    const start = dayStart(now) - (nb - 1) * DAY;
    buckets = Array(nb).fill(0);
    sess.forEach((e) => { const i = Math.floor((dayStart(e.t) - start) / DAY); if (i >= 0 && i < nb) buckets[i] += e.n; });
    labels = [dateS(start), dateS(now)];
  }
  const bmax = Math.max(1, ...buckets);
  const barTip = (i) => period === 1 ? `${i} h · ${dur(buckets[i])}` : `${dateS(dayStart(now) - (buckets.length - 1 - i) * DAY)} · ${dur(buckets[i])}`;

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
    sec("timer", "Focus", `
      <div class="kpis">${kpi(dur(total), "total")}${kpi(sess.length, "sessions", `${done.length} complètes · ${stop.length} écourtées`)}${kpi(rate + "%", "complètes")}
      ${kpi(sess.length ? dur(total / sess.length) : "0 min", "moyenne")}${kpi(dur(longest), "plus longue")}${kpi(streak + " j", "série")}</div>
      <div class="chart"><div class="bars">${buckets.map((v, i) => `<div class="${v ? "" : "z"}" style="height:${(100 * v) / bmax}%" data-tip="${esc(barTip(i))}"></div>`).join("")}</div>
      <div class="axis">${labels.map((l) => `<span>${l}</span>`).join("")}</div></div>
      ${period !== 1 ? `<div class="chart"><div class="heat">${hours.map((v, h) => `<div style="opacity:${v ? 0.15 + 0.85 * (v / hmax) : 0.06}" data-tip="${h} h · ${dur(v)}"></div>`).join("")}</div>
      <div class="axis"><span>0 h</span><span>6 h</span><span>12 h</span><span>18 h</span><span>23 h</span></div></div>` : ""}`),
    sec("gauge", "Sites", `
      <div class="kpis">${kpi(dur(usageTotal / 60), "sur les sites")}${kpi(of("lockout").length, "jauges à sec")}${kpi(hits, "bloqués", "Tentatives d'ouverture bloquées")}</div>
      ${siteNames.length ? `<div class="chart hbars">${siteNames.slice(0, 8).map((s) => `
        <div class="hb-row" data-tip="${esc(`${s} · ${dur((usage[s] || 0) / 60)} · ${lock[s] || 0} à sec · ${(hitF[s] || 0) + (hitG[s] || 0)} bloqués`)}"><b>${esc(short(s))}</b>
          <div class="track"><div style="width:${(100 * (usage[s] || 0)) / umax}%"></div></div>
          <span>${dur((usage[s] || 0) / 60)}</span></div>`).join("")}</div>` : `<div class="st-empty">—</div>`}`),
    sec("bell", "Rappels", `
      <div class="kpis">${kpi(ack.length, "confirmés")}${kpi(miss.length, "ratés")}${kpi(ackRate + "%", "taux")}</div>
      ${remNames.length ? `<div class="chart hbars">${remNames.slice(0, 8).map((r) => {
        const v = perRem[r], t = v.a + v.m;
        return `<div class="hb-row" data-tip="${esc(`${r} · ${v.a} confirmés · ${v.m} ratés`)}"><b>${esc(r)}</b><div class="track"><div style="width:${(100 * v.a) / Math.max(1, t)}%"></div></div><span>${v.a}/${t}</span></div>`;
      }).join("")}</div>` : ""}
      ${later.length ? `<div class="st-empty">${later.length} reportés</div>` : ""}`),
    sec("folder", "Dossiers", `
      <div class="kpis">${kpi(open.length, "en cours")}${kpi(avgLvl + "%", "activité moy.")}${kpi(dAct, "activations")}
      ${kpi(dNew, "créés")}${kpi(dDone, "complétés")}${kpi(dLost, "perdus")}</div>
      ${ended.length ? `<div class="st-empty">Durée de vie moyenne : ${life} j</div>` : ""}`),
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
  $("closeBtn").onclick = () => invoke("set_mode", { mode: "closed" });
  $("gearBtn").onclick = () => showTab(tab === "settings" ? lastTab : "settings");
  $("tabs").onclick = (e) => { const b = e.target.closest("button"); if (b) showTab(b.dataset.tab); };

  // Focus
  $("startBtn").onclick = startFocus;
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
    let t = null, n = 0;
    const step = () => { setMinutes(minutes + d * (n > 12 ? 5 : 1)); n++; t = setTimeout(step, n === 1 ? 380 : 70); };
    const end = () => { clearTimeout(t); t = null; n = 0; };
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

  // Jauges
  $("addSite").onsubmit = (e) => {
    e.preventDefault();
    const v = $("siteInput").value.trim().toLowerCase().replace(/^https?:\/\//, "").replace(/^www\./, "").split(/[/?#:]/)[0];
    if (v && v.includes(".")) saveSettings((c) => { if (!c.sites.includes(v)) c.sites.push(v); });
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
  const onceAt = (at) => quickAt(at, "onceInput");
  $("onceChips").onclick = (e) => { const b = e.target.closest("[data-q]"); if (b) onceAt(Date.now() + +b.dataset.q * 60000); };
  $("onceChips").addEventListener("change", (e) => {
    if (e.target.id !== "onceTime" || !e.target.value) return;
    const [h, m] = e.target.value.split(":").map(Number);
    const d = new Date(); d.setHours(h, m, 0, 0);
    if (d.getTime() <= Date.now()) d.setDate(d.getDate() + 1);
    onceAt(d.getTime());
    e.target.value = "";
  });
  $("onceInput").addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); onceAt(Date.now() + 15 * 60000); } });
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
  $("updBtn").onclick = async () => {
    if (updBusy) return;
    updBusy = true;
    if (S.update) {
      $("updTxt").textContent = `Installation de ${S.update}…`;
      try { await invoke("install_update"); } catch (e) { $("updTxt").textContent = String(e); }
    } else {
      $("updTxt").textContent = "Vérification…";
      try { S.update = await invoke("check_update"); } catch { S.update = null; }
      if (!S.update) $("updTxt").textContent = "À jour";
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
  $("qInput").addEventListener("keydown", (e) => {
    if (e.key === "Escape") { e.stopPropagation(); closeQuick(); }
    if (e.key !== "Enter") return;
    e.preventDefault();
    if (quickKind === "dos") { const v = $("qInput").value.trim(); if (v) { invoke("dossier_add", { title: v }); toast(v); } closeQuick(); }
    else quickAt(Date.now() + 15 * 60000);
  });
  $("qChips").onclick = (e) => {
    const b = e.target.closest("[data-q]"); if (b) quickAt(Date.now() + +b.dataset.q * 60000);
    const a = e.target.closest("[data-qadd]"); if (a) { const v = $("qInput").value.trim(); if (v) { invoke("dossier_add", { title: v }); toast(v); } closeQuick(); }
  };
  $("qChips").addEventListener("change", (e) => {
    if (e.target.id !== "qTime" || !e.target.value) return;
    const [h, m] = e.target.value.split(":").map(Number);
    const d = new Date(); d.setHours(h, m, 0, 0);
    if (d.getTime() <= Date.now()) d.setDate(d.getDate() + 1);
    quickAt(d.getTime());
  });
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

let quickKind = "rem";
function openQuick(kind = "rem") {
  quickKind = kind;
  $("fab").classList.remove("open");
  $("qInput").placeholder = kind === "dos" ? "Dossier" : "Rappel";
  $("qChips").innerHTML = kind === "dos"
    ? `<button data-qadd="1" title="Ajouter">${I.plus}</button>`
    : [5, 15, 30, 60].map((m) => `<button data-q="${m}" title="Dans ${m < 60 ? m + " min" : "1 h"}">${m < 60 ? m + "<small>m</small>" : "1 h"}</button>`).join("") +
      `<label title="À une heure précise">${I.clock}<input type="time" id="qTime" /></label>`;
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

async function refresh() {
  S = await invoke("get_state");
  $("verTag").textContent = S.about ? S.about.version : "";
  if (!primed) { minutes = S.settings.minutes || 25; primed = true; }
  render();
}

bind();
showTab("focus");
refresh();
T.event.listen("tick", (e) => { const a = S && S.autostart, ab = S && S.about; S = e.payload; S.autostart = a; S.about = ab; if (!primed) refresh(); else render(); });
T.event.listen("quick", () => openQuick("rem"));
