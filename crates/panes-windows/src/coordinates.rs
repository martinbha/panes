use panes_core::{Point, Rect};

/// Converts between Windows' y-down virtual-desktop coordinates and the
/// shared panes y-up coordinate space. The primary monitor's top edge is the
/// common origin for both systems, so its bottom edge provides the flip axis.
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) struct CoordinateSpace {
    desktop_top: f64,
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
impl CoordinateSpace {
    #[must_use]
    pub(crate) fn from_primary_frame(primary_frame: Rect) -> Self {
        Self {
            desktop_top: primary_frame.max_y(),
        }
    }

    #[must_use]
    pub(crate) fn native_rect_to_panes(self, rect: Rect) -> Rect {
        Rect::new(
            rect.origin.x,
            self.desktop_top - rect.origin.y - rect.size.height,
            rect.size.width,
            rect.size.height,
        )
    }

    #[must_use]
    pub(crate) fn panes_rect_to_native(self, rect: Rect) -> Rect {
        self.native_rect_to_panes(rect)
    }

    #[must_use]
    pub(crate) fn native_point_to_panes(self, point: Point) -> Point {
        Point::new(point.x, self.desktop_top - point.y)
    }
}

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
    use panes_core::{Command, LayoutConfig, LayoutRequest, calculate};

    use super::*;

    #[test]
    fn preserves_negative_virtual_desktop_coordinates() {
        assert_eq!(
            rect_from_edges(-1920, -100, 0, 980),
            Rect::new(-1920.0, -100.0, 1920.0, 1080.0)
        );
    }

    #[test]
    fn converts_window_rects_on_the_primary_display() {
        let space = CoordinateSpace::from_primary_frame(Rect::new(0.0, 0.0, 1920.0, 1080.0));
        let native = Rect::new(100.0, 80.0, 640.0, 300.0);

        let panes = space.native_rect_to_panes(native);

        assert_eq!(panes, Rect::new(100.0, 700.0, 640.0, 300.0));
        assert_eq!(space.panes_rect_to_native(panes), native);
    }

    #[test]
    fn keeps_displays_above_and_below_the_primary_in_their_physical_direction() {
        let primary = Rect::new(0.0, 0.0, 1920.0, 1080.0);
        let space = CoordinateSpace::from_primary_frame(primary);
        let above = space.native_rect_to_panes(Rect::new(0.0, -720.0, 1280.0, 720.0));
        let below = space.native_rect_to_panes(Rect::new(0.0, 1080.0, 1280.0, 720.0));
        let left = space.native_rect_to_panes(Rect::new(-1280.0, 120.0, 1280.0, 720.0));

        assert_eq!(above, Rect::new(0.0, 1080.0, 1280.0, 720.0));
        assert_eq!(below, Rect::new(0.0, -720.0, 1280.0, 720.0));
        assert_eq!(left, Rect::new(-1280.0, 240.0, 1280.0, 720.0));
        assert_eq!(
            space.panes_rect_to_native(above),
            Rect::new(0.0, -720.0, 1280.0, 720.0)
        );
        assert_eq!(
            space.panes_rect_to_native(below),
            Rect::new(0.0, 1080.0, 1280.0, 720.0)
        );
    }

    #[test]
    fn converts_cursor_points() {
        let space = CoordinateSpace::from_primary_frame(Rect::new(0.0, 0.0, 1920.0, 1080.0));

        assert_eq!(
            space.native_point_to_panes(Point::new(200.0, 300.0)),
            Point::new(200.0, 780.0)
        );
    }

    #[test]
    fn converts_a_top_half_layout_back_to_the_native_top_edge() {
        let space = CoordinateSpace::from_primary_frame(Rect::new(0.0, 0.0, 1920.0, 1080.0));
        let layout = calculate(
            LayoutRequest {
                command: Command::TopHalf,
                window: Rect::new(100.0, 100.0, 640.0, 400.0),
                screen: Rect::new(0.0, 0.0, 1920.0, 1080.0),
            },
            &LayoutConfig::default(),
        )
        .rect;

        assert_eq!(
            space.panes_rect_to_native(layout),
            Rect::new(0.0, 0.0, 1920.0, 540.0)
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
