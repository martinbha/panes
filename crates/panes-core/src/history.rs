use std::collections::HashMap;

use crate::{Command, Rect};

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub struct WindowId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RecordedCommand {
    pub command: Command,
    pub rect: Rect,
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
        let count = self
            .last_commands
            .get(&window_id)
            .filter(|record| record.command == command)
            .map_or(1, |record| record.count + 1);

        self.last_commands.insert(
            window_id,
            RecordedCommand {
                command,
                rect,
                count,
            },
        );
    }

    pub fn clear_last_command(&mut self, window_id: WindowId) {
        self.last_commands.remove(&window_id);
    }
}
