use crate::{Command, Edge, LayoutConfig, Orientation, Rect};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutRequest {
    pub command: Command,
    pub window: Rect,
    pub screen: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutResult {
    pub command: Command,
    pub rect: Rect,
}

#[must_use]
pub fn calculate(request: LayoutRequest, config: &LayoutConfig) -> LayoutResult {
    let config = config.sanitized();
    let screen = request.screen;
    let rect = match request.command {
        Command::LeftHalf => horizontal_split(screen, Edge::LEFT, config.horizontal_split),
        Command::RightHalf => horizontal_split(screen, Edge::RIGHT, 1.0 - config.horizontal_split),
        Command::CenterHalf => center_half(screen),
        Command::TopHalf => vertical_split(screen, Edge::TOP, config.vertical_split),
        Command::BottomHalf => vertical_split(screen, Edge::BOTTOM, 1.0 - config.vertical_split),
        Command::TopLeft => corner(screen, Edge::LEFT.union(Edge::TOP), &config),
        Command::TopRight => corner(screen, Edge::RIGHT.union(Edge::TOP), &config),
        Command::BottomLeft => corner(screen, Edge::LEFT.union(Edge::BOTTOM), &config),
        Command::BottomRight => corner(screen, Edge::RIGHT.union(Edge::BOTTOM), &config),
        Command::FirstThird => third(screen, 0, 1),
        Command::CenterThird => match screen.orientation() {
            Orientation::Landscape => third(screen, 1, 1),
            Orientation::Portrait => horizontal_band(screen, 1.0 / 3.0, 1.0 / 3.0),
        },
        Command::LastThird => third(screen, 2, 1),
        Command::FirstTwoThirds => third(screen, 0, 2),
        Command::CenterTwoThirds => match screen.orientation() {
            Orientation::Landscape => vertical_band(screen, 1.0 / 6.0, 2.0 / 3.0),
            Orientation::Portrait => horizontal_band(screen, 1.0 / 6.0, 2.0 / 3.0),
        },
        Command::LastTwoThirds => third(screen, 1, 2),
        Command::Maximize => screen,
        Command::AlmostMaximize => almost_maximize(screen, &config),
        Command::MaximizeHeight => Rect::new(
            request.window.origin.x,
            screen.origin.y,
            request.window.size.width,
            screen.size.height,
        ),
        Command::Center => request.window.centered_in(screen),
        Command::Restore => request.window,
        Command::MoveLeft => request
            .window
            .with_origin(screen.min_x(), request.window.origin.y),
        Command::MoveRight => request.window.with_origin(
            screen.max_x() - request.window.size.width,
            request.window.origin.y,
        ),
        Command::MoveUp => request.window.with_origin(
            request.window.origin.x,
            screen.max_y() - request.window.size.height,
        ),
        Command::MoveDown => request
            .window
            .with_origin(request.window.origin.x, screen.min_y()),
        Command::Grow => resize_from_center(request.window, config.resize_step, screen),
        Command::Shrink => resize_from_center(request.window, -config.resize_step, screen),
    };

    LayoutResult {
        command: request.command,
        rect: apply_gap(rect, request.command, config.gap),
    }
}

fn horizontal_split(screen: Rect, edge: Edge, width_fraction: f64) -> Rect {
    let width = (screen.size.width * width_fraction).floor();
    let x = if edge.contains(Edge::RIGHT) {
        screen.max_x() - width
    } else {
        screen.min_x()
    };
    Rect::new(x, screen.min_y(), width, screen.size.height)
}

fn vertical_split(screen: Rect, edge: Edge, height_fraction: f64) -> Rect {
    let height = (screen.size.height * height_fraction).floor();
    let y = if edge.contains(Edge::TOP) {
        screen.max_y() - height
    } else {
        screen.min_y()
    };
    Rect::new(screen.min_x(), y, screen.size.width, height)
}

fn corner(screen: Rect, edges: Edge, config: &LayoutConfig) -> Rect {
    let width_fraction = if edges.contains(Edge::RIGHT) {
        1.0 - config.horizontal_split
    } else {
        config.horizontal_split
    };
    let height_fraction = if edges.contains(Edge::BOTTOM) {
        1.0 - config.vertical_split
    } else {
        config.vertical_split
    };
    let width = (screen.size.width * width_fraction).floor();
    let height = (screen.size.height * height_fraction).floor();
    let x = if edges.contains(Edge::RIGHT) {
        screen.max_x() - width
    } else {
        screen.min_x()
    };
    let y = if edges.contains(Edge::TOP) {
        screen.max_y() - height
    } else {
        screen.min_y()
    };
    Rect::new(x, y, width, height)
}

fn center_half(screen: Rect) -> Rect {
    match screen.orientation() {
        Orientation::Landscape => {
            let width = (screen.size.width / 2.0).round();
            Rect::new(screen.min_x(), screen.min_y(), width, screen.size.height).centered_in(screen)
        }
        Orientation::Portrait => {
            let height = (screen.size.height / 2.0).round();
            Rect::new(screen.min_x(), screen.min_y(), screen.size.width, height).centered_in(screen)
        }
    }
}

fn third(screen: Rect, offset: usize, span: usize) -> Rect {
    match screen.orientation() {
        Orientation::Landscape => {
            let unit = (screen.size.width / 3.0).floor();
            Rect::new(
                screen.min_x() + unit * offset as f64,
                screen.min_y(),
                unit * span as f64,
                screen.size.height,
            )
        }
        Orientation::Portrait => {
            let unit = (screen.size.height / 3.0).floor();
            let y = screen.max_y() - unit * (offset + span) as f64;
            Rect::new(screen.min_x(), y, screen.size.width, unit * span as f64)
        }
    }
}

fn vertical_band(screen: Rect, start_fraction: f64, width_fraction: f64) -> Rect {
    let width = (screen.size.width * width_fraction).floor();
    Rect::new(
        screen.min_x() + (screen.size.width * start_fraction).round(),
        screen.min_y(),
        width,
        screen.size.height,
    )
}

fn horizontal_band(screen: Rect, start_fraction: f64, height_fraction: f64) -> Rect {
    let height = (screen.size.height * height_fraction).floor();
    Rect::new(
        screen.min_x(),
        screen.min_y() + (screen.size.height * start_fraction).round(),
        screen.size.width,
        height,
    )
}

fn almost_maximize(screen: Rect, config: &LayoutConfig) -> Rect {
    let width = (screen.size.width * config.almost_maximize_width).round();
    let height = (screen.size.height * config.almost_maximize_height).round();
    Rect::new(screen.min_x(), screen.min_y(), width, height).centered_in(screen)
}

/// Fraction of the work area, per dimension, below which Shrink will not take a
/// window. Core cannot see per-app accessibility minimums, so this keeps
/// repeated Shrink presses from collapsing a window toward unusability.
const SHRINK_FLOOR_FRACTION: f64 = 0.25;

/// Resize `window` by `delta` on each dimension while keeping it centered, then
/// clamp it into `screen` (the work area). Grow caps each dimension at the work
/// area — at the cap a further Grow is a no-op equal to Maximize — and the
/// origin clamp lets growth continue in the available direction once an edge
/// meets the boundary. Shrink stops at [`SHRINK_FLOOR_FRACTION`] of the work
/// area. Both converge instead of running away.
fn resize_from_center(window: Rect, delta: f64, screen: Rect) -> Rect {
    let width = clamp_dimension(window.size.width, delta, screen.size.width);
    let height = clamp_dimension(window.size.height, delta, screen.size.height);
    // width/height never exceed the work area, so the upper bounds below are
    // always >= the lower bounds and `clamp` cannot panic.
    let x = (window.mid_x() - width / 2.0).clamp(screen.min_x(), screen.max_x() - width);
    let y = (window.mid_y() - height / 2.0).clamp(screen.min_y(), screen.max_y() - height);
    Rect::new(x, y, width, height)
}

/// Clamp a resized dimension between the shrink floor and the work-area extent.
/// The floor is capped at the current size so an already-tiny window is never
/// snapped *up* on Shrink; it simply stops shrinking.
fn clamp_dimension(current: f64, delta: f64, available: f64) -> f64 {
    let floor = (available * SHRINK_FLOOR_FRACTION).min(current);
    (current + delta).clamp(floor, available)
}

fn apply_gap(rect: Rect, command: Command, gap: f64) -> Rect {
    if gap <= 0.0 {
        return rect;
    }

    match command {
        Command::Restore
        | Command::Center
        | Command::AlmostMaximize
        | Command::MoveLeft
        | Command::MoveRight
        | Command::MoveUp
        | Command::MoveDown
        | Command::Grow
        | Command::Shrink => rect,
        _ => rect.inset(gap, gap),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Rect {
        Rect::new(10.0, 20.0, 1200.0, 900.0)
    }

    fn window() -> Rect {
        Rect::new(100.0, 120.0, 400.0, 300.0)
    }

    fn rect(command: Command) -> Rect {
        calculate(
            LayoutRequest {
                command,
                window: window(),
                screen: screen(),
            },
            &LayoutConfig::default(),
        )
        .rect
    }

    #[test]
    fn left_and_right_halves_use_split_ratio() {
        let config = LayoutConfig {
            horizontal_split: 0.6,
            ..LayoutConfig::default()
        };
        let left = calculate(
            LayoutRequest {
                command: Command::LeftHalf,
                window: window(),
                screen: screen(),
            },
            &config,
        )
        .rect;
        let right = calculate(
            LayoutRequest {
                command: Command::RightHalf,
                window: window(),
                screen: screen(),
            },
            &config,
        )
        .rect;

        assert_eq!(left, Rect::new(10.0, 20.0, 720.0, 900.0));
        assert_eq!(right, Rect::new(730.0, 20.0, 480.0, 900.0));
    }

    #[test]
    fn corners_use_split_ratios() {
        assert_eq!(rect(Command::TopLeft), Rect::new(10.0, 470.0, 600.0, 450.0));
        assert_eq!(
            rect(Command::BottomRight),
            Rect::new(610.0, 20.0, 600.0, 450.0)
        );
    }

    #[test]
    fn portrait_first_third_is_top_third() {
        let result = calculate(
            LayoutRequest {
                command: Command::FirstThird,
                window: window(),
                screen: Rect::new(0.0, 0.0, 900.0, 1200.0),
            },
            &LayoutConfig::default(),
        );

        assert_eq!(result.rect, Rect::new(0.0, 800.0, 900.0, 400.0));
    }

    fn resize(command: Command, window: Rect) -> Rect {
        calculate(
            LayoutRequest {
                command,
                window,
                screen: screen(),
            },
            &LayoutConfig::default(),
        )
        .rect
    }

    #[test]
    fn grow_near_an_edge_stays_within_the_work_area() {
        // A window pinned against the right edge grows leftward once its right
        // edge meets the boundary instead of pushing past it.
        let result = resize(Command::Grow, Rect::new(1000.0, 400.0, 200.0, 200.0));

        assert_eq!(result, Rect::new(980.0, 385.0, 230.0, 230.0));
        assert!(result.max_x() <= screen().max_x());
        assert!(result.min_x() >= screen().min_x());
    }

    #[test]
    fn grow_converges_to_the_work_area_and_stops() {
        let mut rect = window();
        for _ in 0..200 {
            rect = resize(Command::Grow, rect);
        }

        assert_eq!(rect, screen());
        // At the cap a further Grow is a no-op equal to Maximize.
        assert_eq!(resize(Command::Grow, rect), screen());
    }

    #[test]
    fn shrink_converges_to_the_floor_and_stops() {
        let mut rect = screen();
        for _ in 0..200 {
            rect = resize(Command::Shrink, rect);
        }

        // 25% of the 1200x900 work area, centered.
        assert_eq!(rect, Rect::new(460.0, 357.5, 300.0, 225.0));
        // At the floor a further Shrink is a no-op.
        assert_eq!(resize(Command::Shrink, rect), rect);
    }

    #[test]
    fn shrink_leaves_a_sub_floor_window_alone() {
        // Already smaller than the floor: Shrink must not snap it back up.
        let small = Rect::new(500.0, 400.0, 200.0, 150.0);

        assert_eq!(resize(Command::Shrink, small), small);
    }

    #[test]
    fn gaps_inset_tiling_commands() {
        let result = calculate(
            LayoutRequest {
                command: Command::Maximize,
                window: window(),
                screen: screen(),
            },
            &LayoutConfig {
                gap: 10.0,
                ..LayoutConfig::default()
            },
        );

        assert_eq!(result.rect, Rect::new(20.0, 30.0, 1180.0, 880.0));
    }
}
