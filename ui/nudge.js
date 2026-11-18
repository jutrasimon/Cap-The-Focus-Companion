const T = window.__TAURI__;
const invoke = (c, a) => T.core.invoke(c, a);
const $ = (id) => document.getElementById(id);
const I = window.ICON;

let cur = null, ttlLeft = 0, cdLeft = 0, timer = null, hovering = false, closing = false;

function chime() {
  try {
    const ctx = new AudioContext();
    [[659.25, 0], [987.77, 0.14]].forEach(([f, t]) => {
      const o = ctx.createOscillator(), g = ctx.createGain();
      o.type = "sine"; o.frequency.value = f;
      g.gain.setValueAtTime(0, ctx.currentTime + t);
      g.gain.linearRampToValueAtTime(0.16, ctx.currentTime + t + 0.02);
      g.gain.exponentialRampToValueAtTime(0.0001, ctx.currentTime + t + 1.2);
      o.connect(g).connect(ctx.destination); o.start(ctx.currentTime + t); o.stop(ctx.currentTime + t + 1.3);
    });
    setTimeout(() => ctx.close(), 2000);
  } catch (_) {}
}

function confetti() {
  const colors = ["#ff9a4d", "#ff5f6d", "#ffd166", "#5ad7b4", "#7c8cff"];
  for (let i = 0; i < 22; i++) {
    const s = document.createElement("span");
    s.className = "confetti";
    const a = Math.random() * Math.PI * 2, d = 40 + Math.random() * 90;
    s.style.background = colors[i % colors.length];
    s.style.setProperty("--x", `${Math.cos(a) * d + 40}px`);
    s.style.setProperty("--y", `${Math.sin(a) * d * 0.6}px`);
    s.style.setProperty("--r", `${Math.random() * 540}deg`);
    s.style.animationDelay = `${Math.random() * 0.12}s`;
    $("n").appendChild(s);
    setTimeout(() => s.remove(), 1400);
  }
}

function badge() {
  if (cur.countdown > 0 && cdLeft > 0) {
    const r = 21, c = 2 * Math.PI * r, f = cdLeft / cur.countdown;
    $("badge").innerHTML = `<svg class="ring" viewBox="0 0 46 46"><circle cx="23" cy="23" r="${r}" stroke="rgba(255,255,255,.08)"/><circle cx="23" cy="23" r="${r}" stroke="currentColor" stroke-linecap="round" stroke-dasharray="${c}" stroke-dashoffset="${c * (1 - f)}" style="transition:stroke-dashoffset 1s linear"/></svg><span class="cd">${cdLeft}</span>`;
  } else {
    $("badge").innerHTML = I[cur.icon] || I.bell;
  }
}

function present(n) {
  if (!n) return;
  cur = n; closing = false;
  document.body.classList.toggle("ok", n.kind === "done" || n.kind === "break_done" || n.icon === "eyes");
  document.body.classList.toggle("done", n.kind === "done");
  $("title").textContent = n.title;
  $("body").textContent = n.body;
  const tips = { later: "Me le rappeler dans 10 min", done: "C'est fait", start: "Compte 20 secondes, puis c'est fait", again: "Relancer un focus" };
  $("acts").innerHTML = n.actions.map(([id, label], i) => `<button data-a="${id}" title="${tips[id] || ""}" class="${i === n.actions.length - 1 ? "main" : ""}">${label}</button>`).join("");
  ttlLeft = n.ttl; cdLeft = 0;
  badge();
  const box = $("n");
  box.classList.remove("in"); void box.offsetWidth; box.classList.add("in");
  $("ttl").style.transition = "none"; $("ttl").style.transform = "scaleX(1)";
  if (n.chime) chime();
  if (n.kind === "done") setTimeout(confetti, 250);
  clearInterval(timer);
  timer = setInterval(tick, 1000);
}

// Le délai est géré par Cap : sans confirmation à temps, le rappel compte comme raté.
function tick() {
  if (!cur || closing) return;
  if (cdLeft > 0) {
    cdLeft--; badge();
    if (cdLeft === 0) return setTimeout(() => close("done"), 600);
  }
  ttlLeft--;
  $("ttl").style.transition = "transform 1s linear";
  $("ttl").style.transform = `scaleX(${Math.max(0, ttlLeft / cur.ttl)})`;
}

function close(action) {
  if (closing) return;
  closing = true; clearInterval(timer);
  $("n").classList.remove("in");
  setTimeout(() => invoke("nudge_action", { action }), 320);
}

$("x").innerHTML = I.close;
$("x").onclick = (e) => { e.stopPropagation(); close("dismiss"); };
$("acts").onclick = (e) => {
  const b = e.target.closest("button"); if (!b) return;
  e.stopPropagation();
  if (b.dataset.a === "start") { cdLeft = cur.countdown; badge(); b.remove(); return; } // 20 s puis confirmé
  close(b.dataset.a);
};
// Un clic ailleurs ouvre Cap. Pour un rappel, il reste affiché jusqu'à confirmation.
$("n").addEventListener("click", (e) => {
  if (e.target.closest("button")) return;
  if (cur && cur.kind === "reminder") invoke("nudge_action", { action: "open" });
  else close("open");
});

T.event.listen("nudge", (e) => present(e.payload));
invoke("get_nudge").then(present);
