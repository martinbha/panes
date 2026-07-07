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
        Command::Grow => resize_from_center(request.window, config.resize_step),
        Command::Shrink => resize_from_center(request.window, -config.resize_step),
        Command::NextDisplay | Command::PreviousDisplay => request.window.centered_in(screen),
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

fn resize_from_center(window: Rect, delta: f64) -> Rect {
    let width = (window.size.width + delta).max(1.0);
    let height = (window.size.height + delta).max(1.0);
    Rect::new(
        window.mid_x() - width / 2.0,
        window.mid_y() - height / 2.0,
        width,
        height,
    )
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
        | Command::Shrink
        | Command::NextDisplay
        | Command::PreviousDisplay => rect,
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
