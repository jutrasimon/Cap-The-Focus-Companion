#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Cap : focus, dossiers, jauges par site, rappels et stats. Tout reste local.
//! L'extension de navigateur parle à Cap sur 127.0.0.1:47615.

mod dossier;
mod gauge;
mod perf;

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
const REMINDER_TTL_S: u32 = 120; // sans confirmation après 2 min : mis de côté dans la pile

// ================================================================ langues

type Dict = HashMap<String, String>;

struct Locales {
    /// Code -> textes (intégrés, puis fichiers du dossier des langues par-dessus)
    all: std::collections::BTreeMap<String, Dict>,
    /// Langue effective
    lang: String,
}

static LOC: Mutex<Option<Locales>> = Mutex::new(None);
const BUILTIN: [(&str, &str); 2] = [("fr", include_str!("../../locales/fr.json")), ("en", include_str!("../../locales/en.json"))];

fn parse_dict(txt: &str) -> Option<Dict> {
    let v: serde_json::Map<String, Value> = serde_json::from_str(txt).ok()?;
    Some(v.into_iter().filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string()))).collect())
}

/// Charge les langues intégrées et celles du dossier `locales` (créé avec des modèles au besoin).
fn load_locales(dir: &std::path::Path, lang: &str) {
    let _ = std::fs::create_dir_all(dir);
    let mut all = std::collections::BTreeMap::new();
    for (code, txt) in BUILTIN {
        let f = dir.join(format!("{code}.json"));
        if !f.exists() {
            let _ = std::fs::write(&f, txt);
        }
        if let Some(d) = parse_dict(txt) {
            all.insert(code.to_string(), d);
        }
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().map_or(true, |x| x != "json") {
                continue;
            }
            if let Some(d) = std::fs::read_to_string(&p).ok().and_then(|t| parse_dict(&t)) {
                if let Some(code) = d.get("_meta.code").cloned() {
                    all.entry(code).or_insert_with(Dict::new).extend(d);
                }
            }
        }
    }
    *LOC.lock().unwrap() = Some(Locales { all, lang: lang.to_string() });
}

/// Textes de la langue effective, complétés par l'anglais puis le français.
fn dict() -> Dict {
    let g = LOC.lock().unwrap();
    let Some(l) = g.as_ref() else { return Dict::new() };
    let mut d = l.all.get("fr").cloned().unwrap_or_default();
    if let Some(en) = l.all.get("en") {
        d.extend(en.clone());
    }
    if let Some(cur) = l.all.get(&l.lang) {
        d.extend(cur.clone());
    }
    d
}

fn tr(key: &str) -> String {
    let g = LOC.lock().unwrap();
    if let Some(l) = g.as_ref() {
        for code in [l.lang.as_str(), "en", "fr"] {
            if let Some(v) = l.all.get(code).and_then(|d| d.get(key)) {
                return v.clone();
            }
        }
    }
    key.to_string()
}

fn trn(key: &str, n: impl std::fmt::Display) -> String {
    tr(key).replace("{n}", &n.to_string())
}

fn langs() -> Value {
    let g = LOC.lock().unwrap();
    let Some(l) = g.as_ref() else { return json!([]) };
    Value::Array(
        l.all
            .iter()
            .map(|(c, d)| json!({ "code": c, "name": d.get("_meta.name").cloned().unwrap_or_else(|| c.clone()) }))
            .collect(),
    )
}

fn set_effective_lang(code: &str) {
    if let Some(l) = LOC.lock().unwrap().as_mut() {
        l.lang = if l.all.contains_key(code) { code.to_string() } else { "en".to_string() };
    }
}

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
    /// Rappels éteints par la pause automatique, rallumés quand on réactive
    paused_ids: Vec<String>,
    sound: bool,
    minutes: u32,
    /// Déclin par défaut des dossiers, en % par jour.
    dossier_decay: f64,
    /// "full", "mini" ou "closed"
    mode: String,
    main_pos: Option<[i32; 2]>,
    mini_pos: Option<[i32; 2]>,
    /// Raccourcis globaux : ouvrir/réduire et rappel unique
    sc_open: String,
    sc_quick: String,
    /// "auto" ou un code de langue
    lang: String,
    /// Rappels ratés de suite avant la mise en pause
    misses_to_pause: u32,
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
            paused_ids: vec![],
            sound: true,
            minutes: 25,
            dossier_decay: 10.0,
            mode: "full".into(),
            main_pos: None,
            mini_pos: None,
            sc_open: "ctrl+alt+f".into(),
            sc_quick: "ctrl+alt+r".into(),
            lang: "auto".into(),
            misses_to_pause: 2,
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

