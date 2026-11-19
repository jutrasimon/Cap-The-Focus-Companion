// Cap · extension
// Rapporte à Cap le site de l'onglet actif (pour les jauges) et bloque ce que Cap demande de bloquer :
// tous les sites limités pendant un focus, ou un site dont la jauge est à sec.
// Tout passe par 127.0.0.1 : rien ne sort de l'ordinateur.

const api = globalThis.browser ?? globalThis.chrome;
const actionApi = api.action ?? api.browserAction;
const API = "http://127.0.0.1:47615";
const UA = navigator.userAgent;
const BROWSER = UA.includes("Firefox/") ? "firefox"
  : UA.includes("Edg/") ? "edge"
  : UA.includes("OPR/") ? "opera"
  : UA.includes("Vivaldi") ? "vivaldi"
  : navigator.brave ? "brave"
  : "chrome";
const BLOCKED_PAGE = api.runtime.getURL("blocked.html");
const RULE_ID = 1;

let version = -1;
let looping = false;
let applied = null; // clé des sites actuellement bloqués
let current = { active: false, sites: [] };
let activeHost = ""; // site de l'onglet actif, si le navigateur est au premier plan
let pollCtrl = null;
let restart = false;

const hostOf = (url) => {
  try {
    const u = new URL(url);
    return u.protocol === "http:" || u.protocol === "https:" ? u.hostname.replace(/^www\./, "") : "";
  } catch {
    return "";
  }
};
const matches = (url, sites) => {
  const h = hostOf(url);
  return !!h && sites.some((d) => h === d || h.endsWith("." + d));
};

async function setRules(sites) {
  // Chrome, Edge et Firefox récents gèrent les règles de session. Sinon, la redirection d'onglet suffit.
  try {
    const old = await api.declarativeNetRequest.getSessionRules();
    await api.declarativeNetRequest.updateSessionRules({
      removeRuleIds: old.map((r) => r.id),
      addRules: sites.length
        ? [{ id: RULE_ID, priority: 1, action: { type: "block" }, condition: { requestDomains: sites, resourceTypes: ["main_frame", "sub_frame"] } }]
        : [],
    });
  } catch (e) {
    console.warn("Cap: règles réseau indisponibles", e); // i18n-ok : journal développeur
  }
}

async function redirectTab(tab) {
  if (!tab.id || !tab.url || tab.url.startsWith(BLOCKED_PAGE)) return;
  try { await api.tabs.update(tab.id, { url: `${BLOCKED_PAGE}?u=${encodeURIComponent(tab.url)}` }); } catch {}
}

// Statistiques : une tentative d'ouverture d'un site bloqué
let lastHit = "";
function reportHit(url) {
  const h = hostOf(url);
  const site = current.sites.find((d) => h === d || h.endsWith("." + d));
  if (!site) return;
  const key = `${site}|${Math.floor(Date.now() / 5000)}`;
  if (key === lastHit) return; // une seule par tranche de 5 s
  lastHit = key;
  const reason = (current.reasons || {})[site] || "focus";
  fetch(`${API}/hit?site=${encodeURIComponent(site)}&reason=${reason}`, { headers: { "X-Cap": "1" }, cache: "no-store" }).catch(() => {});
}

async function sweep(sites) {
  const tabs = await api.tabs.query({});
  await Promise.all(tabs.filter((t) => matches(t.url || t.pendingUrl, sites)).map(redirectTab));
}

async function apply(s) {
  const active = !!s.active && s.until > Date.now() && Array.isArray(s.sites) && s.sites.length > 0;
  const sites = active ? s.sites : [];
  current = { ...s, active, sites, at: Date.now(), browser: BROWSER };
  await api.storage.session.set({ cap: current });

  const key = sites.join(",");
  if (key !== applied) {
    await setRules(sites);
    applied = key;
    if (active) await sweep(sites);
  }
  if (active) api.alarms.create("expire", { when: s.until });
  else api.alarms.clear("expire");
  badge(s);
}

