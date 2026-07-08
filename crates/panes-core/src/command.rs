#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum Command {
    LeftHalf,
    RightHalf,
    CenterHalf,
    TopHalf,
    BottomHalf,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    FirstThird,
    CenterThird,
    LastThird,
    FirstTwoThirds,
    CenterTwoThirds,
    LastTwoThirds,
    Maximize,
    AlmostMaximize,
    MaximizeHeight,
    Center,
    Restore,
    MoveLeft,
    MoveRight,
    MoveUp,
    MoveDown,
    Grow,
    Shrink,
}

impl Command {
    pub const ALL: &'static [Self] = &[
        Self::LeftHalf,
        Self::RightHalf,
        Self::CenterHalf,
        Self::TopHalf,
        Self::BottomHalf,
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
        Self::FirstThird,
        Self::CenterThird,
        Self::LastThird,
        Self::FirstTwoThirds,
        Self::CenterTwoThirds,
        Self::LastTwoThirds,
        Self::Maximize,
        Self::AlmostMaximize,
        Self::MaximizeHeight,
        Self::Center,
        Self::Restore,
        Self::MoveLeft,
        Self::MoveRight,
        Self::MoveUp,
        Self::MoveDown,
        Self::Grow,
        Self::Shrink,
    ];

    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::LeftHalf => "left-half",
            Self::RightHalf => "right-half",
            Self::CenterHalf => "center-half",
            Self::TopHalf => "top-half",
            Self::BottomHalf => "bottom-half",
            Self::TopLeft => "top-left",
            Self::TopRight => "top-right",
            Self::BottomLeft => "bottom-left",
            Self::BottomRight => "bottom-right",
            Self::FirstThird => "first-third",
            Self::CenterThird => "center-third",
            Self::LastThird => "last-third",
            Self::FirstTwoThirds => "first-two-thirds",
            Self::CenterTwoThirds => "center-two-thirds",
            Self::LastTwoThirds => "last-two-thirds",
            Self::Maximize => "maximize",
            Self::AlmostMaximize => "almost-maximize",
            Self::MaximizeHeight => "maximize-height",
            Self::Center => "center",
            Self::Restore => "restore",
            Self::MoveLeft => "move-left",
            Self::MoveRight => "move-right",
            Self::MoveUp => "move-up",
            Self::MoveDown => "move-down",
            Self::Grow => "grow",
            Self::Shrink => "shrink",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|command| command.id() == id)
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::LeftHalf => "Left Half",
            Self::RightHalf => "Right Half",
            Self::CenterHalf => "Center Half",
            Self::TopHalf => "Top Half",
            Self::BottomHalf => "Bottom Half",
            Self::TopLeft => "Top Left",
            Self::TopRight => "Top Right",
            Self::BottomLeft => "Bottom Left",
            Self::BottomRight => "Bottom Right",
            Self::FirstThird => "First Third",
            Self::CenterThird => "Center Third",
            Self::LastThird => "Last Third",
            Self::FirstTwoThirds => "First Two Thirds",
            Self::CenterTwoThirds => "Center Two Thirds",
            Self::LastTwoThirds => "Last Two Thirds",
            Self::Maximize => "Maximize",
            Self::AlmostMaximize => "Almost Maximize",
            Self::MaximizeHeight => "Maximize Height",
            Self::Center => "Center",
            Self::Restore => "Restore",
            Self::MoveLeft => "Move Left",
            Self::MoveRight => "Move Right",
            Self::MoveUp => "Move Up",
            Self::MoveDown => "Move Down",
            Self::Grow => "Grow",
            Self::Shrink => "Shrink",
        }
    }

    #[must_use]
    pub const fn category(self) -> CommandCategory {
        match self {
            Self::LeftHalf
            | Self::RightHalf
            | Self::CenterHalf
            | Self::TopHalf
            | Self::BottomHalf => CommandCategory::Halves,
            Self::TopLeft | Self::TopRight | Self::BottomLeft | Self::BottomRight => {
                CommandCategory::Corners
            }
            Self::FirstThird
            | Self::CenterThird
            | Self::LastThird
            | Self::FirstTwoThirds
            | Self::CenterTwoThirds
            | Self::LastTwoThirds => CommandCategory::Thirds,
            Self::Maximize
            | Self::AlmostMaximize
            | Self::MaximizeHeight
            | Self::Center
            | Self::Restore => CommandCategory::SizeAndPosition,
            Self::MoveLeft | Self::MoveRight | Self::MoveUp | Self::MoveDown => {
                CommandCategory::Move
            }
            Self::Grow | Self::Shrink => CommandCategory::Resize,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub enum CommandCategory {
    Halves,
    Corners,
    Thirds,
    SizeAndPosition,
    Move,
    Resize,
}

impl CommandCategory {
    pub const ALL: &'static [Self] = &[
        Self::Halves,
        Self::Corners,
        Self::Thirds,
        Self::SizeAndPosition,
        Self::Move,
        Self::Resize,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Halves => "Halves",
            Self::Corners => "Corners",
            Self::Thirds => "Thirds",
            Self::SizeAndPosition => "Size and Position",
            Self::Move => "Move",
            Self::Resize => "Resize",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_id_round_trips_every_command() {
        for &command in Command::ALL {
            assert_eq!(Command::from_id(command.id()), Some(command));
        }
    }

    #[test]
    fn from_id_rejects_unknown_ids() {
        assert_eq!(Command::from_id("left-halff"), None);
        assert_eq!(Command::from_id(""), None);
        assert_eq!(Command::from_id("Left Half"), None);
    }
}
