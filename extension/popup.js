const api = globalThis.browser ?? globalThis.chrome;
const $ = (id) => document.getElementById(id);
const m = (s) => `${Math.floor(s / 60)} min`;

let host = "";
let T = {};
const tx = (k, v = {}) => { let s = T[k] || globalThis.CAP_FR[k] || k; for (const [a, b] of Object.entries(v)) s = s.split(`{${a}}`).join(b); return s; };

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
  if (s && s.t) T = s.t;
  $("hint").textContent = tx("ext.hint");
  const st = $("st");
  st.className = "st";
  if (!s) { st.textContent = tx("ext.capClosed"); }
  else if (s.focus) { st.classList.add("busy"); st.textContent = tx("ext.focusOn"); }
  else { st.classList.add("on"); st.textContent = tx("ext.connected"); }

  if (!host) return $("siteCard").classList.add("hidden");
  $("siteCard").classList.remove("hidden");
  const site = s && Object.keys(s.gauges || {}).find((d) => host === d || host.endsWith("." + d));
  $("host").textContent = site || suggest(host);
  const body = $("siteBody");

  if (!s) {
    body.innerHTML = `<div class="muted"></div>`;
    body.firstChild.textContent = tx("ext.openCap");
  } else if (site) {
    const g = s.gauges[site];
    if (g.off || !g.cfg.enabled) { body.innerHTML = `<div class="muted"></div>`; body.firstChild.textContent = tx(g.off ? "ext.siteOff" : "ext.focusOnly"); }
    else {
      const pct = Math.max(0, Math.min(100, (100 * g.level) / g.max));
      const state = g.locked ? tx("ext.empty1", { n: Math.ceil(g.refill_in / 60) }) : g.using ? tx("ext.using") : tx("ext.refilling");
      body.innerHTML = `<div class="bar"><div class="fill"></div></div><div class="meta"><span></span><span></span></div>`;
      const fill = body.querySelector(".fill");
      fill.style.width = `${pct}%`;
      fill.classList.toggle("low", pct < 25);
      const [a, b] = body.querySelectorAll(".meta span");
      a.textContent = tx("ext.of", { a: m(g.level), b: m(g.max) });
      b.textContent = state;
    }
  } else {
    body.innerHTML = `<button id="add"></button>`;
    $("add").textContent = tx("ext.add");
    $("add").onclick = async () => {
      $("add").disabled = true;
      $("add").textContent = tx("ext.adding");
      const r = await api.runtime.sendMessage({ type: "add", site: suggest(host) });
      if (r && r.ok) render(await api.runtime.sendMessage({ type: "peek" }));
      else { $("add").disabled = false; $("add").textContent = tx("ext.retry"); }
    };
  }
}

(async () => {
  host = await currentHost();
  render(await api.runtime.sendMessage({ type: "peek" }).catch(() => null));
})();