/// Rappel affiché mais pas encore confirmé. Il reste réclamable jusqu'au prochain rappel du même type.
#[derive(Serialize, Deserialize, Clone)]
struct Pending {
    rid: String,
    label: String,
    icon: String,
    at: u64,
    #[serde(skip)]
    once: Option<Reminder>,
}

struct Data {
    pending: Vec<Pending>,
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
    labels: Vec<(MenuItem<Wry>, &'static str)>,
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
            "pending": self.pending,
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
            "t": dict().into_iter().filter(|(k, _)| k.starts_with("ext.")).map(|(k, v)| (k, Value::String(v))).collect::<serde_json::Map<String, Value>>(),
        })
    }

    /// Met un rappel de côté : il attend d'être confirmé.
    fn park(&mut self, n: &Nudge) {
        self.pending.retain(|p| p.rid != n.rid);
        self.pending.push(Pending { rid: n.rid.clone(), label: n.title.clone(), icon: n.icon.clone(), at: n.shown_at, once: n.once.clone() });
    }

    /// Le rappel suivant du même type arrive : celui en attente est perdu.
    fn lose_pending(&mut self, rid: &str) {
        if let Some(i) = self.pending.iter().position(|p| p.rid == rid) {
            let p = self.pending.remove(i);
            self.reminder_missed(&p.label);
        }
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
        if self.missed_streak >= self.settings.misses_to_pause.max(1) && self.settings.reminders_on {
            self.settings.reminders_on = false;
            // Chaque rappel s'éteint aussi, comme si on l'avait fait à la main
            let mut ids = vec![];
            for r in self.settings.reminders.iter_mut().filter(|r| r.once_at.is_none() && r.enabled) {
                r.enabled = false;
                ids.push(r.id.clone());
            }
            self.settings.paused_ids = ids;
            self.due.clear();
            self.log("rem_paused", "", 0.0);
        }
    }
}

// ================================================================ fenêtres

// Fenêtres créées à la demande et détruites quand elles disparaissent :
// une fenêtre cachée garderait son moteur d'affichage en mémoire.
const MAIN_SIZE: (f64, f64) = (400.0, 604.0);
const MINI_SIZE: (f64, f64) = (250.0, 56.0);
const NUDGE_SIZE: (f64, f64) = (360.0, 150.0);
const STACK_W: f64 = 300.0;
const STACK_ROW: f64 = 46.0;
const STACK_MAX: usize = 5;
const WEBVIEW_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,Translate,OptimizationHints,MediaRouter,AutofillServerCommunication,SpareRendererForSitePerProcess --disable-background-networking --disable-component-update --disable-sync --disable-default-apps --disable-breakpad --no-first-run --no-pings --js-flags=--lite-mode";

/// Demande d'ouverture du rappel unique, pour une fenêtre pas encore chargée
static PENDING_QUICK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn monitor_area(app: &AppHandle) -> (i32, i32, i32, i32, f64) {
    if let Ok(Some(m)) = app.primary_monitor() {
        let wa = m.work_area();
        return (wa.position.x, wa.position.y, wa.size.width as i32, wa.size.height as i32, m.scale_factor());
    }
    (0, 0, 1920, 1040, 1.0)
}

/// Crée la fenêtre si elle n'existe pas. À appeler depuis le fil principal (via run_on_main_thread).
fn ensure_window(app: &AppHandle, label: &str) -> Option<WebviewWindow> {
    if let Some(w) = app.get_webview_window(label) {
        return Some(w);
    }
    let (url, (w, h), focus) = match label {
        "main" => ("index.html", MAIN_SIZE, true),
        "mini" => ("mini.html", MINI_SIZE, false),
        "stack" => ("stack.html", (STACK_W, 120.0), false),
        _ => ("nudge.html", NUDGE_SIZE, false),
    };
    let win = tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App(url.into()))
        .title("Cap")
        .inner_size(w, h)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(focus)
        // Moteur d'affichage au régime : services d'Edge inutiles coupés.
        // Identique pour toutes les fenêtres (exigence de WebView2).
        .additional_browser_args(WEBVIEW_ARGS)
        .build()
        .ok()?;
    if label == "main" {
        let app2 = app.clone();
        win.on_window_event(move |e| match e {
            WindowEvent::Moved(p) => {
                if let Ok(mut pos) = POS.try_lock() {
                    pos[0] = Some([p.x, p.y]);
                }
            }
            // Alt+F4 : comme le bouton fermer
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let sh = app2.state::<S>();
                set_mode_inner(&app2, &sh, "closed");
            }
            _ => {}
        });
    } else {
        win.on_window_event(|e| {
            if let WindowEvent::CloseRequested { api, .. } = e {
                api.prevent_close();
            }
        });
    }
    Some(win)
}

