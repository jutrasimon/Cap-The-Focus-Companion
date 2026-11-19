const api = globalThis.browser ?? globalThis.chrome;
const $ = (id) => document.getElementById(id);
const target = new URLSearchParams(location.search).get("u") || "";
let host = "";
try { host = new URL(target).hostname.replace(/^www\./, ""); } catch {}

let state = null;
const tx = (k, v = {}) => { let s = (state && state.t && state.t[k]) || globalThis.CAP_FR[k] || k; for (const [a, b] of Object.entries(v)) s = s.split(`{${a}}`).join(b); return s; };
const fmt = (ms) => { const s = Math.max(0, Math.ceil(ms / 1000)); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`; };
const siteOf = () => state && (state.sites || []).find((d) => host === d || host.endsWith("." + d));

function render() {
  const site = siteOf();
  const live = state && state.active && state.until > Date.now() && site;
  const reason = live ? (state.reasons || {})[site] || "focus" : null;
  document.body.classList.toggle("free", !live);
  document.body.classList.toggle("is-gauge", reason === "gauge");
  const elapsed = state ? Date.now() - state.at : 0;

  if (reason === "focus") {
    $("kicker").textContent = state.paused ? tx("ext.focusPaused") : tx("ext.focusNow");
    $("h1").textContent = tx("ext.notNow");
    $("site").textContent = tx("ext.wait", { h: host });
    $("goalBox").classList.toggle("hidden", !state.goal);
    $("goal").textContent = state.goal;
    $("goalLbl").textContent = tx("ext.goal");
    $("time").textContent = state.paused ? tx("ext.paused") : fmt(state.left - elapsed);
    $("gaugeBox").classList.add("hidden");
    $("go").classList.add("hidden");
    $("back").textContent = tx("ext.backToWork");
    $("foot").textContent = tx("ext.focusFoot");
    document.title = tx("ext.titleBlocked");
  } else if (reason === "gauge") {
    const g = (state.gauges || {})[site] || {};
    const refill = Math.max(0, (g.refill_in || 0) * 1000 - elapsed);
    $("kicker").textContent = tx("ext.gaugeEmpty");
    $("h1").textContent = tx("ext.enough");
    $("site").textContent = tx("ext.gaugeOf", { s: site });
    $("goalBox").classList.add("hidden");
    $("gaugeBox").classList.remove("hidden");
    const level = Math.min(g.max || 1, (g.level || 0) + (elapsed / 1000) * ((g.cfg && g.cfg.refill_per_hour) || 15) / 60);
    $("gaugeFill").style.width = `${Math.max(1, (100 * level) / (g.max || 1))}%`;
    $("time").textContent = refill > 0 ? fmt(refill) : "";
    $("timeLabel").textContent = refill > 0 ? tx("ext.refillLabel") : "";
    $("go").classList.add("hidden");
    $("back").textContent = tx("ext.closeTab");
    $("foot").textContent = tx("ext.gaugeFoot");
    document.title = tx("ext.titleGauge");
  } else {
    $("kicker").textContent = tx("ext.access");
    $("h1").textContent = tx("ext.freed");
    $("site").textContent = "";
    $("goalBox").classList.add("hidden");
    $("gaugeBox").classList.add("hidden");
    $("time").textContent = "";
    $("timeLabel").textContent = "";
    $("back").textContent = tx("ext.closeTab");
    $("foot").textContent = "";
    if (target) { $("go").href = target; $("go").textContent = host ? tx("ext.continueTo", { h: host }) : tx("ext.continue"); $("go").classList.remove("hidden"); }
    document.title = tx("ext.titleFree");
  }
}

async function load() {
  const { cap } = await api.storage.session.get("cap");
  state = cap || null;
  render();
}

$("back").onclick = async () => {
  const t = await api.tabs.getCurrent();
  if (!t) return;
  const siblings = await api.tabs.query({ windowId: t.windowId });
  if (siblings.length > 1) api.tabs.remove(t.id);
  else api.tabs.update(t.id, { url: "about:blank" });
};

api.storage.session.onChanged.addListener((c) => { if (c.cap) { state = c.cap.newValue; render(); } });
setInterval(render, 1000);
api.runtime.sendMessage("refresh").catch(() => {});
load();
