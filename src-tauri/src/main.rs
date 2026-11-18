#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Cap : focus, dossiers, jauges par site, rappels et stats. Tout reste local.
//! L'extension de navigateur parle à Cap sur 127.0.0.1:47615.

mod dossier;
mod gauge;

use gauge::{GaugeCfg, GaugeState};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, PhysicalPosition, State, WebviewWindow, WindowEvent, Wry,
};

const PORT: u16 = 47615;
const LEASE_MS: u64 = 3 * 60_000;
const NUDGE_GAP_MS: u64 = 4 * 60_000;
const SNOOZE_MS: u64 = 10 * 60_000;
const REMINDER_TTL_S: u32 = 120; // sans confirmation après 2 min : rappel raté
const MISSES_TO_PAUSE: u32 = 2;

/// Version disponible sur GitHub, si plus récente
static UPDATE: Mutex<Option<String>> = Mutex::new(None);

/// Positions des fenêtres, à part du verrou principal (les événements de déplacement arrivent n'importe quand).
static POS: Mutex<[Option<[i32; 2]>; 2]> = Mutex::new([None, None]);

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

// ================================================================ données

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Reminder {
    id: String,
    label: String,
    icon: String,
    enabled: bool,
    every_min: u32,
    /// Rappel unique : moment où il sonne, puis il s'efface.
    once_at: Option<u64>,
}

impl Default for Reminder {
    fn default() -> Self {
        Reminder { id: String::new(), label: String::new(), icon: "bell".into(), enabled: true, every_min: 30, once_at: None }
    }
}

fn rem(id: &str, label: &str, every: u32) -> Reminder {
    Reminder { id: id.into(), label: label.into(), icon: id.into(), enabled: true, every_min: every, once_at: None }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(default)]
struct Settings {
    sites: Vec<String>,
    sites_off: Vec<String>,
    gauge_cfg: HashMap<String, GaugeCfg>,
    reminders: Vec<Reminder>,
    reminders_on: bool,
    sound: bool,
    minutes: u32,
    /// Déclin par défaut des dossiers, en % par jour.
    dossier_decay: f64,
    /// "full", "mini" ou "closed"
    mode: String,
    main_pos: Option<[i32; 2]>,
    mini_pos: Option<[i32; 2]>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            sites: ["facebook.com", "instagram.com", "x.com", "reddit.com", "tiktok.com"].iter().map(|s| s.to_string()).collect(),
            sites_off: vec![],
            gauge_cfg: HashMap::new(),
            reminders: vec![
                rem("eyes", "Yeux", 20),
                rem("water", "Eau", 45),
                rem("stretch", "Étirement", 60),
                rem("move", "Bouger", 90),
            ],
            reminders_on: true,
            sound: true,
            minutes: 25,
            dossier_decay: 10.0,
            mode: "full".into(),
            main_pos: None,
            mini_pos: None,
        }
    }
}

