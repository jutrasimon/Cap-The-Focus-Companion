const api = globalThis.browser ?? globalThis.chrome;
const $ = (id) => document.getElementById(id);
const target = new URLSearchParams(location.search).get("u") || "";
let host = "";
try { host = new URL(target).hostname.replace(/^www\./, ""); } catch {}

let state = null;
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
    $("kicker").textContent = state.paused ? "Session en pause" : "Session de focus en cours";
    $("h1").textContent = "Pas maintenant.";
    $("site").textContent = `${host} attendra un peu.`;
    $("goalBox").classList.toggle("hidden", !state.goal);
    $("goal").textContent = state.goal;
    $("time").textContent = state.paused ? "en pause" : fmt(state.left - elapsed);
    $("gaugeBox").classList.add("hidden");
    $("go").classList.add("hidden");
    $("back").textContent = "Retour au travail";
    $("foot").textContent = "Le blocage se lève tout seul à la fin de la session. Pour libérer les sites avant, utilise Cap.";
    document.title = "Pas maintenant · Cap";
  } else if (reason === "gauge") {
    const g = (state.gauges || {})[site] || {};
    const refill = Math.max(0, (g.refill_in || 0) * 1000 - elapsed);
    $("kicker").textContent = "Jauge à sec";
    $("h1").textContent = "Assez pour l'instant.";
    $("site").textContent = `Ta jauge ${site} est vide.`;
    $("goalBox").classList.add("hidden");
    $("gaugeBox").classList.remove("hidden");
    const level = Math.min(g.max || 1, (g.level || 0) + (elapsed / 1000) * ((g.cfg && g.cfg.refill_per_hour) || 15) / 60);
    $("gaugeFill").style.width = `${Math.max(1, (100 * level) / (g.max || 1))}%`;
    $("time").textContent = refill > 0 ? fmt(refill) : "";
    $("timeLabel").textContent = refill > 0 ? "avant 1 minute de recharge" : "";
    $("go").classList.add("hidden");
    $("back").textContent = "Fermer l'onglet";
    $("foot").textContent = "La jauge se recharge quand tu n'es pas sur le site. Ses réglages sont dans Cap.";
    document.title = "Jauge vide · Cap";
  } else {
    $("kicker").textContent = "Accès rétabli";
    $("h1").textContent = "C'est libéré.";
    $("site").textContent = "";
    $("goalBox").classList.add("hidden");
    $("gaugeBox").classList.add("hidden");
    $("time").textContent = "";
    $("timeLabel").textContent = "";
    $("back").textContent = "Fermer l'onglet";
    $("foot").textContent = "";
    if (target) { $("go").href = target; $("go").textContent = host ? `Continuer vers ${host}` : "Continuer"; $("go").classList.remove("hidden"); }
    document.title = "Libéré · Cap";
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