/// Cache tout de suite, détruit 3 s plus tard si la fenêtre est toujours cachée.
/// Détruire une fenêtre pendant qu'elle traite encore un appel la fait planter.
fn retire_window(app: &AppHandle, label: &str) {
    let Some(w) = app.get_webview_window(label) else { return };
    let _ = w.hide();
    let (a, l) = (app.clone(), label.to_string());
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(3));
        let a2 = a.clone();
        let _ = a.run_on_main_thread(move || {
            if let Some(w) = a2.get_webview_window(&l) {
                if !w.is_visible().unwrap_or(true) {
                    let _ = w.destroy();
                }
            }
        });
    });
}

/// Cache une fenêtre gardée en vie, puis la met en veille 2 s plus tard si elle est toujours cachée :
/// son moteur libère alors l'essentiel de sa mémoire.
fn hide_window(app: &AppHandle, label: &str) {
    let Some(w) = app.get_webview_window(label) else { return };
    let _ = w.hide();
    let (a, l) = (app.clone(), label.to_string());
    thread::spawn(move || {
        thread::sleep(Duration::from_secs(2));
        let a2 = a.clone();
        let _ = a.run_on_main_thread(move || {
            if let Some(w) = a2.get_webview_window(&l) {
                if !w.is_visible().unwrap_or(true) {
                    sleep_webview(&w, true);
                }
            }
        });
    });
}

/// Réveille avant d'afficher.
fn show_window(w: &WebviewWindow) {
    sleep_webview(w, false);
    let _ = w.show();
}

#[cfg(windows)]
fn sleep_webview(w: &WebviewWindow, sleep: bool) {
    let _ = w.with_webview(move |wv| unsafe {
        use webview2_com::Microsoft::Web::WebView2::Win32::*;
        use windows_core::Interface;
        let ctl = wv.controller();
        let Ok(core) = ctl.CoreWebView2() else { return };
        if let Ok(c19) = core.cast::<ICoreWebView2_19>() {
            let _ = c19.SetMemoryUsageTargetLevel(if sleep {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
            } else {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
            });
        }
        if let Ok(c3) = core.cast::<ICoreWebView2_3>() {
            if sleep {
                let _ = ctl.SetIsVisible(false);
                let h = webview2_com::TrySuspendCompletedHandler::create(Box::new(|_, _| Ok(())));
                let _ = c3.TrySuspend(&h);
            } else {
                let _ = c3.Resume();
                let _ = ctl.SetIsVisible(true);
            }
        }
    });
}

#[cfg(not(windows))]
fn sleep_webview(_: &WebviewWindow, _: bool) {}

/// Applique le mode : "full" (fenêtre complète), "mini" (bulle) ou "closed" (rien à l'écran).
/// La bulle prend la place du coin haut droit de la fenêtre, et inversement.
fn apply_mode(app: &AppHandle, mode: &str, focus: bool) {
    let (a, m) = (app.clone(), mode.to_string());
    let _ = app.run_on_main_thread(move || apply_mode_now(&a, &m, focus));
}

