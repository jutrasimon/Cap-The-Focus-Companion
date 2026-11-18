const api = globalThis.browser ?? globalThis.chrome;
const $ = (id) => document.getElementById(id);
const m = (s) => `${Math.floor(s / 60)} min`;

let host = "";

async function currentHost() {
  const [tab] = await api.tabs.query({ active: true, currentWindow: true });
  try {
    const u = new URL(tab.url);
    if (u.protocol === "http:" || u.protocol === "https:") return u.hostname.replace(/^www\./, "");
    if (u.href.startsWith(api.runtime.getURL("blocked.html"))) {
      return new URL(new URLSearchParams(u.search).get("u")).hostname.replace(/^www\./, "");
    }
  } catch {}
  return "";
}

// Domaine proposé : on retire les sous-domaines courants (m., web., mobile.)
const suggest = (h) => h.replace(/^(m|mobile|web|app)\./, "");

function render(s) {
  const st = $("st");
  st.className = "st";
  if (!s) { st.textContent = "Cap fermé"; }
  else if (s.focus) { st.classList.add("busy"); st.textContent = "Focus en cours"; }
  else { st.classList.add("on"); st.textContent = "Connectée"; }

  if (!host) return $("siteCard").classList.add("hidden");
  $("siteCard").classList.remove("hidden");
  const site = s && Object.keys(s.gauges || {}).find((d) => host === d || host.endsWith("." + d));
  $("host").textContent = site || suggest(host);
  const body = $("siteBody");

  if (!s) {
    body.innerHTML = `<div class="muted">Ouvre Cap pour gérer ce site.</div>`;
  } else if (site) {
    const g = s.gauges[site];
    if (g.off) body.innerHTML = `<div class="muted">Dans ta liste, mais désactivé dans Cap.</div>`;
    else if (!g.cfg.enabled) body.innerHTML = `<div class="muted">Limité pendant le focus seulement (pas de jauge).</div>`;
    else {
      const pct = Math.max(0, Math.min(100, (100 * g.level) / g.max));
      const state = g.locked ? `vide · 1 min dans ${Math.ceil(g.refill_in / 60)} min` : g.using ? "en cours d'usage" : "se recharge";
      body.innerHTML = `<div class="bar"><div class="fill"></div></div><div class="meta"><span></span><span></span></div>`;
      const fill = body.querySelector(".fill");
      fill.style.width = `${pct}%`;
      fill.classList.toggle("low", pct < 25);
      const [a, b] = body.querySelectorAll(".meta span");
      a.textContent = `${m(g.level)} sur ${m(g.max)}`;
      b.textContent = state;
    }
  } else {
    body.innerHTML = `<button id="add">Ajouter à mes sites limités</button>`;
    $("add").onclick = async () => {
      $("add").disabled = true;
      $("add").textContent = "Ajout…";
      const r = await api.runtime.sendMessage({ type: "add", site: suggest(host) });
      if (r && r.ok) render(await api.runtime.sendMessage({ type: "peek" }));
      else { $("add").disabled = false; $("add").textContent = "Cap ne répond pas. Réessayer"; }
    };
  }
}

(async () => {
  host = await currentHost();
  render(await api.runtime.sendMessage({ type: "peek" }).catch(() => null));
})();
