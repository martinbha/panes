use panes_core::Rect;

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn rect_from_edges(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    Rect::new(
        f64::from(left),
        f64::from(top),
        f64::from(right - left),
        f64::from(bottom - top),
    )
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn rounded_i32(value: f64) -> Option<i32> {
    if !value.is_finite() {
        return None;
    }

    let rounded = value.round();
    (rounded >= f64::from(i32::MIN) && rounded <= f64::from(i32::MAX)).then_some(rounded as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_negative_virtual_desktop_coordinates() {
        assert_eq!(
            rect_from_edges(-1920, -100, 0, 980),
            Rect::new(-1920.0, -100.0, 1920.0, 1080.0)
        );
    }

    #[test]
    fn rounds_valid_native_coordinates() {
        assert_eq!(rounded_i32(12.5), Some(13));
        assert_eq!(rounded_i32(-12.5), Some(-13));
        assert_eq!(rounded_i32(f64::from(i32::MAX)), Some(i32::MAX));
    }

    #[test]
    fn rejects_unrepresentable_native_coordinates() {
        assert_eq!(rounded_i32(f64::NAN), None);
        assert_eq!(rounded_i32(f64::INFINITY), None);
        assert_eq!(rounded_i32(f64::from(i32::MAX) + 1.0), None);
    }
}