fn apply_mode_now(app: &AppHandle, mode: &str, focus: bool) {
    let (ax, ay, aw, ah, sc) = monitor_area(app);
    let px = |v: f64| (v * sc).round() as i32;
    let (mw, mh, bw) = (px(MAIN_SIZE.0), px(MAIN_SIZE.1), px(MINI_SIZE.0));
    // Marges transparentes : carte à 10 px du bord de la fenêtre, bulle à 6 px.
    let d4 = px(4.0);
    let visible_pos = |label: &str| {
        app.get_webview_window(label)
            .filter(|w| w.is_visible().unwrap_or(false))
            .and_then(|w| w.outer_position().ok())
            .map(|p| (p.x, p.y))
    };
    let main_pos = visible_pos("main")
        .or_else(|| POS.lock().ok().and_then(|p| p[0]).map(|[x, y]| (x, y)))
        .unwrap_or((ax + aw - mw - px(14.0), ay + ah - mh - px(14.0)));
    SHOWN.store(match mode { "mini" => 2, "closed" => 0, _ => 1 }, std::sync::atomic::Ordering::Relaxed);
    match mode {
        "mini" => {
            let (mx, my) = main_pos;
            if let Some(b) = ensure_window(app, "mini") {
                let _ = b.set_position(PhysicalPosition::new(mx + mw - bw - d4, my + d4));
                show_window(&b);
            }
            hide_window(app, "main");
        }
        "closed" => {
            hide_window(app, "main");
            hide_window(app, "mini");
        }
        _ => {
            let (mut mx, mut my) = match visible_pos("mini") {
                Some((x, y)) => (x + bw - mw + d4, y - d4),
                None => main_pos,
            };
            mx = mx.clamp(ax, (ax + aw - mw).max(ax));
            my = my.clamp(ay, (ay + ah - mh).max(ay));
            if let Some(w) = ensure_window(app, "main") {
                let _ = w.set_position(PhysicalPosition::new(mx, my));
                show_window(&w);
                if focus {
                    let _ = w.set_focus();
                }
            }
            hide_window(app, "mini");
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

/// Ouvre Cap sur le rappel unique. Si la fenêtre n'existe pas encore, elle le fera en chargeant.
fn open_quick(app: &AppHandle, sh: &Shared) {
    if app.get_webview_window("main").is_some() {
        set_mode_inner(app, sh, "full");
        let _ = app.emit("quick", ());
    } else {
        PENDING_QUICK.store(true, std::sync::atomic::Ordering::Relaxed);
        set_mode_inner(app, sh, "full");
    }
}

fn current_mode(sh: &Shared) -> String {
    sh.d.lock().unwrap().settings.mode.clone()
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
                let state = if s.paused() { tr("tray.paused") } else { String::new() };
                (format!("Cap · {}{}{}", fmt_left(s.left(now)), state, if g.is_empty() { String::new() } else { format!("\n{g}") }), if s.paused() { 3 } else { 1 })
            }
            None => ("Cap".into(), 0),
        };
        let quick = match &d.session {
            None => trn("tray.focus", d.settings.minutes),
            Some(s) if s.paused() => tr("tray.resume"),
            Some(_) => tr("tray.pause"),
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

/// Fenêtre affichée : 0 aucune, 1 principale, 2 bulle. Seule celle-là reçoit l'état,
/// les fenêtres en veille n'accumulent rien.
static SHOWN: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn emit_tick<S: serde::Serialize + Clone>(app: &AppHandle, snap: S) {
    match SHOWN.load(std::sync::atomic::Ordering::Relaxed) {
        1 => { let _ = app.emit_to("main", "tick", snap); }
        2 => { let _ = app.emit_to("mini", "tick", snap); }
        _ => {}
    }
}

/// Enregistre, réveille l'extension et pousse l'état. Appelé depuis le fil principal.
fn commit(app: &AppHandle, sh: &Shared, d: &mut Data) {
    d.version += 1;
    d.save();
    sh.cv.notify_all();
    let _ = emit_tick(&app, d.snapshot());
    apply_tray(app, TrayInfo::of(d));
}

// ================================================================ rappels et focus

fn reminder_nudge(r: &Reminder, once: bool) -> Nudge {
    // Rappels par défaut : texte prêt à l'emploi ; sinon, le nom choisi
    let builtin = !once && ["eyes", "water", "stretch", "move"].contains(&r.id.as_str()) && is_default_label(&r.id, &r.label);
    let (title, body, countdown) = match r.icon.as_str() {
        "eyes" if builtin => (tr("nudge.eyes"), tr("nudge.eyesBody"), 20),
        "water" if builtin => (tr("nudge.water"), String::new(), 0),
        "stretch" if builtin => (tr("nudge.stretch"), String::new(), 0),
        "move" if builtin => (tr("nudge.move"), String::new(), 0),
        _ => (r.label.clone(), String::new(), 0),
    };
    let mut actions: Vec<[String; 2]> = vec![["later".into(), tr("nudge.later")]]; // reporter de 10 min
    if countdown > 0 {
        actions.push(["start".into(), tr("nudge.20s")]);
    }
    actions.push(["done".into(), tr("nudge.done")]);
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

/// Le nom du rappel est-il encore le nom par défaut (dans n'importe quelle langue) ?
fn is_default_label(id: &str, label: &str) -> bool {
    let key = format!("rem.{id}");
    let g = LOC.lock().unwrap();
    g.as_ref().map_or(false, |l| l.all.values().any(|d| d.get(&key).map_or(false, |v| v == label)))
}

/// Affiche la notification. Une fenêtre neuve lit elle-même la notification en cours (get_nudge).
fn present_nudge(app: &AppHandle, n: &Nudge) {
    let (a, n) = (app.clone(), n.clone());
    let _ = app.run_on_main_thread(move || {
        let existed = a.get_webview_window("nudge").is_some();
        if let Some(w) = ensure_window(&a, "nudge") {
            let (ax, ay, aw, ah, sc) = monitor_area(&a);
            let px = |v: f64| (v * sc).round() as i32;
            let _ = w.set_position(PhysicalPosition::new(ax + aw - px(NUDGE_SIZE.0) - px(14.0), ay + ah - px(NUDGE_SIZE.1) - px(14.0)));
            if existed {
                let _ = a.emit_to("nudge", "nudge", &n);
            }
            let _ = w.show();
        }
    });
}

/// Pile des rappels en attente, au-dessus de l'emplacement des notifications.
fn present_stack(app: &AppHandle, pending: Vec<Pending>) {
    let a = app.clone();
    let _ = app.run_on_main_thread(move || {
        if pending.is_empty() {
            retire_window(&a, "stack");
            return;
        }
        let existed = a.get_webview_window("stack").is_some();
        if let Some(w) = ensure_window(&a, "stack") {
            let rows = pending.len().min(STACK_MAX) as f64 + if pending.len() > STACK_MAX { 0.6 } else { 0.0 };
            let h = 16.0 + rows * STACK_ROW;
            let _ = w.set_size(tauri::LogicalSize::new(STACK_W, h));
            let (ax, ay, aw, ah, sc) = monitor_area(&a);
            let px = |v: f64| (v * sc).round() as i32;
            let _ = w.set_position(PhysicalPosition::new(ax + aw - px(STACK_W) - px(14.0), ay + ah - px(NUDGE_SIZE.1) - px(h) - px(10.0)));
            if existed {
                let _ = a.emit_to("stack", "stack", &pending);
            }
            let _ = w.show();
        }
    });
}

fn hide_nudge_window(app: &AppHandle) {
    let a = app.clone();
    let _ = app.run_on_main_thread(move || retire_window(&a, "nudge"));
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
                    title: if s.goal.is_empty() { tr("nudge.focus") } else { s.goal.clone() },
                    body: trn("nudge.ended", mins),
                    icon: "done".into(),
                    actions: vec![["again".into(), trn("nudge.again", d.settings.minutes)]],
                    countdown: 0,
                    chime: d.settings.sound,
                    ttl: 60,
                    missable: false,
                    shown_at: 0,
                    once: None,
                };
                show = Some(d.arm_nudge(n));
            }

            let pending_before = d.pending.len();
            // Rappel non confirmé à temps : il passe dans la pile, encore réclamable
            if let Some(n) = d.nudge.clone() {
                if now.saturating_sub(n.shown_at) > n.ttl as u64 * 1000 {
                    d.nudge = None;
                    hide = true;
                    if n.missable {
                        d.park(&n);
                    }
                    changed = true;
                }
            }
            // Un rappel unique en attente depuis 12 h est perdu
            let stale: Vec<String> = d.pending.iter().filter(|p| p.once.is_some() && now.saturating_sub(p.at) > 12 * 3_600_000).map(|p| p.rid.clone()).collect();
            for rid in stale {
                d.lose_pending(&rid);
                changed = true;
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
                    // Le rappel revient : l'occurrence précédente non confirmée est perdue
                    if now >= due && d.pending.iter().any(|p| p.rid == r.id) {
                        d.lose_pending(&r.id);
                        changed = true;
                    }
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
            let stack = if changed || d.pending.len() != pending_before { Some(d.pending.clone()) } else { None };
            // Personne n'affiche l'état : on évite de le construire
            let shown = d.settings.mode != "closed";
            (if shown { d.snapshot() } else { Value::Null }, show, hide, TrayInfo::of(&d), (mini_text, stack))
        };

        // 2. Interface, verrou relâché.
        if hide {
            hide_nudge_window(&app);
        }
        if let Some(p) = mini_text.1.clone() {
            present_stack(&app, p);
        }
        if let Some(n) = show {
            present_nudge(&app, &n);
        }
        if !snap.is_null() {
            let _ = emit_tick(&app, snap);
        }
        apply_tray(&app, tray);

    }
}

// ================================================================ serveur local pour l'extension

fn serve(app: AppHandle, sh: S) {
    let Ok(listener) = TcpListener::bind(("127.0.0.1", PORT)) else {
        eprintln!("Cap: port {PORT} occupé, extension non disponible"); // i18n-ok : journal développeur
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
            let _ = emit_tick(&app, snap);
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
    v["i18n"] = json!(dict());
    v["quick"] = json!(PENDING_QUICK.swap(false, std::sync::atomic::Ordering::Relaxed));
    v["langs"] = langs();
    let now = now_ms();
    v["about"] = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "built": env!("CAP_BUILD_MS").parse::<u64>().unwrap_or(0),
        "ext_version": env!("CAP_EXT_VERSION"),
        "sc_ok": SC_OK.lock().map(|g| *g).unwrap_or([false, false]),
        "data_dir": d.path.parent().map(|p| p.display().to_string()).unwrap_or_default(),
        "browsers": d.ext_connected(now).iter().map(|b| json!({ "name": b, "version": d.ext_ver.get(b).cloned().unwrap_or_default() })).collect::<Vec<_>>(),
    });
    v
}