function badge(s) {
  try {
    // Pendant un focus : minutes restantes. Sinon : minutes de jauge du site ouvert.
    let text = "", color = "#ff7a50", title = "Cap";
    const tx = (k, v = {}) => { let r = (s.t && s.t[k]) || k; for (const [a, b] of Object.entries(v)) r = r.split(`{${a}}`).join(b); return r; };
    if (s.offline) title = "Cap";
    else if (s.focus) {
      text = s.paused ? "II" : String(Math.max(1, Math.ceil(s.left / 60000)));
      title = tx("ext.badgeFocus");
    } else if (activeHost && s.gauges) {
      const site = Object.keys(s.gauges).find((d) => activeHost === d || activeHost.endsWith("." + d));
      const g = site && s.gauges[site];
      if (g && !g.off && g.cfg && g.cfg.enabled) {
        text = String(Math.floor(g.level / 60));
        color = g.level / g.max < 0.25 ? "#ff5f6d" : "#4cc9a6";
        title = tx("ext.badgeGauge", { s: site, n: Math.floor(g.level / 60) });
      }
    }
    actionApi.setBadgeBackgroundColor({ color });
    actionApi.setBadgeText({ text });
    actionApi.setTitle({ title });
  } catch {}
}

async function fetchState(wait) {
  pollCtrl = new AbortController();
  const ctrl = pollCtrl;
  let timedOut = false;
  const t = setTimeout(() => { timedOut = true; ctrl.abort(); }, wait ? 25000 : 4000);
  try {
    const q = `ext=${BROWSER}&ver=${api.runtime.getManifest().version}&active=${encodeURIComponent(activeHost)}${wait ? `&v=${version}` : ""}`;
    const r = await fetch(`${API}/state?${q}`, { cache: "no-store", signal: ctrl.signal, headers: { "X-Cap": "1" } });
    if (!r.ok) throw new Error(String(r.status));
    const s = await r.json();
    version = s.v;
    return s;
  } catch (e) {
    if (!timedOut && restart) { restart = false; return null; } // relance volontaire
    throw e;
  } finally {
    clearTimeout(t);
  }
}

// Attente longue : Cap répond dès que l'état change. Si le navigateur endort l'extension, l'alarme la réveille.
async function loop() {
  if (looping) return;
  looping = true;
  try {
    let wait = false;
    for (;;) {
      const s = await fetchState(wait);
      if (s) { await apply(s); wait = true; } else wait = false;
    }
  } catch {
    // Cap fermé ou injoignable : rien à faire respecter, on libère.
    await apply({ active: false, offline: true, sites: [] });
  } finally {
    looping = false;
  }
}

// Le site actif a changé : on le dit tout de suite à Cap.
async function refreshActive() {
  let host = "";
  try {
    const win = await api.windows.getLastFocused({});
    if (win && win.focused) {
      const [tab] = await api.tabs.query({ active: true, windowId: win.id });
      if (tab) host = hostOf(tab.url || "");
    }
  } catch {}
  if (host !== activeHost) {
    activeHost = host;
    badge(current);
    if (looping && pollCtrl) { restart = true; pollCtrl.abort(); } else loop();
  }
}

api.alarms.create("poll", { periodInMinutes: 0.5 });
api.alarms.onAlarm.addListener(async (a) => {
  if (a.name === "expire") {
    const { cap } = await api.storage.session.get("cap");
    if (!cap || !cap.until || cap.until <= Date.now()) await apply({ active: false, sites: [] });
  }
  refreshActive();
  loop();
});
api.runtime.onStartup.addListener(loop);
api.runtime.onInstalled.addListener(loop);

api.tabs.onActivated.addListener(refreshActive);
api.windows.onFocusChanged.addListener(refreshActive);
api.tabs.onUpdated.addListener(async (id, info, tab) => {
  const url = info.url || (info.status === "loading" ? tab.url : null);
  if (!url) return;
  if (!current.at) {
    const { cap } = await api.storage.session.get("cap");
    if (cap) current = cap;
  }
  if (current.active && current.until > Date.now() && matches(url, current.sites)) {
    redirectTab({ ...tab, id, url });
    reportHit(url);
  }
  if (tab.active) refreshActive();
  loop();
});

// Messages de la fenêtre de l'extension et de la page de blocage
api.runtime.onMessage.addListener((msg, _sender, reply) => {
  if (msg === "refresh") { loop(); reply(true); return; }
  if (msg && msg.type === "add") {
    fetch(`${API}/add?site=${encodeURIComponent(msg.site)}`, { headers: { "X-Cap": "1" }, cache: "no-store" })
      .then((r) => r.json())
      .then((j) => reply(j))
      .catch(() => reply({ ok: false }));
    return true; // réponse asynchrone
  }
  if (msg && msg.type === "peek") {
    fetch(`${API}/state?ext=${BROWSER}&active=${encodeURIComponent(activeHost)}`, { headers: { "X-Cap": "1" }, cache: "no-store" })
      .then((r) => r.json())
      .then((j) => reply(j))
      .catch(() => reply(null));
    return true;
  }
});

refreshActive();
loop();
