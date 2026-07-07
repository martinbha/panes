#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Size {
    pub width: f64,
    pub height: f64,
}

impl Size {
    #[must_use]
    pub const fn new(width: f64, height: f64) -> Self {
        Self { width, height }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

impl Rect {
    #[must_use]
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            origin: Point::new(x, y),
            size: Size::new(width, height),
        }
    }

    #[must_use]
    pub fn min_x(self) -> f64 {
        self.origin.x
    }

    #[must_use]
    pub fn max_x(self) -> f64 {
        self.origin.x + self.size.width
    }

    #[must_use]
    pub fn min_y(self) -> f64 {
        self.origin.y
    }

    #[must_use]
    pub fn max_y(self) -> f64 {
        self.origin.y + self.size.height
    }

    #[must_use]
    pub fn area(self) -> f64 {
        self.size.width.max(0.0) * self.size.height.max(0.0)
    }

    #[must_use]
    pub fn mid_x(self) -> f64 {
        self.origin.x + self.size.width / 2.0
    }

    #[must_use]
    pub fn mid_y(self) -> f64 {
        self.origin.y + self.size.height / 2.0
    }

    #[must_use]
    pub fn orientation(self) -> Orientation {
        if self.size.width >= self.size.height {
            Orientation::Landscape
        } else {
            Orientation::Portrait
        }
    }

    #[must_use]
    pub fn inset(self, horizontal: f64, vertical: f64) -> Self {
        Self::new(
            self.origin.x + horizontal,
            self.origin.y + vertical,
            (self.size.width - horizontal * 2.0).max(0.0),
            (self.size.height - vertical * 2.0).max(0.0),
        )
    }

    #[must_use]
    pub fn contains_point(self, point: Point) -> bool {
        point.x >= self.min_x()
            && point.x <= self.max_x()
            && point.y >= self.min_y()
            && point.y <= self.max_y()
    }

    #[must_use]
    pub fn intersection(self, other: Self) -> Option<Self> {
        let min_x = self.min_x().max(other.min_x());
        let min_y = self.min_y().max(other.min_y());
        let max_x = self.max_x().min(other.max_x());
        let max_y = self.max_y().min(other.max_y());

        if max_x <= min_x || max_y <= min_y {
            return None;
        }

        Some(Self::new(min_x, min_y, max_x - min_x, max_y - min_y))
    }

    #[must_use]
    pub fn with_origin(self, x: f64, y: f64) -> Self {
        Self::new(x, y, self.size.width, self.size.height)
    }

    #[must_use]
    pub fn with_size(self, width: f64, height: f64) -> Self {
        Self::new(self.origin.x, self.origin.y, width, height)
    }

    #[must_use]
    pub fn centered_in(self, container: Self) -> Self {
        Self::new(
            container.origin.x + ((container.size.width - self.size.width) / 2.0).round(),
            container.origin.y + ((container.size.height - self.size.height) / 2.0).round(),
            self.size.width,
            self.size.height,
        )
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Orientation {
    Landscape,
    Portrait,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Edge(u8);

impl Edge {
    pub const NONE: Self = Self(0);
    pub const LEFT: Self = Self(1 << 0);
    pub const RIGHT: Self = Self(1 << 1);
    pub const TOP: Self = Self(1 << 2);
    pub const BOTTOM: Self = Self(1 << 3);

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_points_inside_rect_bounds() {
        let rect = Rect::new(10.0, 20.0, 100.0, 200.0);

        assert!(rect.contains_point(Point::new(10.0, 20.0)));
        assert!(rect.contains_point(Point::new(50.0, 100.0)));
        assert!(rect.contains_point(Point::new(110.0, 220.0)));
        assert!(!rect.contains_point(Point::new(9.0, 100.0)));
        assert!(!rect.contains_point(Point::new(50.0, 221.0)));
    }

    #[test]
    fn calculates_intersection_area() {
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let other = Rect::new(50.0, 25.0, 100.0, 100.0);

        let intersection = rect.intersection(other).expect("rects should overlap");

        assert_eq!(intersection, Rect::new(50.0, 25.0, 50.0, 75.0));
        assert_eq!(intersection.area(), 3750.0);
    }

    #[test]
    fn touching_edges_do_not_intersect() {
        let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
        let other = Rect::new(100.0, 0.0, 100.0, 100.0);

        assert_eq!(rect.intersection(other), None);
    }
}