// ---------------------------------------------------------------- raccourcis

/// État de chaque raccourci : vrai s'il est bien enregistré auprès de Windows
static SC_OK: Mutex<[bool; 2]> = Mutex::new([false, false]);

fn register_sc(app: &AppHandle, kind: &str, accel: &str) -> bool {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let ok = app.global_shortcut().register(accel).is_ok();
    if let Ok(mut g) = SC_OK.lock() {
        g[if kind == "quick" { 1 } else { 0 }] = ok;
    }
    ok
}

#[tauri::command]
fn set_shortcut(app: AppHandle, st: State<S>, kind: String, accel: String) -> Result<(), String> {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};
    let accel = accel.to_lowercase();
    accel.parse::<Shortcut>().map_err(|_| tr("set.scInvalid"))?;
    let old = {
        let d = st.d.lock().unwrap();
        if kind == "quick" { d.settings.sc_quick.clone() } else { d.settings.sc_open.clone() }
    };
    let _ = app.global_shortcut().unregister(old.as_str());
    if !register_sc(&app, &kind, &accel) {
        register_sc(&app, &kind, &old);
        return Err(tr("set.scBusy"));
    }
    let mut d = st.d.lock().unwrap();
    if kind == "quick" {
        d.settings.sc_quick = accel;
    } else {
        d.settings.sc_open = accel;
    }
    commit(&app, &st, &mut d);
    Ok(())
}

