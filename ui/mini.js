const T = window.__TAURI__;
const I = window.ICON;
const $ = (id) => document.getElementById(id);
const invoke = (c, a) => T.core.invoke(c, a);
const fmt = (ms) => { const s = Math.ceil(ms / 1000); const h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60); return h ? `${h}:${String(m).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}` : `${m}:${String(s % 60).padStart(2, "0")}`; };
const mins = (ms) => { const m = Math.max(1, Math.round(ms / 60000)); return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h ${String(m % 60).padStart(2, "0")}`; };

$("aw").innerHTML = I.coffee;
$("go").innerHTML = I.play;
$("ex").innerHTML = I.expand;
$("cl").innerHTML = I.close;

let minutes = 25, L = {};
const t = (k, v = {}) => { let s = L[k] ?? k; for (const [a, b] of Object.entries(v)) s = s.split(`{${a}}`).join(b); return s; };
function render(S) {
  const s = S.session;
  minutes = S.settings.minutes || 25;
  $("m").classList.toggle("idle", !s);
  $("m").classList.toggle("paused", !!s && s.paused);
  $("go").classList.toggle("hidden", !!s);
  if (s) {
    $("g").textContent = s.goal || "";
    $("t").textContent = s.paused ? "II" : fmt(s.left);
    $("bar").style.width = `${100 * (1 - s.left / s.total)}%`;
  } else {
    $("g").textContent = "Cap";
    $("t").textContent = "";
    $("bar").style.width = "0";
    $("go").title = t("mini.focus", { n: minutes });
  }
  $("m").classList.toggle("running", !!s && !s.paused);
  away = S.settings.away_since != null;
  $("aw").classList.toggle("away", away);
  $("aw").title = t(away ? "hdr.back" : "hdr.away");
}
let away = false;

T.event.listen("tick", (e) => render(e.payload));
invoke("get_state").then((S) => {
  L = S.i18n || {};
  $("ex").title = t("mini.open");
  $("cl").title = t("mini.close");
  render(S);
});

$("aw").addEventListener("click", (e) => { e.stopPropagation(); invoke("set_away", { on: !away }); });
$("go").addEventListener("click", (e) => { e.stopPropagation(); invoke("start", { minutes }); });
$("ex").addEventListener("click", (e) => { e.stopPropagation(); invoke("set_mode", { mode: "full" }); });
$("cl").addEventListener("click", (e) => { e.stopPropagation(); invoke("set_mode", { mode: "closed" }); });

// Clic = ouvrir, glisser = déplacer
let down = null;
$("m").addEventListener("mousedown", (e) => { if (e.button === 0 && !e.target.closest("button")) down = { x: e.screenX, y: e.screenY }; });
window.addEventListener("mousemove", (e) => {
  if (down && Math.hypot(e.screenX - down.x, e.screenY - down.y) > 4) { down = null; T.core.invoke("start_drag"); }
});
window.addEventListener("mouseup", () => { if (down) invoke("set_mode", { mode: "full" }); down = null; });
