#[derive(Debug, Clone, PartialEq)]
pub struct LayoutConfig {
    pub gap: f64,
    pub horizontal_split: f64,
    pub vertical_split: f64,
    pub almost_maximize_width: f64,
    pub almost_maximize_height: f64,
    pub resize_step: f64,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            gap: 0.0,
            horizontal_split: 0.5,
            vertical_split: 0.5,
            almost_maximize_width: 0.9,
            almost_maximize_height: 0.9,
            resize_step: 30.0,
        }
    }
}

impl LayoutConfig {
    #[must_use]
    pub fn sanitized(&self) -> Self {
        Self {
            gap: self.gap.max(0.0),
            horizontal_split: sanitize_fraction(self.horizontal_split, 0.5),
            vertical_split: sanitize_fraction(self.vertical_split, 0.5),
            almost_maximize_width: sanitize_fraction(self.almost_maximize_width, 0.9),
            almost_maximize_height: sanitize_fraction(self.almost_maximize_height, 0.9),
            resize_step: self.resize_step.max(1.0),
        }
    }
}

fn sanitize_fraction(value: f64, fallback: f64) -> f64 {
    if value > 0.0 && value < 1.0 {
        value
    } else {
        fallback
    }
}
