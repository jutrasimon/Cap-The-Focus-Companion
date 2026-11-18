//! Jauge d'activité d'un dossier : 100 % quand on clique « Actif », descend lentement ensuite.

pub const DAY_MS: f64 = 86_400_000.0;

/// Niveau en % (0 à 100) selon le temps écoulé depuis la dernière activation.
pub fn level(last_active: u64, now: u64, decay_pct_per_day: f64) -> f64 {
    let days = now.saturating_sub(last_active) as f64 / DAY_MS;
    (100.0 - days * decay_pct_per_day.max(0.0)).clamp(0.0, 100.0)
}

/// Millisecondes avant que le dossier soit perdu.
pub fn ms_left(last_active: u64, now: u64, decay_pct_per_day: f64) -> u64 {
    if decay_pct_per_day <= 0.0 {
        return u64::MAX;
    }
    let total = (100.0 / decay_pct_per_day * DAY_MS) as u64;
    (last_active + total).saturating_sub(now)
}

#[cfg(test)]
mod tests {
    use super::*;
    const D: u64 = 86_400_000;

    #[test]
    fn plein_a_l_activation() {
        assert_eq!(level(1000, 1000, 10.0), 100.0);
    }
    #[test]
    fn dix_pourcent_par_jour() {
        assert!((level(0, 3 * D, 10.0) - 70.0).abs() < 1e-9);
    }
    #[test]
    fn perdu_apres_dix_jours() {
        assert_eq!(level(0, 10 * D, 10.0), 0.0);
        assert_eq!(level(0, 30 * D, 10.0), 0.0);
        assert_eq!(ms_left(0, 4 * D, 10.0), 6 * D);
    }
    #[test]
    fn declin_nul() {
        assert_eq!(level(0, 365 * D, 0.0), 100.0);
    }
}
