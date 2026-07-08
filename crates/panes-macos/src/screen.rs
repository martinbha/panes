use core_graphics::{
    event::CGEvent,
    event_source::{CGEventSource, CGEventSourceStateID},
};
use objc2::MainThreadMarker;
use objc2_app_kit::NSScreen;
use objc2_foundation::{NSRect, NSUInteger};
use panes_core::{Point, Rect};
use panes_platform::{PlatformError, PlatformResult, ScreenId, ScreenInfo};

use crate::coordinates::CoordinateSpace;

pub(crate) fn cursor_position() -> PlatformResult<Point> {
    let space = coordinate_space()?;
    let source = CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|()| PlatformError::Native("failed to create macOS event source".to_owned()))?;
    let event = CGEvent::new(source)
        .map_err(|()| PlatformError::Native("failed to read macOS cursor event".to_owned()))?;
    let location = event.location();

    Ok(space.native_point_to_panes(Point::new(location.x, location.y)))
}

pub(crate) fn screens() -> PlatformResult<Vec<ScreenInfo>> {
    let screens = read_screens()?;
    if screens.is_empty() {
        return Err(PlatformError::NotFound("no macOS screens found"));
    }

    Ok(screens)
}

/// Reads only the screen frames, skipping the per-screen name and work-area
/// lookups that `screens` pays for; this runs on every command.
pub(crate) fn coordinate_space() -> PlatformResult<CoordinateSpace> {
    let mtm = main_thread_marker()?;
    let screens = NSScreen::screens(mtm);
    let count = screens.count();
    let mut frames = Vec::with_capacity(count);

    for index in 0..count {
        frames.push(ns_rect_to_rect(screens.objectAtIndex(index).frame()));
    }

    CoordinateSpace::from_screen_frames(&frames)
        .ok_or(PlatformError::NotFound("no macOS screens found"))
}

fn main_thread_marker() -> PlatformResult<MainThreadMarker> {
    MainThreadMarker::new().ok_or_else(|| {
        PlatformError::Native("macOS screen APIs must be called on the main thread".to_owned())
    })
}

fn read_screens() -> PlatformResult<Vec<ScreenInfo>> {
    let mtm = main_thread_marker()?;
    let screens = NSScreen::screens(mtm);
    let count = screens.count();
    let mut result = Vec::with_capacity(count);

    for index in 0..count {
        let screen = screens.objectAtIndex(index);
        result.push(screen_info(&screen, index));
    }

    Ok(result)
}

fn screen_info(screen: &NSScreen, index: NSUInteger) -> ScreenInfo {
    let frame = ns_rect_to_rect(screen.frame());
    let work_area = ns_rect_to_rect(screen.visibleFrame());
    let display_id = u64::from(screen.CGDirectDisplayID());
    let id = if display_id == 0 {
        index as u64
    } else {
        display_id
    };

    ScreenInfo {
        id: ScreenId(id),
        name: screen.localizedName().to_string(),
        frame,
        work_area,
    }
}

fn ns_rect_to_rect(rect: NSRect) -> Rect {
    Rect::new(
        rect.origin.x,
        rect.origin.y,
        rect.size.width,
        rect.size.height,
    )
}