// ---------------------------------------------------------------- langues (commandes)

/// Applique la langue choisie (ou celle du système quand "auto") partout : menus, rappels par défaut, extension.
fn apply_lang(app: &AppHandle, d: &mut Data, system: &str) {
    let want = if d.settings.lang == "auto" { system.split('-').next().unwrap_or("en").to_lowercase() } else { d.settings.lang.clone() };
    set_effective_lang(&want);
    // Rappels par défaut pas encore renommés : on suit la langue
    for r in d.settings.reminders.iter_mut() {
        if ["eyes", "water", "stretch", "move"].contains(&r.id.as_str()) && is_default_label(&r.id, &r.label) {
            r.label = tr(&format!("rem.{}", r.id));
        }
    }
    if let Some(t) = app.try_state::<TrayItems>() {
        for (item, key) in t.labels.iter() {
            let _ = item.set_text(tr(key));
        }
        *t.last.lock().unwrap() = String::new();
    }
}

static SYS_LANG: Mutex<String> = Mutex::new(String::new());

/// L'interface donne la langue du système (navigator.language) au démarrage.
#[tauri::command]
fn set_system_lang(app: AppHandle, st: State<S>, lang: String) {
    *SYS_LANG.lock().unwrap() = lang.clone();
    let mut d = st.d.lock().unwrap();
    apply_lang(&app, &mut d, &lang);
    commit(&app, &st, &mut d);
}

#[tauri::command]
fn set_lang(app: AppHandle, st: State<S>, lang: String) -> Value {
    let sys = SYS_LANG.lock().unwrap().clone();
    {
        let mut d = st.d.lock().unwrap();
        d.settings.lang = lang;
        apply_lang(&app, &mut d, &sys);
        commit(&app, &st, &mut d);
    }
    json!({ "i18n": dict(), "langs": langs() })
}

fn locales_dir(d: &Data) -> PathBuf {
    d.path.parent().map(|p| p.join("locales")).unwrap_or_else(|| PathBuf::from("locales"))
}

