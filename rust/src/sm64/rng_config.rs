/// Tuning for the RNG that perturbs controller inputs. Mirrors the arguments of
/// the `set_rng_config` WASM export.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RngConfig {
    pub window_length: u32,
    pub random_amount: u32,
    pub random_burst_length: u32,
    pub a_prob: f32,
    pub b_prob: f32,
    pub z_prob: f32,
}

impl RngConfig {
    pub const fn new(
        window_length: u32,
        random_amount: u32,
        random_burst_length: u32,
        a_prob: f32,
        b_prob: f32,
        z_prob: f32,
    ) -> Self {
        RngConfig {
            window_length,
            random_amount,
            random_burst_length,
            a_prob,
            b_prob,
            z_prob,
        }
    }
}

impl Default for RngConfig {
    fn default() -> Self {
        RngConfig::new(100, 5, 2, 0.5, 0.5, 0.2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_the_documented_configuration() {
        let cfg = RngConfig::default();
        assert_eq!(cfg.window_length, 100);
        assert_eq!(cfg.random_amount, 5);
        assert_eq!(cfg.random_burst_length, 2);
        assert_eq!(cfg.a_prob, 0.5);
        assert_eq!(cfg.b_prob, 0.5);
        assert_eq!(cfg.z_prob, 0.2);
    }

    #[test]
    fn new_stores_every_field() {
        let cfg = RngConfig::new(1, 2, 3, 0.1, 0.2, 0.3);
        assert_eq!(cfg.window_length, 1);
        assert_eq!(cfg.random_amount, 2);
        assert_eq!(cfg.random_burst_length, 3);
        assert_eq!(cfg.a_prob, 0.1);
        assert_eq!(cfg.b_prob, 0.2);
        assert_eq!(cfg.z_prob, 0.3);
    }

    #[test]
    fn defaults_are_unit_ranged_probabilities() {
        let cfg = RngConfig::default();
        for (name, p) in [
            ("a_prob", cfg.a_prob),
            ("b_prob", cfg.b_prob),
            ("z_prob", cfg.z_prob),
        ] {
            assert!(
                (0.0..=1.0).contains(&p),
                "{name} must be a probability, got {p}"
            );
        }
    }

    #[test]
    fn a_zero_window_disables_perturbation() {
        let cfg = RngConfig::new(0, 0, 0, 0.0, 0.0, 0.0);
        assert_eq!(cfg.random_amount, 0);
    }
}
