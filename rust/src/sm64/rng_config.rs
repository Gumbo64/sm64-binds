#[derive(Debug, Clone, Copy)]
pub struct RngConfig {
    pub(crate) random_amount: u32,
    pub(crate) random_burst_length: u32,
    pub(crate) window_length: u32,
    pub(crate) a_prob: f32,
    pub(crate) b_prob: f32,
    pub(crate) z_prob: f32,
}
impl RngConfig {
    pub const fn default() -> Self {
        RngConfig::new(
            5, 
            2,
            100, 
            0.5, 
            0.5, 
            0.2)
    }

    pub const fn new(random_amount: u32, random_burst_length: u32, window_length: u32, a_prob: f32, b_prob: f32, z_prob: f32) -> Self {
        return RngConfig { 
            random_amount,
            random_burst_length,
            window_length, 
            a_prob, 
            b_prob,
            z_prob 
        }
    }
}