/// Importe un fichier de langue (contenu JSON). Il doit avoir `_meta.code` et `_meta.name`.
#[tauri::command]
fn import_locale(app: AppHandle, st: State<S>, content: String) -> Result<String, String> {
    let dict = parse_dict(&content).ok_or_else(|| tr("set.langBad"))?;
    let code = dict.get("_meta.code").cloned().unwrap_or_default().to_lowercase();
    let name = dict.get("_meta.name").cloned().unwrap_or_default();
    let ok = !code.is_empty() && code.len() <= 12 && code.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !name.is_empty();
    if !ok {
        return Err(tr("set.langBad"));
    }
    let mut d = st.d.lock().unwrap();
    let dir = locales_dir(&d);
    let _ = std::fs::create_dir_all(&dir);
    std::fs::write(dir.join(format!("{code}.json")), &content).map_err(|e| e.to_string())?;
    load_locales(&dir, &code);
    d.settings.lang = code;
    let sys = SYS_LANG.lock().unwrap().clone();
    apply_lang(&app, &mut d, &sys);
    commit(&app, &st, &mut d);
    Ok(name)
}

#[tauri::command]
fn open_locales_folder(st: State<S>) {
    let _dir = locales_dir(&st.d.lock().unwrap());
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(_dir).spawn();
}

// ---------------------------------------------------------------- performance

static STARTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static PROBE: Mutex<Option<perf::Probe>> = Mutex::new(None);

/// Ce que Cap coûte à la machine : Cap et ses processus WebView2. Mesuré seulement quand l'onglet est ouvert.
#[tauri::command]
fn get_perf(app: AppHandle, st: State<S>) -> Value {
    let p = PROBE.lock().unwrap().get_or_insert_with(perf::Probe::new).sample();
    let d = st.d.lock().unwrap();
    let size = |p: &PathBuf| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    let windows: Vec<&str> = ["main", "mini", "nudge"].into_iter().filter(|l| app.get_webview_window(l).is_some()).collect();
    json!({
        "cpu": p.cpu,
        "cap_cpu": p.cap_cpu,
        "mem": p.private,
        "cap_mem": p.cap_private,
        "working": p.working,
        "procs": p.procs,
        "windows": windows,
        "cores": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        "uptime": now_ms().saturating_sub(STARTED.load(std::sync::atomic::Ordering::Relaxed)),
        "data": size(&d.path) + size(&d.stats_path),
        "events": d.stats.events.len(),
    })
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
    let Some(update) = update else { return Err(tr("err.upToDate")) };
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
    s.paused_ids = d.settings.paused_ids.clone();
    s.mode = d.settings.mode.clone();
    s.main_pos = d.settings.main_pos;
    s.sc_open = d.settings.sc_open.clone();
    s.sc_quick = d.settings.sc_quick.clone();
    s.lang = d.settings.lang.clone();
    s.misses_to_pause = s.misses_to_pause.clamp(1, 20);
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
    d.pending.retain(|p| p.rid != id);
    let p = d.pending.clone();
    commit(&app, &st, &mut d);
    drop(d);
    present_stack(&app, p);
}

/// Réclamer un rappel en attente : il compte comme confirmé.
#[tauri::command]
fn pending_claim(app: AppHandle, st: State<S>, rid: String) {
    let mut d = st.d.lock().unwrap();
    if let Some(i) = d.pending.iter().position(|p| p.rid == rid) {
        let p = d.pending.remove(i);
        d.missed_streak = 0;
        d.log("rem_ack", &p.label, 0.0);
    }
    let p = d.pending.clone();
    commit(&app, &st, &mut d);
    drop(d);
    present_stack(&app, p);
}

/// Laisser tomber un rappel en attente : il compte comme raté.
#[tauri::command]
fn pending_drop(app: AppHandle, st: State<S>, rid: String) {
    let mut d = st.d.lock().unwrap();
    d.lose_pending(&rid);
    let p = d.pending.clone();
    commit(&app, &st, &mut d);
    drop(d);
    present_stack(&app, p);
}

#[tauri::command]
fn get_pending(st: State<S>) -> Vec<Pending> {
    st.d.lock().unwrap().pending.clone()
}