impl Settings {
    fn active_sites(&self) -> Vec<String> {
        self.sites.iter().filter(|x| !self.sites_off.contains(x)).cloned().collect()
    }
    fn gauge(&self, site: &str) -> GaugeCfg {
        self.gauge_cfg.get(site).cloned().unwrap_or_default()
    }
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
struct Session {
    goal: String,
    total_ms: u64,
    ends_at: Option<u64>,
    paused_left: Option<u64>,
    started: u64,
}

impl Session {
    fn left(&self, now: u64) -> u64 {
        match (self.paused_left, self.ends_at) {
            (Some(p), _) => p,
            (None, Some(e)) => e.saturating_sub(now),
            _ => 0,
        }
    }
    fn paused(&self) -> bool {
        self.paused_left.is_some()
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
enum DState {
    #[default]
    Open,
    /// Jauge à zéro, en attente d'archivage
    Lost,
    /// Au cimetière
    Dead,
    Done,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
struct Dossier {
    id: u64,
    title: String,
    notes: String,
    color: u8,
    created: u64,
    last_active: u64,
    decay: Option<f64>,
    state: DState,
    ended: Option<u64>,
    activations: u32,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct LegacyGoal {
    text: String,
    done: bool,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Saved {
    settings: Settings,
    session: Option<Session>,
    dossiers: Vec<Dossier>,
    next_id: u64,
    gauges: HashMap<String, GaugeState>,
    gauges_at: u64,
    #[serde(skip_serializing)]
    goals: Vec<LegacyGoal>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Ev {
    t: u64,
    k: String,
    #[serde(default)]
    a: String,
    #[serde(default)]
    n: f64,
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct Stats {
    /// Début d'heure (ms) -> site -> secondes passées
    usage: BTreeMap<u64, BTreeMap<String, f64>>,
    events: Vec<Ev>,
}

#[derive(Serialize, Clone)]
struct Nudge {
    kind: String,
    rid: String,
    title: String,
    body: String,
    icon: String,
    actions: Vec<[String; 2]>,
    countdown: u32,
    chime: bool,
    ttl: u32,
    #[serde(skip)]
    missable: bool,
    #[serde(skip)]
    shown_at: u64,
    #[serde(skip)]
    once: Option<Reminder>,
}

struct Data {
    settings: Settings,
    session: Option<Session>,
    dossiers: Vec<Dossier>,
    next_id: u64,
    gauges: HashMap<String, GaugeState>,
    stats: Stats,
    stats_dirty: bool,
    due: HashMap<String, u64>,
    last_nudge: u64,
    nudge: Option<Nudge>,
    missed_streak: u32,
    ext_seen: HashMap<String, u64>,
    ext_active: HashMap<String, (String, u64)>,
    ext_ver: HashMap<String, String>,
    version: u64,
    path: PathBuf,
    stats_path: PathBuf,
    last_tick: u64,
    last_save: u64,
}

struct Shared {
    d: Mutex<Data>,
    cv: Condvar,
}
type S = Arc<Shared>;

struct TrayItems {
    tray: TrayIcon<Wry>,
    quick: MenuItem<Wry>,
    stop: MenuItem<Wry>,
    last: Mutex<String>,
}

fn write_json(path: &PathBuf, v: &impl Serialize) {
    if let Ok(txt) = serde_json::to_string(v) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, txt).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

impl Data {
    fn save(&mut self) {
        if let Ok(p) = POS.lock() {
            if p[0].is_some() {
                self.settings.main_pos = p[0];
            }
            if p[1].is_some() {
                self.settings.mini_pos = p[1];
            }
        }
        let saved = Saved {
            settings: self.settings.clone(),
            session: self.session.clone(),
            dossiers: self.dossiers.clone(),
            next_id: self.next_id,
            gauges: self.gauges.clone(),
            gauges_at: now_ms(),
            goals: vec![],
        };
        write_json(&self.path, &saved);
        if self.stats_dirty {
            write_json(&self.stats_path, &self.stats);
            self.stats_dirty = false;
        }
        self.last_save = now_ms();
    }

    fn log(&mut self, k: &str, a: &str, n: f64) {
        self.stats.events.push(Ev { t: now_ms(), k: k.into(), a: a.into(), n });
        self.stats_dirty = true;
    }

    fn decay_of(&self, x: &Dossier) -> f64 {
        x.decay.unwrap_or(self.settings.dossier_decay)
    }

    fn ext_connected(&self, now: u64) -> Vec<String> {
        let mut v: Vec<String> =
            self.ext_seen.iter().filter(|(_, t)| now.saturating_sub(**t) < 75_000).map(|(k, _)| k.clone()).collect();
        v.sort();
        v
    }

    fn next_reminder(&self, now: u64) -> Value {
        let once = self
            .settings
            .reminders
            .iter()
            .filter_map(|r| r.once_at.map(|t| (r, t)));
        let recurring = self
            .settings
            .reminders
            .iter()
            .filter(|r| r.once_at.is_none() && r.enabled && self.settings.reminders_on)
            .filter_map(|r| self.due.get(&r.id).map(|t| (r, *t)));
        once.chain(recurring)
            .min_by_key(|(_, t)| *t)
            .map(|(r, t)| json!({ "id": r.id, "label": r.label, "icon": r.icon, "in": t.saturating_sub(now) }))
            .unwrap_or(Value::Null)
    }

    fn focus_on(&self) -> bool {
        self.session.is_some()
    }

    /// Sites bloqués en ce moment, avec la raison : focus ou jauge vide.
    fn blocked(&self) -> Vec<(String, &'static str)> {
        let focus = self.focus_on();
        self.settings
            .active_sites()
            .into_iter()
            .filter_map(|site| {
                if focus {
                    Some((site, "focus"))
                } else if self.settings.gauge(&site).enabled && self.gauges.get(&site).map_or(false, |g| g.locked) {
                    Some((site, "gauge"))
                } else {
                    None
                }
            })
            .collect()
    }

    fn using_hosts(&self, now: u64) -> Vec<String> {
        self.ext_active
            .iter()
            .filter(|(b, (h, at))| {
                !h.is_empty()
                    && now.saturating_sub(*at) < 30_000
                    && self.ext_seen.get(*b).map_or(false, |t| now.saturating_sub(*t) < 30_000)
            })
            .map(|(_, (h, _))| h.clone())
            .collect()
    }

    fn gauges_json(&self, now: u64) -> Value {
        let hosts = self.using_hosts(now);
        let focus = self.focus_on();
        let mut m = serde_json::Map::new();
        for site in self.settings.sites.iter() {
            let cfg = self.settings.gauge(site);
            let st = self.gauges.get(site).cloned().unwrap_or_default();
            let level = if st.level_s.is_finite() { st.level_s.min(cfg.max_s()) } else { cfg.max_s() };
            m.insert(
                site.clone(),
                json!({
                    "level": level.round(),
                    "max": cfg.max_s(),
                    "locked": st.locked,
                    "refill_in": gauge::refill_in_s(&cfg, &st),
                    "using": !focus && !st.locked && hosts.iter().any(|h| gauge::host_matches(h, site)),
                    "cfg": cfg,
                    "off": self.settings.sites_off.contains(site),
                }),
            );
        }
        Value::Object(m)
    }

    fn dossiers_json(&self, now: u64) -> Value {
        Value::Array(
            self.dossiers
                .iter()
                .map(|x| {
                    let decay = self.decay_of(x);
                    let mut v = serde_json::to_value(x).unwrap_or(Value::Null);
                    let lvl = if x.state == DState::Open { dossier::level(x.last_active, now, decay) } else { 0.0 };
                    v["level"] = json!(lvl);
                    v["decay_eff"] = json!(decay);
                    v["ms_left"] = json!(dossier::ms_left(x.last_active, now, decay).min(u64::MAX / 2));
                    v
                })
                .collect(),
        )
    }

    fn snapshot(&self) -> Value {
        let now = now_ms();
        json!({
            "now": now,
            "session": self.session.as_ref().map(|s| json!({
                "goal": s.goal,
                "total": s.total_ms,
                "left": s.left(now),
                "paused": s.paused(),
            })),
            "settings": self.settings,
            "dossiers": self.dossiers_json(now),
            "gauges": self.gauges_json(now),
            "ext": self.ext_connected(now),
            "next": self.next_reminder(now),
            "missed": self.missed_streak,
            "update": UPDATE.lock().ok().and_then(|u| u.clone()),
            // Temps avant chaque rappel récurrent (absent si en pause ou désactivé)
            "due": self.due.iter().map(|(k, t)| (k.clone(), json!(t.saturating_sub(now)))).collect::<serde_json::Map<String, Value>>(),
        })
    }

    fn ext_state(&self) -> Value {
        let now = now_ms();
        let blocked = self.blocked();
        let (left, paused, goal) = match &self.session {
            Some(s) => (s.left(now), s.paused(), s.goal.clone()),
            None => (0, false, String::new()),
        };
        let reasons: serde_json::Map<String, Value> = blocked.iter().map(|(s, r)| (s.clone(), json!(r))).collect();
        json!({
            "v": self.version,
            "active": !blocked.is_empty(),
            "until": now + LEASE_MS,
            "left": left,
            "paused": paused,
            "goal": goal,
            "focus": self.focus_on(),
            "sites": blocked.iter().map(|(s, _)| s.clone()).collect::<Vec<_>>(),
            "reasons": reasons,
            "limited": self.settings.active_sites(),
            "gauges": self.gauges_json(now),
        })
    }

    fn arm_nudge(&mut self, mut n: Nudge) -> Nudge {
        n.shown_at = now_ms();
        self.last_nudge = n.shown_at;
        self.nudge = Some(n.clone());
        n
    }

    fn reminder_missed(&mut self, label: &str) {
        self.missed_streak += 1;
        self.log("rem_missed", label, 0.0);
        if self.missed_streak >= MISSES_TO_PAUSE && self.settings.reminders_on {
            self.settings.reminders_on = false;
            self.due.clear();
            self.log("rem_paused", "", 0.0);
        }
    }
}

// ================================================================ fenêtres

/// Applique le mode : "full" (fenêtre complète), "mini" (bulle) ou "closed".
/// La bulle prend la place du coin haut droit de la fenêtre, et inversement : on ne la perd jamais de vue.
/// À appeler sans tenir le verrou principal.
fn apply_mode(app: &AppHandle, mode: &str, focus: bool) {
    let (Some(main), Some(mini)) = (app.get_webview_window("main"), app.get_webview_window("mini")) else { return };
    let sc = main.scale_factor().unwrap_or(1.0);
    let px = |v: f64| (v * sc).round() as i32;
    let (mw, mh) = main.outer_size().map(|s| (s.width as i32, s.height as i32)).unwrap_or((px(400.0), px(604.0)));
    let (bw, _bh) = mini.outer_size().map(|s| (s.width as i32, s.height as i32)).unwrap_or((px(250.0), px(56.0)));
    // Marges transparentes : carte à 10 px du bord de la fenêtre, bulle à 6 px.
    let (dx, dy) = (px(4.0), px(4.0));
    let saved_main = POS.lock().ok().and_then(|p| p[0]);
    let main_pos = || -> (i32, i32) {
        if main.is_visible().unwrap_or(false) {
            if let Ok(p) = main.outer_position() {
                return (p.x, p.y);
            }
        }
        match saved_main {
            Some([x, y]) => (x, y),
            None => {
                let (ax, ay, aw, ah, s) = work_area(&main);
                (ax + aw - mw - (14.0 * s) as i32, ay + ah - mh - (14.0 * s) as i32)
            }
        }
    };
    match mode {
        "mini" => {
            let (mx, my) = main_pos();
            let _ = mini.set_position(PhysicalPosition::new(mx + mw - bw - dx, my + dy));
            let _ = main.hide();
            let _ = mini.show();
        }
        "closed" => {
            let _ = main.hide();
            let _ = mini.hide();
        }
        _ => {
            let (mut mx, mut my) = if mini.is_visible().unwrap_or(false) {
                match mini.outer_position() {
                    Ok(p) => (p.x + bw - mw + dx, p.y - dy),
                    Err(_) => main_pos(),
                }
            } else {
                main_pos()
            };
            // Toujours entièrement à l'écran
            let (ax, ay, aw, ah, _) = work_area(&main);
            mx = mx.clamp(ax, (ax + aw - mw).max(ax));
            my = my.clamp(ay, (ay + ah - mh).max(ay));
            let _ = main.set_position(PhysicalPosition::new(mx, my));
            let _ = mini.hide();
            let _ = main.show();
            let _ = main.unminimize();
            if focus {
                let _ = main.set_focus();
            }
        }
    }
}

fn set_mode_inner(app: &AppHandle, sh: &Shared, mode: &str) {
    let mode = match mode {
        "mini" | "closed" => mode,
        _ => "full",
    };
    {
        let mut d = sh.d.lock().unwrap();
        d.settings.mode = mode.into();
        d.save();
    }
    apply_mode(app, mode, true);
}

fn current_mode(sh: &Shared) -> String {
    sh.d.lock().unwrap().settings.mode.clone()
}

fn work_area(w: &WebviewWindow) -> (i32, i32, i32, i32, f64) {
    if let Ok(Some(m)) = w.primary_monitor() {
        let wa = m.work_area();
        return (wa.position.x, wa.position.y, wa.size.width as i32, wa.size.height as i32, m.scale_factor());
    }
    (0, 0, 1920, 1040, 1.0)
}

fn place_bottom_right(w: &WebviewWindow) {
    let (ax, ay, aw, ah, sc) = work_area(w);
    if let Ok(sz) = w.outer_size() {
        let m = (14.0 * sc) as i32;
        let _ = w.set_position(PhysicalPosition::new(ax + aw - sz.width as i32 - m, ay + ah - sz.height as i32 - m));
    }
}

fn fmt_left(ms: u64) -> String {
    let s = (ms + 999) / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

// ================================================================ zone de notification

struct TrayInfo {
    tip: String,
    quick: String,
    has_session: bool,
    icon: u8,
}

impl TrayInfo {
    fn of(d: &Data) -> TrayInfo {
        let now = now_ms();
        let (tip, icon) = match &d.session {
            Some(s) => {
                let g: String = s.goal.chars().take(70).collect();
                let state = if s.paused() { " (en pause)" } else { "" };
                (format!("Cap · {}{}{}", fmt_left(s.left(now)), state, if g.is_empty() { String::new() } else { format!("\n{g}") }), if s.paused() { 3 } else { 1 })
            }
            None => ("Cap".into(), 0),
        };
        let quick = match &d.session {
            None => format!("Focus {} min", d.settings.minutes),
            Some(s) if s.paused() => "Reprendre".into(),
            Some(_) => "Pause".into(),
        };
        TrayInfo { tip, quick, has_session: d.session.is_some(), icon }
    }
}

fn apply_tray(app: &AppHandle, i: TrayInfo) {
    let Some(t) = app.try_state::<TrayItems>() else { return };
    let key = format!("{}|{}|{}", i.tip, i.quick, i.icon);
    let mut last = t.last.lock().unwrap();
    if *last == key {
        return;
    }
    let icon_changed = last.rsplit('|').next() != key.rsplit('|').next();
    *last = key;
    drop(last);
    let _ = t.tray.set_tooltip(Some(i.tip));
    let _ = t.quick.set_text(i.quick);
    let _ = t.stop.set_enabled(i.has_session);
    if icon_changed {
        let bytes: &[u8] = match i.icon {
            1 => include_bytes!("../icons/tray-focus.png"),
            3 => include_bytes!("../icons/tray-paused.png"),
            _ => include_bytes!("../icons/tray.png"),
        };
        if let Ok(img) = Image::from_bytes(bytes) {
            let _ = t.tray.set_icon(Some(img));
        }
    }
}

/// Enregistre, réveille l'extension et pousse l'état. Appelé depuis le fil principal.
fn commit(app: &AppHandle, sh: &Shared, d: &mut Data) {
    d.version += 1;
    d.save();
    sh.cv.notify_all();
    let _ = app.emit("tick", d.snapshot());
    apply_tray(app, TrayInfo::of(d));
}

// ================================================================ rappels et focus

fn reminder_nudge(r: &Reminder, once: bool) -> Nudge {
    let (title, body, countdown) = match r.icon.as_str() {
        "eyes" if !once => ("Regarde au loin".to_string(), "6 m · 20 s".to_string(), 20),
        "water" if !once => ("Une gorgée d'eau".into(), String::new(), 0),
        "stretch" if !once => ("Étire-toi".into(), String::new(), 0),
        "move" if !once => ("Lève-toi".into(), String::new(), 0),
        _ => (r.label.clone(), String::new(), 0),
    };
    let mut actions: Vec<[String; 2]> = vec![["later".into(), "+10 min".into()]]; // reporter de 10 min
    if countdown > 0 {
        actions.push(["start".into(), "20 s".into()]);
    }
    actions.push(["done".into(), "Fait".into()]);
    Nudge {
        kind: "reminder".into(),
        rid: r.id.clone(),
        title,
        body,
        icon: r.icon.clone(),
        actions,
        countdown,
        chime: false,
        ttl: REMINDER_TTL_S,
        missable: true,
        shown_at: 0,
        once: if once { Some(r.clone()) } else { None },
    }
}

fn present_nudge(app: &AppHandle, n: &Nudge) {
    if let Some(w) = app.get_webview_window("nudge") {
        place_bottom_right(&w);
        let _ = app.emit_to("nudge", "nudge", n);
        let _ = w.show();
    }
}

fn hide_nudge_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("nudge") {
        let _ = w.hide();
    }
}

fn start_session(d: &mut Data, goal: String, minutes: u32) {
    let minutes = minutes.clamp(1, 600);
    let total = minutes as u64 * 60_000;
    d.settings.minutes = minutes;
    let now = now_ms();
    d.session = Some(Session { goal: goal.trim().chars().take(160).collect(), total_ms: total, ends_at: Some(now + total), paused_left: None, started: now });
    d.log("focus_start", "", minutes as f64);
}

fn new_id(d: &mut Data) -> u64 {
    d.next_id += 1;
    d.next_id
}

// ================================================================ boucle

fn ticker(app: AppHandle, sh: S) {
    loop {
        thread::sleep(Duration::from_millis(1000 - (now_ms() % 1000)));

        // 1. Calcul sous verrou, sans toucher à l'interface.
        let (snap, show, hide, tray, mini_text) = {
            let mut d = sh.d.lock().unwrap();
            let now = now_ms();
            let mut changed = false;
            let mut show: Option<Nudge> = None;
            let mut hide = false;

            // Fin de session
            let finished = match &d.session {
                Some(s) if !s.paused() && s.ends_at.map_or(false, |e| now >= e) => Some(s.clone()),
                _ => None,
            };
            if let Some(s) = finished {
                d.session = None;
                changed = true;
                let mins = (s.total_ms + 30_000) / 60_000;
                d.log("focus_done", &s.goal, mins as f64);
                let n = Nudge {
                    kind: "done".into(),
                    rid: String::new(),
                    title: if s.goal.is_empty() { "Focus".into() } else { s.goal.clone() },
                    body: format!("Session terminée · {mins} min"),
                    icon: "done".into(),
                    actions: vec![["again".into(), format!("Encore {} min", d.settings.minutes)]],
                    countdown: 0,
                    chime: d.settings.sound,
                    ttl: 60,
                    missable: false,
                    shown_at: 0,
                    once: None,
                };
                show = Some(d.arm_nudge(n));
            }

            // Rappel non confirmé à temps : raté
            if let Some(n) = d.nudge.clone() {
                if now.saturating_sub(n.shown_at) > n.ttl as u64 * 1000 {
                    d.nudge = None;
                    hide = true;
                    if n.missable {
                        d.reminder_missed(&n.title);
                    }
                    changed = true;
                }
            }

            // Rappels uniques : sonnent à l'heure dite, puis s'effacent
            if d.nudge.is_none() && show.is_none() {
                if let Some(i) = d.settings.reminders.iter().position(|r| r.once_at.map_or(false, |t| t <= now)) {
                    let r = d.settings.reminders.remove(i);
                    let n = reminder_nudge(&r, true);
                    show = Some(d.arm_nudge(n));
                    changed = true;
                }
            }

            // Rappels récurrents
            if d.settings.reminders_on {
                let list: Vec<Reminder> = d.settings.reminders.iter().filter(|r| r.once_at.is_none() && r.enabled).cloned().collect();
                for r in list.iter() {
                    let every = r.every_min.max(1) as u64 * 60_000;
                    let due = *d.due.entry(r.id.clone()).or_insert(now + every);
                    if now >= due && d.nudge.is_none() && show.is_none() && now.saturating_sub(d.last_nudge) >= NUDGE_GAP_MS {
                        d.due.insert(r.id.clone(), now + every);
                        let n = reminder_nudge(r, false);
                        show = Some(d.arm_nudge(n));
                        break;
                    }
                }
            } else {
                d.due.clear();
            }

            // Jauges des sites
            let dt_ms = now.saturating_sub(d.last_tick);
            d.last_tick = now;
            let asleep = dt_ms > 10_000; // après une veille : recharge seulement
            let dt = (dt_ms as f64 / 1000.0).min(86_400.0);
            let hosts = d.using_hosts(now);
            let focus = d.focus_on();
            let hour = now - now % 3_600_000;
            for site in d.settings.active_sites() {
                let cfg = d.settings.gauge(&site);
                let using = !asleep && !focus && hosts.iter().any(|h| gauge::host_matches(h, &site));
                if using {
                    let locked = d.gauges.get(&site).map_or(false, |g| g.locked);
                    if !locked {
                        *d.stats.usage.entry(hour).or_default().entry(site.clone()).or_insert(0.0) += dt;
                        d.stats_dirty = true;
                    }
                }
                if !cfg.enabled {
                    continue;
                }
                let st = d.gauges.entry(site.clone()).or_default();
                if gauge::step(&cfg, st, dt, using) {
                    let locked = st.locked;
                    changed = true;
                    if locked {
                        d.log("lockout", &site, 0.0);
                    }
                }
            }

            // Dossiers : une jauge à zéro, c'est un dossier perdu
            let decay_default = d.settings.dossier_decay;
            let mut lost = vec![];
            for x in d.dossiers.iter_mut() {
                let decay = x.decay.unwrap_or(decay_default);
                if x.state == DState::Open && dossier::level(x.last_active, now, decay) <= 0.0 {
                    x.state = DState::Lost;
                    x.ended = Some(now);
                    lost.push(x.title.clone());
                }
            }
            for t in lost {
                d.log("dos_lost", &t, 0.0);
                changed = true;
            }

            if changed {
                d.version += 1;
                d.save();
                sh.cv.notify_all();
            } else if now.saturating_sub(d.last_save) > 30_000 {
                d.save();
            }
            let mini_text = d.settings.mode == "mini";
            (d.snapshot(), show, hide, TrayInfo::of(&d), mini_text)
        };

        // 2. Interface, verrou relâché.
        if hide {
            hide_nudge_window(&app);
        }
        if let Some(n) = show {
            present_nudge(&app, &n);
        }
        let _ = app.emit("tick", snap);
        apply_tray(&app, tray);
        let _ = mini_text;
    }
}

// ================================================================ serveur local pour l'extension

fn serve(app: AppHandle, sh: S) {
    let Ok(listener) = TcpListener::bind(("127.0.0.1", PORT)) else {
        eprintln!("Cap: port {PORT} occupé, extension non disponible");
        return;
    };
    for stream in listener.incoming().flatten() {
        let (sh, app) = (sh.clone(), app.clone());
        thread::spawn(move || {
            let _ = handle(stream, app, sh);
        });
    }
}

const BROWSERS: [&str; 6] = ["chrome", "edge", "firefox", "brave", "opera", "vivaldi"];

fn decode(v: &str) -> String {
    let b = v.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => {
                if let Ok(x) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16) {
                    out.push(x);
                    i += 3;
                    continue;
                }
                out.push(b'%');
            }
            b'+' => out.push(b' '),
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn respond(stream: &mut TcpStream, code: &str, body: &str) -> std::io::Result<()> {
    let resp = format!(
        "HTTP/1.1 {code}\r\nContent-Type: application/json; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream.write_all(resp.as_bytes())
}

fn handle(mut stream: TcpStream, app: AppHandle, sh: S) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let (mut origin, mut host, mut marker) = (String::new(), String::new(), false);
    loop {
        let mut h = String::new();
        if reader.read_line(&mut h)? == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            let v = v.trim();
            if k.eq_ignore_ascii_case("origin") {
                origin = v.to_string();
            } else if k.eq_ignore_ascii_case("host") {
                host = v.to_string();
            } else if k.eq_ignore_ascii_case("x-cap") {
                marker = v == "1";
            }
        }
    }
    // Seule l'extension parle à Cap : en-tête X-Cap (impossible pour une page web sans contrôle CORS),
    // pas d'Origin http(s), Host exact (protection contre le DNS rebinding).
    let ok_origin = origin.is_empty()
        || origin.starts_with("chrome-extension://")
        || origin.starts_with("moz-extension://")
        || origin.starts_with("extension://");
    let ok_host = host == format!("127.0.0.1:{PORT}");
    let target = line.split_whitespace().nth(1).unwrap_or("");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if !line.starts_with("GET ") || !marker || !ok_origin || !ok_host {
        return respond(&mut stream, "403 Forbidden", "{}");
    }
    let (mut ext, mut since, mut active, mut site_q, mut reason) = ("chrome".to_string(), None::<u64>, None::<String>, String::new(), String::new());
    let mut ver: Option<String> = None;
    for kv in query.split('&') {
        match kv.split_once('=') {
            Some(("ext", v)) if BROWSERS.contains(&v) => ext = v.into(),
            Some(("v", v)) => since = v.parse().ok(),
            Some(("active", v)) => {
                let h = decode(v).to_lowercase();
                let ok = h.len() < 200 && h.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
                active = Some(if ok { h } else { String::new() });
            }
            Some(("site", v)) => site_q = decode(v),
            Some(("reason", v)) if v == "focus" || v == "gauge" => reason = v.into(),
            Some(("ver", v)) if v.len() < 20 && v.chars().all(|c| c.is_ascii_digit() || c == '.') => ver = Some(v.to_string()),
            _ => {}
        }
    }

    match path {
        "/state" => {
            let body = {
                let mut d = sh.d.lock().unwrap();
                let now = now_ms();
                d.ext_seen.insert(ext.clone(), now);
                if let Some(v) = ver.clone() {
                    d.ext_ver.insert(ext.clone(), v);
                }
                if let Some(a) = active.clone() {
                    d.ext_active.insert(ext.clone(), (a, now));
                }
                if let Some(v) = since {
                    // Attente longue : réponse dès que l'état change, ou après 20 s.
                    let (g, _) = sh.cv.wait_timeout_while(d, Duration::from_secs(20), |d| d.version == v).unwrap();
                    d = g;
                    let now = now_ms();
                    d.ext_seen.insert(ext.clone(), now);
                    if let Some(e) = d.ext_active.get_mut(&ext) {
                        e.1 = now;
                    }
                }
                d.ext_state().to_string()
            };
            respond(&mut stream, "200 OK", &body)
        }
        "/add" => {
            let Some(site) = normalize_site(&site_q) else {
                return respond(&mut stream, "400 Bad Request", r#"{"ok":false}"#);
            };
            let snap = {
                let mut d = sh.d.lock().unwrap();
                if !d.settings.sites.contains(&site) {
                    d.settings.sites.push(site.clone());
                }
                d.settings.sites_off.retain(|x| x != &site);
                d.gauges.entry(site.clone()).or_default();
                d.version += 1;
                d.save();
                sh.cv.notify_all();
                d.snapshot()
            };
            let _ = app.emit("tick", snap);
            respond(&mut stream, "200 OK", &json!({ "ok": true, "site": site }).to_string())
        }
        "/hit" => {
            if let Some(site) = normalize_site(&site_q) {
                let mut d = sh.d.lock().unwrap();
                let r = if reason.is_empty() { "focus".to_string() } else { reason };
                d.log("block_hit", &format!("{site}|{r}"), 0.0);
            }
            respond(&mut stream, "200 OK", r#"{"ok":true}"#)
        }
        _ => respond(&mut stream, "404 Not Found", "{}"),
    }
}

fn normalize_site(raw: &str) -> Option<String> {
    let mut s = raw.trim().to_lowercase();
    for p in ["https://", "http://"] {
        if let Some(rest) = s.strip_prefix(p) {
            s = rest.to_string();
        }
    }
    let s = s.split(['/', '?', '#', ':']).next().unwrap_or("").trim_start_matches("www.").to_string();
    let valid = s.contains('.')
        && s.len() < 120
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        && !s.starts_with('.')
        && !s.ends_with('.');
    valid.then_some(s)
}

// ================================================================ commandes

#[tauri::command]
fn get_state(app: AppHandle, st: State<S>) -> Value {
    let d = st.d.lock().unwrap();
    let mut v = d.snapshot();
    use tauri_plugin_autostart::ManagerExt;
    v["autostart"] = json!(app.autolaunch().is_enabled().unwrap_or(false));
    let now = now_ms();
    v["about"] = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "built": env!("CAP_BUILD_MS").parse::<u64>().unwrap_or(0),
        "ext_version": env!("CAP_EXT_VERSION"),
        "data_dir": d.path.parent().map(|p| p.display().to_string()).unwrap_or_default(),
        "browsers": d.ext_connected(now).iter().map(|b| json!({ "name": b, "version": d.ext_ver.get(b).cloned().unwrap_or_default() })).collect::<Vec<_>>(),
    });
    v
}

// ---------------------------------------------------------------- mises à jour

async fn check_update_inner(app: &AppHandle) -> Option<String> {
    use tauri_plugin_updater::UpdaterExt;
    let found = app.updater().ok()?.check().await.ok()?.map(|u| u.version);
    if let Ok(mut g) = UPDATE.lock() {
        *g = found.clone();
    }
    found
}

#[tauri::command]
async fn check_update(app: AppHandle) -> Option<String> {
    check_update_inner(&app).await
}

/// Télécharge, installe et relance Cap. Les données restent en place.
#[tauri::command]
async fn install_update(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    let Some(update) = update else { return Err("Déjà à jour".into()) };
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| e.to_string())?;
    app.restart();
}

#[tauri::command]
fn open_data_folder(st: State<S>) {
    let dir = st.d.lock().unwrap().path.parent().map(|p| p.to_path_buf());
    if let Some(_d) = dir {
        #[cfg(windows)]
        let _ = std::process::Command::new("explorer").arg(_d).spawn();
    }
}

#[tauri::command]
fn get_stats(st: State<S>) -> Value {
    let d = st.d.lock().unwrap();
    json!({ "now": now_ms(), "usage": d.stats.usage, "events": d.stats.events })
}

#[tauri::command]
fn set_mode(app: AppHandle, st: State<S>, mode: String) {
    set_mode_inner(&app, &st, &mode);
}

#[tauri::command]
fn start(app: AppHandle, st: State<S>, minutes: u32, goal: Option<String>) {
    let mut d = st.d.lock().unwrap();
    start_session(&mut d, goal.unwrap_or_default(), minutes);
    commit(&app, &st, &mut d);
}

#[tauri::command]
fn pause(app: AppHandle, st: State<S>) {
    let mut d = st.d.lock().unwrap();
    let now = now_ms();
    if let Some(s) = d.session.as_mut() {
        if !s.paused() {
            s.paused_left = Some(s.left(now));
            s.ends_at = None;
        }
    }
    commit(&app, &st, &mut d);
}

#[tauri::command]
fn resume(app: AppHandle, st: State<S>) {
    let mut d = st.d.lock().unwrap();
    if let Some(s) = d.session.as_mut() {
        if let Some(p) = s.paused_left.take() {
            s.ends_at = Some(now_ms() + p);
        }
    }
    commit(&app, &st, &mut d);
}

#[tauri::command]
fn stop(app: AppHandle, st: State<S>) {
    let mut d = st.d.lock().unwrap();
    if let Some(s) = d.session.take() {
        let done = (s.total_ms.saturating_sub(s.left(now_ms())) as f64 / 60_000.0).round();
        d.log("focus_stop", &s.goal, done);
    }
    commit(&app, &st, &mut d);
}

#[tauri::command]
fn add_minutes(app: AppHandle, st: State<S>, minutes: i64) {
    let mut d = st.d.lock().unwrap();
    let now = now_ms();
    if let Some(s) = d.session.as_mut() {
        let delta = minutes * 60_000;
        let left = (s.left(now) as i64 + delta).max(60_000) as u64;
        s.total_ms = (s.total_ms as i64 + delta).max(left as i64) as u64;
        if s.paused() {
            s.paused_left = Some(left);
        } else {
            s.ends_at = Some(now + left);
        }
    }
    commit(&app, &st, &mut d);
}

#[tauri::command]
fn set_goal(app: AppHandle, st: State<S>, goal: String) {
    let mut d = st.d.lock().unwrap();
    if let Some(s) = d.session.as_mut() {
        s.goal = goal.trim().chars().take(160).collect();
    }
    commit(&app, &st, &mut d);
}

/// Sites, jauges, son, durée, déclin. Les rappels, le mode et les positions ont leurs propres commandes.
#[tauri::command]
fn save_settings(app: AppHandle, st: State<S>, settings: Settings) {
    let mut d = st.d.lock().unwrap();
    let mut s = settings;
    s.sites = s.sites.iter().filter_map(|x| normalize_site(x)).fold(Vec::new(), |mut acc, x| {
        if !acc.contains(&x) {
            acc.push(x);
        }
        acc
    });
    let kept = s.sites.clone();
    s.sites_off.retain(|x| kept.contains(x));
    s.gauge_cfg = s.gauge_cfg.into_iter().filter(|(k, _)| kept.contains(k)).map(|(k, v)| (k, v.sanitized())).collect();
    s.dossier_decay = if s.dossier_decay.is_finite() { s.dossier_decay.clamp(0.0, 100.0) } else { 10.0 };
    s.reminders = d.settings.reminders.clone();
    s.reminders_on = d.settings.reminders_on;
    s.mode = d.settings.mode.clone();
    s.main_pos = d.settings.main_pos;
    s.mini_pos = d.settings.mini_pos;
    d.settings = s;
    commit(&app, &st, &mut d);
}

// ---------------------------------------------------------------- rappels

#[tauri::command]
fn reminder_add(app: AppHandle, st: State<S>, label: String, every_min: u32, icon: Option<String>) {
    let label: String = label.trim().chars().take(60).collect();
    if label.is_empty() {
        return;
    }
    let mut d = st.d.lock().unwrap();
    let id = format!("r{}", new_id(&mut d));
    d.settings.reminders.push(Reminder { id, label, icon: icon.unwrap_or_else(|| "bell".into()), enabled: true, every_min: every_min.clamp(1, 1440), once_at: None });
    commit(&app, &st, &mut d);
}

#[tauri::command]
fn reminder_once(app: AppHandle, st: State<S>, label: String, at: u64) {
    let label: String = label.trim().chars().take(80).collect();
    let mut d = st.d.lock().unwrap();
    let id = format!("o{}", new_id(&mut d));
    let at = at.max(now_ms() + 5_000);
    d.settings.reminders.push(Reminder { id, label: if label.is_empty() { "Rappel".into() } else { label }, icon: "bell".into(), enabled: true, every_min: 0, once_at: Some(at) });
    commit(&app, &st, &mut d);
}

#[tauri::command]
fn reminder_update(app: AppHandle, st: State<S>, id: String, label: Option<String>, every_min: Option<u32>, enabled: Option<bool>, icon: Option<String>) {
    let mut d = st.d.lock().unwrap();
    if let Some(r) = d.settings.reminders.iter_mut().find(|r| r.id == id) {
        if let Some(l) = label {
            let l: String = l.trim().chars().take(60).collect();
            if !l.is_empty() {
                r.label = l;
            }
        }
        if let Some(m) = every_min {
            r.every_min = m.clamp(1, 1440);
        }
        if let Some(e) = enabled {
            r.enabled = e;
        }
        if let Some(i) = icon {
            r.icon = i;
        }
    }
    d.due.remove(&id);
    commit(&app, &st, &mut d);
}

#[tauri::command]
fn reminder_delete(app: AppHandle, st: State<S>, id: String) {
    let mut d = st.d.lock().unwrap();
    d.settings.reminders.retain(|r| r.id != id);
    d.due.remove(&id);
    commit(&app, &st, &mut d);
}

#[tauri::command]
fn reminders_set_on(app: AppHandle, st: State<S>, on: bool) {
    let mut d = st.d.lock().unwrap();
    d.settings.reminders_on = on;
    d.missed_streak = 0;
    d.due.clear();
    d.log(if on { "rem_on" } else { "rem_off" }, "", 0.0);
    commit(&app, &st, &mut d);
}

#[tauri::command]
fn get_nudge(st: State<S>) -> Option<Nudge> {
    st.d.lock().unwrap().nudge.clone()
}

#[tauri::command]
fn nudge_action(app: AppHandle, st: State<S>, action: String) {
    let mut open = false;
    {
        let mut d = st.d.lock().unwrap();
        let Some(n) = d.nudge.take() else {
            drop(d);
            hide_nudge_window(&app);
            return;
        };
        let now = now_ms();
        match action.as_str() {
            "done" if n.missable => {
                d.missed_streak = 0;
                d.log("rem_ack", &n.title, 0.0);
            }
            "later" if n.missable => {
                d.missed_streak = 0;
                d.log("rem_later", &n.title, 0.0);
                match n.once.clone() {
                    Some(mut r) => {
                        r.once_at = Some(now + SNOOZE_MS);
                        d.settings.reminders.push(r);
                    }
                    None => {
                        d.due.insert(n.rid.clone(), now + SNOOZE_MS);
                    }
                }
            }
            "again" => {
                let goal = n.title.clone();
                let m = d.settings.minutes;
                start_session(&mut d, if goal == "Focus" { String::new() } else { goal }, m);
            }
            "open" => {
                // Ouvrir Cap n'est pas confirmer : le rappel reste à l'écran.
                if n.missable {
                    d.nudge = Some(n);
                    drop(d);
                    set_mode_inner(&app, &st, "full");
                    return;
                }
                open = true;
            }
            // Fermer sans confirmer un rappel : il compte comme raté.
            "dismiss" if n.missable => d.reminder_missed(&n.title),
            _ => {}
        }
        commit(&app, &st, &mut d);
    }
    hide_nudge_window(&app);
    if open {
        set_mode_inner(&app, &st, "full");
    }
}

// ---------------------------------------------------------------- dossiers

#[tauri::command]
fn dossier_add(app: AppHandle, st: State<S>, title: String) -> Option<u64> {
    let title: String = title.trim().chars().take(120).collect();
    if title.is_empty() {
        return None;
    }
    let mut d = st.d.lock().unwrap();
    let id = new_id(&mut d);
    let now = now_ms();
    let color = (d.dossiers.len() % 6) as u8;
    d.dossiers.insert(0, Dossier { id, title: title.clone(), notes: String::new(), color, created: now, last_active: now, decay: None, state: DState::Open, ended: None, activations: 1 });
    d.log("dos_new", &title, 0.0);
    commit(&app, &st, &mut d);
    Some(id)
}

#[tauri::command]
fn dossier_update(app: AppHandle, st: State<S>, id: u64, title: Option<String>, notes: Option<String>, decay: Option<f64>, color: Option<u8>, decay_default: Option<bool>) {
    let mut d = st.d.lock().unwrap();
    if let Some(x) = d.dossiers.iter_mut().find(|x| x.id == id) {
        if let Some(t) = title {
            let t: String = t.trim().chars().take(120).collect();
            if !t.is_empty() {
                x.title = t;
            }
        }
        if let Some(n) = notes {
            x.notes = n.chars().take(20_000).collect();
        }
        if let Some(v) = decay {
            if v.is_finite() {
                x.decay = Some(v.clamp(0.0, 100.0));
            }
        }
        if decay_default == Some(true) {
            x.decay = None;
        }
        if let Some(c) = color {
            x.color = c % 6;
        }
    }
    commit(&app, &st, &mut d);
}

#[tauri::command]
fn dossier_activate(app: AppHandle, st: State<S>, id: u64) {
    let mut d = st.d.lock().unwrap();
    let mut t = None;
    if let Some(x) = d.dossiers.iter_mut().find(|x| x.id == id && x.state == DState::Open) {
        x.last_active = now_ms();
        x.activations += 1;
        t = Some(x.title.clone());
    }
    if let Some(t) = t {
        d.log("dos_active", &t, 0.0);
    }
    commit(&app, &st, &mut d);
}

#[tauri::command]
fn dossier_set_state(app: AppHandle, st: State<S>, id: u64, state: String) {
    let mut d = st.d.lock().unwrap();
    let now = now_ms();
    let mut ev = None;
    if let Some(x) = d.dossiers.iter_mut().find(|x| x.id == id) {
        match (state.as_str(), x.state) {
            ("done", DState::Open) => {
                x.state = DState::Done;
                x.ended = Some(now);
                ev = Some(("dos_done", x.title.clone()));
            }
            ("dead", DState::Lost) => {
                x.state = DState::Dead;
                ev = Some(("dos_dead", x.title.clone()));
            }
            ("open", DState::Done) => {
                x.state = DState::Open;
                x.ended = None;
                x.last_active = now;
            }
            _ => {}
        }
    }
    if let Some((k, t)) = ev {
        d.log(k, &t, 0.0);
    }
    commit(&app, &st, &mut d);
}

#[tauri::command]
fn dossier_delete(app: AppHandle, st: State<S>, id: u64) {
    let mut d = st.d.lock().unwrap();
    d.dossiers.retain(|x| x.id != id);
    commit(&app, &st, &mut d);
}

#[tauri::command]
fn dossier_move(app: AppHandle, st: State<S>, id: u64, before: Option<u64>) {
    let mut d = st.d.lock().unwrap();
    if let Some(i) = d.dossiers.iter().position(|x| x.id == id) {
        let x = d.dossiers.remove(i);
        let pos = before.and_then(|b| d.dossiers.iter().position(|y| y.id == b)).unwrap_or(d.dossiers.len());
        d.dossiers.insert(pos, x);
    }
    commit(&app, &st, &mut d);
}

// ---------------------------------------------------------------- divers

#[tauri::command]
fn set_autostart(app: AppHandle, on: bool) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    let al = app.autolaunch();
    let _ = if on { al.enable() } else { al.disable() };
    al.is_enabled().unwrap_or(false)
}

#[tauri::command]
fn open_extension_folder(app: AppHandle) -> bool {
    let candidates = [
        app.path().resource_dir().ok(),
        std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf())),
    ];
    let Some(dir) = candidates.into_iter().flatten().find(|d| d.join("extension").exists()) else { return false };
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(dir.join("extension")).spawn();
    true
}

#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

async fn tokio_sleep(secs: u64) {
    // Pas de dépendance à tokio : un fil dédié qui réveille la tâche
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let _ = tauri::async_runtime::spawn_blocking(move || {
        thread::sleep(Duration::from_secs(secs));
        let _ = tx.send(());
    })
    .await;
    let _ = rx.try_recv();
}

// ================================================================ démarrage

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let sh = app.state::<S>();
            set_mode_inner(app, &sh, "full");
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--hidden"])))
        .invoke_handler(tauri::generate_handler![
            get_state,
            get_stats,
            set_mode,
            start,
            pause,
            resume,
            stop,
            add_minutes,
            set_goal,
            save_settings,
            reminder_add,
            reminder_once,
            reminder_update,
            reminder_delete,
            reminders_set_on,
            get_nudge,
            nudge_action,
            dossier_add,
            dossier_update,
            dossier_activate,
            dossier_set_state,
            dossier_delete,
            dossier_move,
            set_autostart,
            open_extension_folder,
            open_data_folder,
            check_update,
            install_update,
            quit
        ])
        .setup(move |app| {
            let dir = app.path().app_config_dir().unwrap_or_else(|_| PathBuf::from("."));
            let path = dir.join("cap.json");
            let stats_path = dir.join("stats.json");
            let first_run = !path.exists();
            let saved: Saved = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
            let stats: Stats = std::fs::read_to_string(&stats_path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
            let mut settings = saved.settings;
            if settings.mode.is_empty() {
                settings.mode = "full".into();
            }
            if let Ok(mut p) = POS.lock() {
                *p = [settings.main_pos, settings.mini_pos];
            }
            let now = now_ms();
            let mut next_id = saved.next_id;
            let mut dossiers = saved.dossiers;
            // Anciens objectifs : ils deviennent des dossiers
            if dossiers.is_empty() {
                for (i, g) in saved.goals.iter().filter(|g| !g.done && !g.text.trim().is_empty()).enumerate() {
                    next_id += 1;
                    dossiers.push(Dossier { id: next_id, title: g.text.clone(), color: (i % 6) as u8, created: now, last_active: now, activations: 1, ..Default::default() });
                }
            }
            let sh: S = Arc::new(Shared {
                d: Mutex::new(Data {
                    settings,
                    session: saved.session,
                    dossiers,
                    next_id,
                    gauges: saved.gauges,
                    stats,
                    stats_dirty: false,
                    due: HashMap::new(),
                    last_nudge: 0,
                    nudge: None,
                    missed_streak: 0,
                    ext_seen: HashMap::new(),
                    ext_active: HashMap::new(),
                    ext_ver: HashMap::new(),
                    version: 1,
                    path,
                    stats_path,
                    // L'écart depuis la dernière sauvegarde (ordinateur éteint) recharge les jauges.
                    last_tick: if saved.gauges_at > 0 { saved.gauges_at } else { now },
                    last_save: now,
                }),
                cv: Condvar::new(),
            });
            app.manage(sh.clone());

            // Raccourcis : Ctrl+Alt+F ouvre ou réduit Cap, Ctrl+Alt+R ajoute un rappel
            {
                use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
                app.handle().plugin(
                    tauri_plugin_global_shortcut::Builder::new()
                        .with_handler(|app, sc, ev| {
                            if ev.state != ShortcutState::Pressed {
                                return;
                            }
                            let sh = app.state::<S>();
                            if sc.key == tauri_plugin_global_shortcut::Code::KeyR {
                                set_mode_inner(app, &sh, "full");
                                let _ = app.emit_to("main", "quick", ());
                            } else {
                                let next = if current_mode(&sh) == "full" { "mini" } else { "full" };
                                set_mode_inner(app, &sh, next);
                            }
                        })
                        .build(),
                )?;
                let _ = app.global_shortcut().register("ctrl+alt+f");
                let _ = app.global_shortcut().register("ctrl+alt+r");
            }

            // Zone de notification
            let open = MenuItem::with_id(app, "open", "Ouvrir", true, Some("Ctrl+Alt+F"))?;
            let mini = MenuItem::with_id(app, "mini", "Réduire", true, None::<&str>)?;
            let quickrem = MenuItem::with_id(app, "quickrem", "Rappel rapide", true, Some("Ctrl+Alt+R"))?;
            let quick = MenuItem::with_id(app, "quick", "Focus", true, None::<&str>)?;
            let stop_i = MenuItem::with_id(app, "stop", "Terminer le focus", false, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
            let menu = Menu::with_items(
                app,
                &[&open, &mini, &PredefinedMenuItem::separator(app)?, &quickrem, &quick, &stop_i, &PredefinedMenuItem::separator(app)?, &quit_i],
            )?;
            let tray = TrayIconBuilder::with_id("cap")
                .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
                .tooltip("Cap")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, ev| {
                    let sh = app.state::<S>();
                    match ev.id().as_ref() {
                        "open" => set_mode_inner(app, &sh, "full"),
                        "mini" => set_mode_inner(app, &sh, "mini"),
                        "quickrem" => {
                            set_mode_inner(app, &sh, "full");
                            let _ = app.emit_to("main", "quick", ());
                        }
                        "quick" => {
                            let mut d = sh.d.lock().unwrap();
                            let now = now_ms();
                            match d.session.as_ref().map(|s| s.paused()) {
                                None => {
                                    let m = d.settings.minutes;
                                    start_session(&mut d, String::new(), m);
                                }
                                Some(true) => {
                                    if let Some(s) = d.session.as_mut() {
                                        if let Some(p) = s.paused_left.take() {
                                            s.ends_at = Some(now + p);
                                        }
                                    }
                                }
                                Some(false) => {
                                    if let Some(s) = d.session.as_mut() {
                                        s.paused_left = Some(s.left(now));
                                        s.ends_at = None;
                                    }
                                }
                            }
                            commit(app, &sh, &mut d);
                        }
                        "stop" => {
                            let mut d = sh.d.lock().unwrap();
                            if let Some(s) = d.session.take() {
                                let done = (s.total_ms.saturating_sub(s.left(now_ms())) as f64 / 60_000.0).round();
                                d.log("focus_stop", &s.goal, done);
                            }
                            commit(app, &sh, &mut d);
                        }
                        "quit" => app.exit(0),
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, ev| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                        let app = tray.app_handle();
                        let sh = app.state::<S>();
                        set_mode_inner(app, &sh, "full");
                    }
                })
                .build(app)?;
            app.manage(TrayItems { tray, quick, stop: stop_i, last: Mutex::new(String::new()) });

            // Retenir où l'utilisateur place chaque fenêtre
            for (label, idx) in [("main", 0usize), ("mini", 1usize)] {
                if let Some(w) = app.get_webview_window(label) {
                    let w2 = w.clone();
                    w.on_window_event(move |e| match e {
                        WindowEvent::Moved(p) => {
                            if let Ok(mut pos) = POS.try_lock() {
                                pos[idx] = Some([p.x, p.y]);
                            }
                        }
                        // Alt+F4 : on cache au lieu de détruire la fenêtre
                        WindowEvent::CloseRequested { api, .. } => {
                            api.prevent_close();
                            let _ = w2.hide();
                        }
                        _ => {}
                    });
                }
            }

            let mode = {
                let d = sh.d.lock().unwrap();
                apply_tray(app.handle(), TrayInfo::of(&d));
                if first_run { "full".to_string() } else { d.settings.mode.clone() }
            };
            apply_mode(app.handle(), &mode, true);

            let (h1, s1) = (app.handle().clone(), sh.clone());
            thread::spawn(move || ticker(h1, s1));
            let (h2, s2) = (app.handle().clone(), sh.clone());
            thread::spawn(move || serve(h2, s2));

            // Vérifie les mises à jour 20 s après le démarrage, puis toutes les 6 h
            let h3 = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                tokio_sleep(20).await;
                loop {
                    check_update_inner(&h3).await;
                    tokio_sleep(6 * 3600).await;
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Cap n'a pas pu démarrer");
}
