use panes_core::{Point, Rect};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CoordinateSpace {
    desktop_top: f64,
}

impl CoordinateSpace {
    /// Builds the conversion between native CG coordinates (y-down from the
    /// top of the primary screen) and panes coordinates (y-up AppKit space).
    ///
    /// `frames` must be AppKit screen frames in `NSScreen::screens` order:
    /// the first frame is the primary screen, whose origin is the global
    /// AppKit origin. Both coordinate systems flip about that screen's top
    /// edge, so using any other screen (for example one arranged above the
    /// primary) would offset every converted rect.
    #[must_use]
    pub(crate) fn from_screen_frames(frames: &[Rect]) -> Option<Self> {
        let desktop_top = frames.first()?.max_y();

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
    fn flips_about_the_primary_screen_even_with_a_display_above_it() {
        let primary = Rect::new(0.0, 0.0, 1440.0, 900.0);
        let above = Rect::new(0.0, 900.0, 1024.0, 768.0);
        let space = CoordinateSpace::from_screen_frames(&[primary, above]).expect("screen frames");

        // A window on the primary display stays within the primary frame.
        let on_primary = space.native_rect_to_panes(Rect::new(100.0, 100.0, 640.0, 300.0));
        assert_eq!(on_primary, Rect::new(100.0, 500.0, 640.0, 300.0));
        assert!(primary.contains_point(on_primary.origin));

        // A window on the display above the primary has a negative native y
        // and lands within that display's frame.
        let on_above = space.native_rect_to_panes(Rect::new(100.0, -668.0, 640.0, 480.0));
        assert_eq!(on_above, Rect::new(100.0, 1088.0, 640.0, 480.0));
        assert!(above.contains_point(on_above.origin));

        assert_eq!(
            space.panes_rect_to_native(on_above),
            Rect::new(100.0, -668.0, 640.0, 480.0)
        );
    }

    #[test]
    fn converts_rects_on_a_display_below_the_primary() {
        let primary = Rect::new(0.0, 0.0, 1440.0, 900.0);
        let below = Rect::new(0.0, -768.0, 1024.0, 768.0);
        let space = CoordinateSpace::from_screen_frames(&[primary, below]).expect("screen frames");

        let on_below = space.native_rect_to_panes(Rect::new(100.0, 1000.0, 500.0, 300.0));

        assert_eq!(on_below, Rect::new(100.0, -400.0, 500.0, 300.0));
        assert!(below.contains_point(on_below.origin));
        assert_eq!(
            space.panes_rect_to_native(on_below),
            Rect::new(100.0, 1000.0, 500.0, 300.0)
        );
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
