use std::collections::HashMap;

use crate::{Command, Rect};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct WindowId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecordedCommand {
    pub command: Command,
    /// The rectangle reported by the platform after applying the command.
    pub rect: Rect,
    /// The exact logical rectangle requested before native rounding or app constraints.
    pub requested_rect: Rect,
    pub count: u32,
}

#[derive(Debug, Default)]
pub struct WindowHistory {
    restore_rects: HashMap<WindowId, Rect>,
    last_commands: HashMap<WindowId, RecordedCommand>,
}

impl WindowHistory {
    #[must_use]
    pub fn restore_rect(&self, window_id: WindowId) -> Option<Rect> {
        self.restore_rects.get(&window_id).copied()
    }

    pub fn set_restore_rect(&mut self, window_id: WindowId, rect: Rect) {
        self.restore_rects.insert(window_id, rect);
    }

    pub fn clear_restore_rect(&mut self, window_id: WindowId) {
        self.restore_rects.remove(&window_id);
    }

    #[must_use]
    pub fn last_command(&self, window_id: WindowId) -> Option<RecordedCommand> {
        self.last_commands.get(&window_id).copied()
    }

    pub fn record_command(&mut self, window_id: WindowId, command: Command, rect: Rect) {
        self.record_applied_command(window_id, command, rect, rect);
    }

    pub fn record_applied_command(
        &mut self,
        window_id: WindowId,
        command: Command,
        requested_rect: Rect,
        applied_rect: Rect,
    ) {
        let count = self
            .last_commands
            .get(&window_id)
            .filter(|record| record.command == command)
            .map_or(1, |record| record.count + 1);

        self.last_commands.insert(
            window_id,
            RecordedCommand {
                command,
                rect: applied_rect,
                requested_rect,
                count,
            },
        );
    }

    pub fn clear_last_command(&mut self, window_id: WindowId) {
        self.last_commands.remove(&window_id);
    }

    pub fn clear_window(&mut self, window_id: WindowId) {
        self.clear_restore_rect(window_id);
        self.clear_last_command(window_id);
    }

    pub fn clear(&mut self) {
        self.restore_rects.clear();
        self.last_commands.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_requested_and_applied_rects_separately() {
        let mut history = WindowHistory::default();
        let requested = Rect::new(10.5, 20.5, 300.0, 200.0);
        let applied = Rect::new(11.0, 21.0, 300.0, 200.0);

        history.record_applied_command(WindowId(1), Command::Grow, requested, applied);

        let record = history.last_command(WindowId(1)).unwrap();
        assert_eq!(record.requested_rect, requested);
        assert_eq!(record.rect, applied);
    }

    #[test]
    fn clears_all_history_for_one_window() {
        let mut history = WindowHistory::default();
        history.set_restore_rect(WindowId(1), Rect::new(0.0, 0.0, 100.0, 100.0));
        history.record_command(
            WindowId(1),
            Command::Maximize,
            Rect::new(0.0, 0.0, 500.0, 500.0),
        );

        history.clear_window(WindowId(1));

        assert_eq!(history.restore_rect(WindowId(1)), None);
        assert_eq!(history.last_command(WindowId(1)), None);
    }
}
