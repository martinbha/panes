use panes_core::{Point, Rect};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CoordinateSpace {
    desktop_top: f64,
}

impl CoordinateSpace {
    #[must_use]
    pub(crate) fn from_screen_frames(frames: &[Rect]) -> Option<Self> {
        let desktop_top = frames
            .iter()
            .map(|frame| frame.max_y())
            .max_by(f64::total_cmp)?;

        Some(Self { desktop_top })
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
        Rect::new(
            rect.origin.x,
            self.desktop_top - rect.origin.y - rect.size.height,
            rect.size.width,
            rect.size.height,
        )
    }

    #[must_use]
    pub(crate) fn native_point_to_panes(self, point: Point) -> Point {
        Point::new(point.x, self.desktop_top - point.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_window_rects_on_single_display() {
        let space = CoordinateSpace::from_screen_frames(&[Rect::new(0.0, 0.0, 1440.0, 900.0)])
            .expect("screen frame");
        let native = Rect::new(100.0, 80.0, 400.0, 300.0);

        let panes = space.native_rect_to_panes(native);

        assert_eq!(panes, Rect::new(100.0, 520.0, 400.0, 300.0));
        assert_eq!(space.panes_rect_to_native(panes), native);
    }

    #[test]
    fn uses_highest_display_edge_for_vertical_conversion() {
        let space = CoordinateSpace::from_screen_frames(&[
            Rect::new(0.0, 0.0, 1440.0, 900.0),
            Rect::new(1440.0, 900.0, 1024.0, 768.0),
        ])
        .expect("screen frames");
        let native = Rect::new(1500.0, 48.0, 640.0, 480.0);

        let panes = space.native_rect_to_panes(native);

        assert_eq!(panes, Rect::new(1500.0, 1140.0, 640.0, 480.0));
        assert_eq!(space.panes_rect_to_native(panes), native);
    }

    #[test]
    fn keeps_negative_horizontal_origins_stable() {
        let space = CoordinateSpace::from_screen_frames(&[
            Rect::new(0.0, 0.0, 1440.0, 900.0),
            Rect::new(-1280.0, 120.0, 1280.0, 720.0),
        ])
        .expect("screen frames");

        let panes = space.native_rect_to_panes(Rect::new(-1200.0, 200.0, 500.0, 300.0));

        assert_eq!(panes, Rect::new(-1200.0, 400.0, 500.0, 300.0));
    }

    #[test]
    fn converts_work_area_rects_with_menu_bar_offsets() {
        let space = CoordinateSpace::from_screen_frames(&[Rect::new(0.0, 0.0, 1440.0, 900.0)])
            .expect("screen frame");

        let panes = space.native_rect_to_panes(Rect::new(0.0, 25.0, 1440.0, 850.0));

        assert_eq!(panes, Rect::new(0.0, 25.0, 1440.0, 850.0));
    }

    #[test]
    fn converts_cursor_points() {
        let space = CoordinateSpace::from_screen_frames(&[Rect::new(0.0, 0.0, 1440.0, 900.0)])
            .expect("screen frame");

        assert_eq!(
            space.native_point_to_panes(Point::new(200.0, 300.0)),
            Point::new(200.0, 600.0)
        );
    }

    #[test]
    fn returns_none_without_screen_frames() {
        assert_eq!(CoordinateSpace::from_screen_frames(&[]), None);
    }
}
