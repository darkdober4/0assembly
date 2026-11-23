//! Statistiques de mesure : médiane, dispersion, effet minimum détectable.
//!
//! On ne raisonne jamais en moyenne : les mesures de programmes sont bruitées
//! (système, thermalisation, caches). La médiane et le MAD sont robustes.

/// Échantillon de durées, en millisecondes.
#[derive(Debug, Clone, Default)]
pub struct Samples {
    pub values: Vec<f64>,
}

impl Samples {
    pub fn new(values: Vec<f64>) -> Samples {
        Samples { values }
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Médiane.
    pub fn median(&self) -> f64 {
        median(&self.values)
    }

    /// Déviation absolue médiane.
    pub fn mad(&self) -> f64 {
        mad(&self.values)
    }

    /// Dispersion relative : MAD / médiane, en pourcentage.
    pub fn mad_pct(&self) -> f64 {
        let m = self.median();
        if m <= 0.0 {
            return 0.0;
        }
        self.mad() / m * 100.0
    }

    /// Effet minimum détectable à 95 %, en pourcentage.
    ///
    /// Le seuil de détection d'une différence de deux médianes est
    /// proportionnel à la dispersion et décroît comme `1/sqrt(n)`.
    pub fn mde_pct(&self) -> f64 {
        mde_pct(self.mad_pct(), self.values.len())
    }
}

/// Médiane d'une série.
pub fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = v.len() / 2;
    if v.len().is_multiple_of(2) {
        (v[mid - 1] + v[mid]) / 2.0
    } else {
        v[mid]
    }
}

/// Déviation absolue médiane.
pub fn mad(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let m = median(values);
    let devs: Vec<f64> = values.iter().map(|v| (v - m).abs()).collect();
    median(&devs)
}

/// Effet minimum détectable à 95 %, en pourcentage.
pub fn mde_pct(mad_relative_pct: f64, n: usize) -> f64 {
    if n == 0 {
        return f64::INFINITY;
    }
    // 1.253 (facteur de l'erreur standard de la médiane) x 1.96 (95 %) x sqrt(2)
    // (comparaison de deux médianes) : on retient 2.8 comme coefficient usuel.
    2.8 * mad_relative_pct / (n as f64).sqrt()
}

/// Compare deux échantillons : gain local et dispersion.
#[derive(Debug, Clone)]
pub struct Comparison {
    pub before_ms: f64,
    pub after_ms: f64,
    pub local_gain_pct: f64,
    pub mad_pct: f64,
    pub mde_pct: f64,
    pub n: usize,
    /// La différence est-elle distinguable du bruit ?
    pub significant: bool,
}

/// Compare deux échantillons (avant / après).
pub fn compare(before: &Samples, after: &Samples) -> Comparison {
    let before_ms = before.median();
    let after_ms = after.median();
    let local_gain_pct = if before_ms > 0.0 {
        (before_ms - after_ms) / before_ms * 100.0
    } else {
        0.0
    };
    let mad_pct = before.mad_pct().max(after.mad_pct());
    let n = before.len().min(after.len());
    let mde_pct = mde_pct(mad_pct, n);
    Comparison {
        before_ms,
        after_ms,
        local_gain_pct,
        mad_pct,
        mde_pct,
        n,
        significant: local_gain_pct.abs() > mde_pct,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_of_odd_and_even_sets() {
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 3.0, 2.0]), 2.5);
        assert_eq!(median(&[]), 0.0);
    }

    #[test]
    fn mad_ignores_outliers() {
        let clean = Samples::new(vec![10.0, 10.1, 9.9, 10.0, 10.05]);
        let noisy = Samples::new(vec![10.0, 10.1, 9.9, 10.0, 90.0]);
        assert!(clean.mad_pct() < noisy.mad_pct());
        assert!(clean.mad_pct() < 2.0);
    }

    #[test]
    fn mde_shrinks_with_more_samples() {
        let small = mde_pct(1.0, 10);
        let large = mde_pct(1.0, 1000);
        assert!(large < small);
    }

    #[test]
    fn comparison_flags_significant_gains() {
        let before = Samples::new(vec![100.0; 30]);
        let after = Samples::new(vec![80.0; 30]);
        let c = compare(&before, &after);
        assert!((c.local_gain_pct - 20.0).abs() < 0.001);
        assert!(c.significant);

        let same = Samples::new(vec![100.0; 30]);
        let c2 = compare(&before, &same);
        assert!(!c2.significant);
    }

    #[test]
    fn regression_is_detected_as_negative_gain() {
        let before = Samples::new(vec![100.0; 10]);
        let after = Samples::new(vec![110.0; 10]);
        let c = compare(&before, &after);
        assert!(c.local_gain_pct < 0.0);
    }
}
