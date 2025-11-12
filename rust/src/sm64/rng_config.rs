pub struct RandomConfig {
    pub(crate) seed: u32,
    pub(crate) max_random_action: u32,
    pub(crate) max_window_length: u32,
    pub(crate) a_prob: f32,
    pub(crate) b_prob: f32,
    pub(crate) z_prob: f32,
}
impl RandomConfig {
    pub fn new(seed: u32) -> Self {
        RandomConfig::full_new(
            seed, 
            5, 
            100, 
            0.5, 
            0.5, 
            0.2)
    }

    pub fn full_new(seed: u32, max_random_action: u32, max_window_length: u32, a_prob: f32, b_prob: f32, z_prob: f32) -> Self {
        return RandomConfig { 
            seed, 
            max_random_action,
            max_window_length, 
            a_prob, 
            b_prob,
            z_prob 
        }
    }
}