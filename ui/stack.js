const T = window.__TAURI__;
const I = window.ICON;
const invoke = (c, a) => T.core.invoke(c, a);
let L = {}, PEND = [];
const t = (k, v = {}) => { let s = L[k] ?? k; for (const [a, b] of Object.entries(v)) s = s.split(`{${a}}`).join(b); return s; };
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
const ago = (ms) => {
  const m = Math.floor(ms / 60000);
  if (m < 1) return t("rem.now");
  const d = m < 60 ? t("fmt.min", { n: m }) : t("fmt.hm", { h: Math.floor(m / 60), m: String(m % 60).padStart(2, "0") });
  return t("rem.ago", { d });
};
const MAX = 5;

function render() {
  const list = PEND.slice().sort((a, b) => b.at - a.at);
  document.getElementById("st").innerHTML = list.slice(0, MAX).map((p) => `
    <div class="pr" data-open="1" title="${esc(t("rem.pendingTip"))}">
      <span class="pi">${I[p.icon] || I.bell}</span>
      <span class="pl"><b>${esc(p.label)}</b><span>${ago(Date.now() - p.at)}</span></span>
      <button class="pb" data-drop="${esc(p.rid)}" title="${esc(t("rem.lose"))}">${I.x}</button>
      <button class="pb ok" data-claim="${esc(p.rid)}" title="${esc(t("rem.claim"))}">${I.check}</button>
    </div>`).join("") + (list.length > MAX ? `<div class="more">${esc(t("stack.more", { n: list.length - MAX }))}</div>` : "");
}

document.getElementById("st").addEventListener("click", (e) => {
  const c = e.target.closest("[data-claim]");
  if (c) return invoke("pending_claim", { rid: c.dataset.claim });
  const d = e.target.closest("[data-drop]");
  if (d) return invoke("pending_drop", { rid: d.dataset.drop });
  if (e.target.closest("[data-open]")) invoke("set_mode", { mode: "full" });
});

T.event.listen("stack", (e) => { PEND = e.payload || []; render(); });
Promise.all([invoke("get_state"), invoke("get_pending")]).then(([S, p]) => { L = S.i18n || {}; PEND = p || []; render(); });
setInterval(render, 30000);