#[tauri::command]
fn reminders_set_on(app: AppHandle, st: State<S>, on: bool) {
    let mut d = st.d.lock().unwrap();
    d.settings.reminders_on = on;
    d.missed_streak = 0;
    if on {
        let ids = std::mem::take(&mut d.settings.paused_ids);
        for r in d.settings.reminders.iter_mut().filter(|r| ids.contains(&r.id)) {
            r.enabled = true;
        }
    }
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
    let stack: Option<Vec<Pending>>;
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
                // Nouveau focus, texte vierge : le texte précédent reste dans les stats
                let m = d.settings.minutes;
                start_session(&mut d, String::new(), m);
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
            // Fermer sans confirmer : mis de côté, encore réclamable
            "dismiss" if n.missable => d.park(&n),
            _ => {}
        }
        commit(&app, &st, &mut d);
        stack = Some(d.pending.clone());
    }
    hide_nudge_window(&app);
    if let Some(p) = stack {
        present_stack(&app, p);
    }
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
    // Journal de plantage dans le dossier de données, pour comprendre ce qui s'est passé
    std::panic::set_hook(Box::new(|info| {
        let dir = std::env::var("APPDATA").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(".")).join("app.cap.focus");
        let _ = std::fs::create_dir_all(&dir);
        let line = format!("{} · {} · {}\n", now_ms(), env!("CARGO_PKG_VERSION"), info); // i18n-ok : journal développeur
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("crash.log")) {
            let _ = f.write_all(line.as_bytes());
        }
    }));
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
            set_shortcut,
            get_perf,
            pending_claim,
            pending_drop,
            get_pending,
            set_system_lang,
            set_lang,
            import_locale,
            open_locales_folder,
            quit
        ])
        .setup(move |app| {
            STARTED.store(now_ms(), std::sync::atomic::Ordering::Relaxed);
            let dir = app.path().app_config_dir().unwrap_or_else(|_| PathBuf::from("."));
            let path = dir.join("cap.json");
            let stats_path = dir.join("stats.json");
            let first_run = !path.exists();
            let saved: Saved = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
            let stats: Stats = std::fs::read_to_string(&stats_path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
            let mut settings = saved.settings;
            load_locales(&dir.join("locales"), if settings.lang == "auto" { "fr" } else { &settings.lang });
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
                    pending: vec![],
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

            // Raccourcis globaux (personnalisables dans les réglages)
            {
                use tauri_plugin_global_shortcut::ShortcutState;
                app.handle().plugin(
                    tauri_plugin_global_shortcut::Builder::new()
                        .with_handler(|app, sc, ev| {
                            if ev.state != ShortcutState::Pressed {
                                return;
                            }
                            let sh = app.state::<S>();
                            let quick = sh.d.lock().unwrap().settings.sc_quick.clone();
                            let is_quick = quick.parse::<tauri_plugin_global_shortcut::Shortcut>().map_or(false, |q| q.id() == sc.id());
                            if is_quick {
                                open_quick(app, &sh);
                            } else {
                                let next = if current_mode(&sh) == "full" { "mini" } else { "full" };
                                set_mode_inner(app, &sh, next);
                            }
                        })
                        .build(),
                )?;
                let (o, q) = {
                    let d = sh.d.lock().unwrap();
                    (d.settings.sc_open.clone(), d.settings.sc_quick.clone())
                };
                register_sc(app.handle(), "open", &o);
                register_sc(app.handle(), "quick", &q);
            }

            // Zone de notification
            let open = MenuItem::with_id(app, "open", tr("tray.open"), true, None::<&str>)?;
            let mini = MenuItem::with_id(app, "mini", tr("tray.mini"), true, None::<&str>)?;
            let quickrem = MenuItem::with_id(app, "quickrem", tr("tray.quickrem"), true, None::<&str>)?;
            let quick = MenuItem::with_id(app, "quick", "Focus", true, None::<&str>)?;
            let stop_i = MenuItem::with_id(app, "stop", tr("tray.stop"), false, None::<&str>)?;
            let quit_i = MenuItem::with_id(app, "quit", tr("tray.quit"), true, None::<&str>)?;
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
                        "quickrem" => open_quick(app, &sh),
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
            let labels = vec![(open.clone(), "tray.open"), (mini.clone(), "tray.mini"), (quickrem.clone(), "tray.quickrem"), (stop_i.clone(), "tray.stop"), (quit_i.clone(), "tray.quit")];
            app.manage(TrayItems { tray, quick, stop: stop_i, labels, last: Mutex::new(String::new()) });

            let mode = {
                let d = sh.d.lock().unwrap();
                apply_tray(app.handle(), TrayInfo::of(&d));
                if first_run { "full".to_string() } else { d.settings.mode.clone() }
            };
            // Fenêtre principale et bulle créées une fois au démarrage, comme en 1.4.0 (stable)
            ensure_window(app.handle(), "main");
            ensure_window(app.handle(), "mini");
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
        .expect("Cap n'a pas pu démarrer"); // i18n-ok : message de plantage, jamais affiché
}
