//! Jauges par site : se vident quand le site est utilisé, se rechargent sinon.
//! Logique pure, sans dépendance à Tauri, testable seule.

use serde::{Deserialize, Serialize};

/// Une jauge vide se débloque seulement quand au moins 1 minute s'est rechargée.
pub const UNLOCK_S: f64 = 60.0;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct GaugeCfg {
    /// Jauge active hors focus. Sinon le site n'est limité que pendant le focus.
    pub enabled: bool,
    /// Taille maximale, en minutes.
    pub max_min: u32,
    /// Vitesse d'usure : minutes de jauge consommées par minute passée sur le site.
    pub drain: f64,
    /// Vitesse de recharge : minutes de jauge regagnées par heure loin du site.
    pub refill_per_hour: u32,
}

impl Default for GaugeCfg {
    fn default() -> Self {
        GaugeCfg { enabled: true, max_min: 15, drain: 1.0, refill_per_hour: 15 }
    }
}

impl GaugeCfg {
    pub fn max_s(&self) -> f64 {
        self.max_min.max(1) as f64 * 60.0
    }
    /// Secondes de jauge regagnées par seconde.
    pub fn refill_rate(&self) -> f64 {
        self.refill_per_hour as f64 / 60.0
    }
    pub fn unlock_s(&self) -> f64 {
        UNLOCK_S.min(self.max_s())
    }
    pub fn sanitized(mut self) -> Self {
        self.max_min = self.max_min.clamp(1, 600);
        self.drain = if self.drain.is_finite() { self.drain.clamp(0.25, 10.0) } else { 1.0 };
        self.refill_per_hour = self.refill_per_hour.clamp(1, 600);
        self
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct GaugeState {
    pub level_s: f64,
    pub locked: bool,
}

impl Default for GaugeState {
    fn default() -> Self {
        GaugeState { level_s: f64::MAX, locked: false }
    }
}

/// Avance la jauge de `dt_s` secondes. `using` : le site est l'onglet actif d'un navigateur au premier plan.
/// Retourne `true` si l'état bloqué a changé.
pub fn step(cfg: &GaugeCfg, st: &mut GaugeState, dt_s: f64, using: bool) -> bool {
    let max = cfg.max_s();
    if !st.level_s.is_finite() || st.level_s > max {
        st.level_s = max;
    }
    if using && !st.locked {
        st.level_s -= dt_s * cfg.drain;
    } else {
        st.level_s += dt_s * cfg.refill_rate();
    }
    st.level_s = st.level_s.clamp(0.0, max);
    let was = st.locked;
    if st.level_s <= 0.0 {
        st.locked = true;
    } else if st.locked && st.level_s >= cfg.unlock_s() {
        st.locked = false;
    }
    was != st.locked
}

/// Secondes avant que le site se débloque (0 s'il ne l'est pas).
pub fn refill_in_s(cfg: &GaugeCfg, st: &GaugeState) -> f64 {
    if !st.locked {
        return 0.0;
    }
    ((cfg.unlock_s() - st.level_s).max(0.0) / cfg.refill_rate().max(1e-9)).ceil()
}

/// `host` appartient-il au domaine `site` (sous-domaines inclus) ?
pub fn host_matches(host: &str, site: &str) -> bool {
    let h = host.trim_start_matches("www.");
    h == site || h.ends_with(&format!(".{site}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> GaugeCfg {
        GaugeCfg::default() // 15 min, ×1, +15 min/h
    }

    #[test]
    fn nouvelle_jauge_pleine() {
        let mut s = GaugeState::default();
        step(&cfg(), &mut s, 0.0, false);
        assert_eq!(s.level_s, 900.0);
        assert!(!s.locked);
    }

    #[test]
    fn se_vide_a_l_usage_et_bloque_a_sec() {
        let mut s = GaugeState { level_s: 900.0, locked: false };
        for _ in 0..899 {
            assert!(!step(&cfg(), &mut s, 1.0, true));
        }
        assert!(!s.locked);
        assert!(step(&cfg(), &mut s, 1.0, true), "doit se bloquer à 0");
        assert!(s.locked);
        assert_eq!(s.level_s, 0.0);
    }

    #[test]
    fn deblocage_seulement_apres_une_minute_rechargee() {
        let c = cfg(); // +0,25 s de jauge par seconde : 1 min de jauge en 240 s
        let mut s = GaugeState { level_s: 0.0, locked: true };
        assert_eq!(refill_in_s(&c, &s), 240.0);
        for _ in 0..239 {
            assert!(!step(&c, &mut s, 1.0, false));
        }
        assert!(s.locked, "pas de déblocage à 59 s de jauge");
        assert!(step(&c, &mut s, 1.0, false));
        assert!(!s.locked);
        assert!((s.level_s - 60.0).abs() < 1e-6);
    }

    #[test]
    fn bloque_ne_consomme_pas() {
        let mut s = GaugeState { level_s: 0.0, locked: true };
        step(&cfg(), &mut s, 10.0, true);
        assert!(s.level_s > 0.0, "un site bloqué se recharge même si l'onglet est actif");
    }

    #[test]
    fn usure_double() {
        let c = GaugeCfg { drain: 2.0, ..cfg() };
        let mut s = GaugeState { level_s: 900.0, locked: false };
        step(&c, &mut s, 60.0, true);
        assert_eq!(s.level_s, 780.0);
    }

    #[test]
    fn nuit_complete_remplit() {
        let mut s = GaugeState { level_s: 0.0, locked: true };
        step(&cfg(), &mut s, 8.0 * 3600.0, false);
        assert_eq!(s.level_s, 900.0);
        assert!(!s.locked);
    }

    #[test]
    fn reduire_la_taille_plafonne() {
        let mut s = GaugeState { level_s: 900.0, locked: false };
        let c = GaugeCfg { max_min: 5, ..cfg() };
        step(&c, &mut s, 0.0, false);
        assert_eq!(s.level_s, 300.0);
    }

    #[test]
    fn domaines() {
        assert!(host_matches("www.facebook.com", "facebook.com"));
        assert!(host_matches("m.facebook.com", "facebook.com"));
        assert!(!host_matches("notfacebook.com", "facebook.com"));
        assert!(!host_matches("facebook.com.evil.io", "facebook.com"));
    }
}
