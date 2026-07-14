use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    ffi::c_void,
};

use accessibility::{AXAttribute, AXUIElement, Error as AccessibilityError};
use accessibility_sys::{
    AXError, AXUIElementGetPid, AXUIElementSetAttributeValue, AXValueCreate, AXValueGetType,
    AXValueGetTypeID, AXValueGetValue, AXValueRef, error_string, kAXErrorAPIDisabled,
    kAXErrorAttributeUnsupported, kAXErrorNoValue, kAXFloatingWindowSubrole,
    kAXFocusedApplicationAttribute, kAXPositionAttribute, kAXSizeAttribute, kAXSystemDialogSubrole,
    kAXSystemFloatingWindowSubrole, kAXValueTypeCGPoint, kAXValueTypeCGSize, kAXWindowRole, pid_t,
};
use core_foundation::{
    base::{CFType, CFTypeRef, TCFType},
    boolean::CFBoolean,
    string::CFString,
};
use core_graphics::geometry::{CGPoint, CGSize};
use objc2_app_kit::NSRunningApplication;
use panes_core::{MAX_TRACKED_WINDOWS, Rect, WindowId};
use panes_platform::{PlatformError, PlatformResult, WindowInfo};

use crate::{accessibility_authorization, coordinates::CoordinateSpace};

const AX_ENHANCED_USER_INTERFACE_ATTRIBUTE: &str = "AXEnhancedUserInterface";
const AX_FULL_SCREEN_ATTRIBUTE: &str = "AXFullScreen";

#[derive(Default)]
pub(crate) struct WindowCache {
    state: RefCell<WindowCacheState>,
}

#[derive(Default)]
struct WindowCacheState {
    windows: HashMap<WindowId, AXUIElement>,
    recent_windows: VecDeque<WindowId>,
}

impl WindowCache {
    pub(crate) fn remember(&self, id: WindowId, window: AXUIElement) {
        let mut state = self.state.borrow_mut();
        state.windows.insert(id, window);
        touch_cached_window(&mut state, id);
    }

    pub(crate) fn get(&self, id: WindowId) -> Option<AXUIElement> {
        let mut state = self.state.borrow_mut();
        let window = state.windows.get(&id).cloned();
        if window.is_some() {
            touch_cached_window(&mut state, id);
        }
        window
    }

    pub(crate) fn forget(&self, id: WindowId) {
        let mut state = self.state.borrow_mut();
        state.windows.remove(&id);
        remove_recent_window(&mut state.recent_windows, id);
    }

    /// AXUIElements that refer to the same window compare CFEqual, so a hit
    /// here preserves the window identity across successive accessibility
    /// reads.
    pub(crate) fn known_id(&self, window: &AXUIElement) -> Option<WindowId> {
        let mut state = self.state.borrow_mut();
        let id = state
            .windows
            .iter()
            .find_map(|(id, cached)| (cached == window).then_some(*id));
        if let Some(id) = id {
            touch_cached_window(&mut state, id);
        }
        id
    }
}

fn touch_cached_window(state: &mut WindowCacheState, id: WindowId) {
    remove_recent_window(&mut state.recent_windows, id);
    state.recent_windows.push_back(id);

    while state.recent_windows.len() > MAX_TRACKED_WINDOWS {
        if let Some(evicted) = state.recent_windows.pop_front() {
            state.windows.remove(&evicted);
        }
    }
}

fn remove_recent_window(recent_windows: &mut VecDeque<WindowId>, id: WindowId) {
    if let Some(index) = recent_windows.iter().position(|candidate| *candidate == id) {
        recent_windows.remove(index);
    }
}

pub(crate) fn front_window_in(
    cache: &WindowCache,
    space: CoordinateSpace,
) -> PlatformResult<Option<WindowInfo>> {
    ensure_accessibility_permission()?;

    // AXFocusedWindow only exists on application elements, so resolve the
    // focused application first; asking the system-wide element for it
    // always fails with kAXErrorAttributeUnsupported.
    let system = AXUIElement::system_wide();
    let _ = system.set_messaging_timeout(1.0);
    let Some(application) = focused_application(&system)? else {
        return Ok(None);
    };
    let _ = application.set_messaging_timeout(1.0);
    let Some(window) = focused_or_first_window(&application)? else {
        return Ok(None);
    };

    if !is_window(&window)? {
        return Ok(None);
    }

    let info = window_info(&window, space, cache.known_id(&window))?;
    cache.remember(info.id, window);

    Ok(Some(info))
}

/// Callers reject hidden or minimized windows using the `WindowInfo` returned
/// by `front_window`. Frame writes are intentionally best-effort: macOS apps
/// vary in whether they support changing size, position, or both.
pub(crate) fn set_window_rect_in(
    cache: &WindowCache,
    window_id: WindowId,
    rect: Rect,
    space: CoordinateSpace,
    screens: &[panes_platform::ScreenInfo],
) -> PlatformResult<Rect> {
    ensure_accessibility_permission()?;
    let window = cache.get(window_id).ok_or(PlatformError::NotFound(
        "macOS window is not cached; read the front window before moving it",
    ))?;

    let requested_native_rect = space.panes_rect_to_native(rect);
    let current_native_rect = native_window_rect(&window)?;
    let destination_work_area = destination_work_area(space, requested_native_rect, screens);

    // Chromium-based apps can enable this application-level accessibility
    // mode, which prevents ordinary window frame updates. Disable it around
    // the write when present and restore the previous state afterwards.
    let enhanced_ui_application = temporarily_disable_enhanced_user_interface(&window);
    let result = apply_window_frame(
        &window,
        space,
        current_native_rect,
        requested_native_rect,
        destination_work_area,
    );
    if let Some(application) = enhanced_ui_application {
        let _ = set_enhanced_user_interface(&application, true);
    }

    result
}

/// Sub-pixel differences come from float round-tripping through the panes
/// coordinate space, never from a real frame change, so treat them as equal
/// and skip the synchronous write.
fn coordinates_match(left: f64, right: f64) -> bool {
    (left - right).abs() < 0.1
}

fn apply_window_frame(
    window: &AXUIElement,
    space: CoordinateSpace,
    current: Rect,
    requested: Rect,
    destination_work_area: Rect,
) -> PlatformResult<Rect> {
    let can_resize = is_size_settable(window);
    let target = if can_resize {
        requested
    } else {
        align_constrained_rect(current, requested, destination_work_area)
    };
    let size_changed = !coordinates_match(current.size.width, target.size.width)
        || !coordinates_match(current.size.height, target.size.height);
    let position_changed = !coordinates_match(current.origin.x, target.origin.x)
        || !coordinates_match(current.origin.y, target.origin.y);
    let mut first_error = None;

    // macOS constrains an AXSize update to the display the window is currently
    // on. Resizing before and after the position update lets cross-display
    // changes succeed in Chromium and other apps that enforce that rule.
    if can_resize && size_changed {
        remember_first_error(
            &mut first_error,
            set_ax_size(
                window,
                kAXSizeAttribute,
                CGSize::new(target.size.width, target.size.height),
            ),
        );
    }
    if position_changed {
        remember_first_error(
            &mut first_error,
            set_ax_point(
                window,
                kAXPositionAttribute,
                CGPoint::new(target.origin.x, target.origin.y),
            ),
        );
    }
    if can_resize && size_changed {
        remember_first_error(
            &mut first_error,
            set_ax_size(
                window,
                kAXSizeAttribute,
                CGSize::new(target.size.width, target.size.height),
            ),
        );
    }

    let mut applied = native_window_rect(window)?;
    let aligned = align_constrained_rect(applied, requested, destination_work_area);
    if !coordinates_match(applied.origin.x, aligned.origin.x)
        || !coordinates_match(applied.origin.y, aligned.origin.y)
    {
        remember_first_error(
            &mut first_error,
            set_ax_point(
                window,
                kAXPositionAttribute,
                CGPoint::new(aligned.origin.x, aligned.origin.y),
            ),
        );
        applied = native_window_rect(window)?;
    }

    if native_rects_match(applied, current) && !native_rects_match(requested, current) {
        if let Some(error) = first_error {
            return Err(error);
        }
    }

    Ok(space.native_rect_to_panes(applied))
}

fn remember_first_error(first_error: &mut Option<PlatformError>, result: PlatformResult<()>) {
    if first_error.is_none() {
        if let Err(error) = result {
            *first_error = Some(error);
        }
    }
}

fn destination_work_area(
    space: CoordinateSpace,
    requested_native_rect: Rect,
    screens: &[panes_platform::ScreenInfo],
) -> Rect {
    let requested_panes_rect = space.native_rect_to_panes(requested_native_rect);

    screens
        .iter()
        .filter_map(|screen| {
            screen
                .work_area
                .intersection(requested_panes_rect)
                .map(|overlap| (screen.work_area, overlap.area()))
        })
        .max_by(|(_, left_area), (_, right_area)| left_area.total_cmp(right_area))
        .map(|(work_area, _)| space.panes_rect_to_native(work_area))
        .unwrap_or(requested_native_rect)
}

fn align_constrained_rect(actual: Rect, zone: Rect, work_area: Rect) -> Rect {
    Rect::new(
        aligned_axis_origin(
            actual.min_x(),
            actual.size.width,
            zone.min_x(),
            zone.size.width,
            work_area.min_x(),
            work_area.size.width,
        ),
        aligned_axis_origin(
            actual.min_y(),
            actual.size.height,
            zone.min_y(),
            zone.size.height,
            work_area.min_y(),
            work_area.size.height,
        ),
        actual.size.width,
        actual.size.height,
    )
}

fn aligned_axis_origin(
    _actual_origin: f64,
    actual_size: f64,
    zone_origin: f64,
    zone_size: f64,
    work_area_origin: f64,
    work_area_size: f64,
) -> f64 {
    let zone_max = zone_origin + zone_size;
    let work_area_max = work_area_origin + work_area_size;
    let zone_touches_start = coordinates_match(zone_origin, work_area_origin);
    let zone_touches_end = coordinates_match(zone_max, work_area_max);
    let origin =
        if coordinates_match(actual_size, zone_size) || zone_touches_start && zone_touches_end {
            zone_origin + (zone_size - actual_size) / 2.0
        } else if zone_touches_start {
            zone_origin
        } else if zone_touches_end {
            zone_max - actual_size
        } else {
            zone_origin + (zone_size - actual_size) / 2.0
        };

    if actual_size <= work_area_size {
        origin.clamp(work_area_origin, work_area_max - actual_size)
    } else {
        work_area_origin
    }
}

fn native_rects_match(left: Rect, right: Rect) -> bool {
    coordinates_match(left.origin.x, right.origin.x)
        && coordinates_match(left.origin.y, right.origin.y)
        && coordinates_match(left.size.width, right.size.width)
        && coordinates_match(left.size.height, right.size.height)
}

fn temporarily_disable_enhanced_user_interface(window: &AXUIElement) -> Option<AXUIElement> {
    let application = AXUIElement::application(element_pid(window).ok()?);
    let _ = application.set_messaging_timeout(1.0);

    if optional_custom_cf_boolean(&application, AX_ENHANCED_USER_INTERFACE_ATTRIBUTE)
        .ok()
        .flatten()
        == Some(true)
        && set_enhanced_user_interface(&application, false).is_ok()
    {
        Some(application)
    } else {
        None
    }
}

fn set_enhanced_user_interface(application: &AXUIElement, enabled: bool) -> PlatformResult<()> {
    let attribute = AXAttribute::<CFType>::new(&CFString::from_static_string(
        AX_ENHANCED_USER_INTERFACE_ATTRIBUTE,
    ));
    let value = if enabled {
        CFBoolean::true_value()
    } else {
        CFBoolean::false_value()
    };

    application
        .set_attribute(&attribute, value.as_CFType())
        .map_err(|error| map_accessibility_error("failed to set macOS enhanced UI mode", error))
}

fn focused_application(system: &AXUIElement) -> PlatformResult<Option<AXUIElement>> {
    let attribute = AXAttribute::<CFType>::new(&CFString::from_static_string(
        kAXFocusedApplicationAttribute,
    ));
    let value = match system.attribute(&attribute) {
        Ok(value) => value,
        Err(error) if optional_accessibility_error(&error) => return Ok(None),
        Err(error) => {
            return Err(map_accessibility_error(
                "failed to read focused macOS application",
                error,
            ));
        }
    };

    match value.downcast_into::<AXUIElement>() {
        Some(application) => Ok(Some(application)),
        None => Err(PlatformError::Native(
            "focused macOS application was not an accessibility element".to_owned(),
        )),
    }
}

fn focused_or_first_window(application: &AXUIElement) -> PlatformResult<Option<AXUIElement>> {
    match application.attribute(&AXAttribute::focused_window()) {
        Ok(window) if is_window(&window)? => return Ok(Some(window)),
        // Some Chromium PWAs and system windows do not expose a focused AX
        // window reliably. Their application window list is still usable.
        Ok(_) => {}
        Err(error) if optional_accessibility_error(&error) => {}
        Err(error) => {
            return Err(map_accessibility_error(
                "failed to read focused macOS window",
                error,
            ));
        }
    }

    let windows = match application.attribute(&AXAttribute::windows()) {
        Ok(windows) => windows,
        Err(error) if optional_accessibility_error(&error) => return Ok(None),
        Err(error) => {
            return Err(map_accessibility_error(
                "failed to read macOS application windows",
                error,
            ));
        }
    };

    for window in &windows {
        let window = window.clone();
        if is_window(&window)? {
            return Ok(Some(window));
        }
    }

    Ok(None)
}

fn ensure_accessibility_permission() -> PlatformResult<()> {
    if accessibility_authorization::is_trusted() {
        Ok(())
    } else {
        Err(accessibility_authorization::permission_denied())
    }
}

fn window_info(
    window: &AXUIElement,
    space: CoordinateSpace,
    known_id: Option<WindowId>,
) -> PlatformResult<WindowInfo> {
    let pid = element_pid(window)?;
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid);
    let title = optional_cf_string(window, &AXAttribute::title())?.unwrap_or_default();
    let id = known_id.unwrap_or_else(|| window_id(window));

    Ok(WindowInfo {
        id,
        app_id: app
            .as_ref()
            .and_then(|app| app.bundleIdentifier())
            .map(|bundle_id| bundle_id.to_string())
            .unwrap_or_else(|| format!("pid:{pid}")),
        title,
        rect: window_rect(window, space)?,
        is_resizable: is_size_settable(window),
        is_minimized: optional_cf_boolean(window, &AXAttribute::minimized())?.unwrap_or(false),
        is_hidden: app.as_ref().is_some_and(|app| app.isHidden()),
        is_fullscreen: optional_custom_cf_boolean(window, AX_FULL_SCREEN_ATTRIBUTE)?
            .unwrap_or(false),
    })
}

fn is_window(window: &AXUIElement) -> PlatformResult<bool> {
    if optional_cf_string(window, &AXAttribute::role())?.as_deref() != Some(kAXWindowRole) {
        return Ok(false);
    }

    let subrole = optional_cf_string(window, &AXAttribute::subrole())?;
    Ok(!subrole.as_deref().is_some_and(|subrole| {
        subrole == kAXSystemDialogSubrole
            || subrole == kAXFloatingWindowSubrole
            || subrole == kAXSystemFloatingWindowSubrole
    }))
}

fn window_rect(window: &AXUIElement, space: CoordinateSpace) -> PlatformResult<Rect> {
    Ok(space.native_rect_to_panes(native_window_rect(window)?))
}

fn native_window_rect(window: &AXUIElement) -> PlatformResult<Rect> {
    let position = ax_point(window, kAXPositionAttribute)?;
    let size = ax_size(window, kAXSizeAttribute)?;

    Ok(Rect::new(position.x, position.y, size.width, size.height))
}

fn window_id(window: &AXUIElement) -> WindowId {
    ax_pointer_window_id(window)
}

fn ax_pointer_window_id(window: &AXUIElement) -> WindowId {
    WindowId(window.as_concrete_TypeRef() as usize as u64)
}

fn element_pid(element: &AXUIElement) -> PlatformResult<pid_t> {
    let mut pid = 0;
    // SAFETY: The AXUIElement reference is owned by the accessibility wrapper and the pid out
    // pointer is valid for this call.
    let error = unsafe { AXUIElementGetPid(element.as_concrete_TypeRef(), &mut pid) };
    if error == accessibility_sys::kAXErrorSuccess {
        Ok(pid)
    } else {
        Err(ax_error("failed to read macOS window process id", error))
    }
}

fn optional_cf_string(
    element: &AXUIElement,
    attribute: &AXAttribute<core_foundation::string::CFString>,
) -> PlatformResult<Option<String>> {
    Ok(optional_attribute(element, attribute)?.map(|value| value.to_string()))
}

fn optional_cf_boolean(
    element: &AXUIElement,
    attribute: &AXAttribute<CFBoolean>,
) -> PlatformResult<Option<bool>> {
    Ok(optional_attribute(element, attribute)?.map(bool::from))
}

fn optional_custom_cf_boolean(
    element: &AXUIElement,
    name: &'static str,
) -> PlatformResult<Option<bool>> {
    let attribute = AXAttribute::<CFType>::new(&CFString::from_static_string(name));
    match element.attribute(&attribute) {
        Ok(value) => Ok(value.downcast::<CFBoolean>().map(bool::from)),
        Err(error) if optional_accessibility_error(&error) => Ok(None),
        Err(error) => Err(map_accessibility_error(
            "failed to read macOS accessibility attribute",
            error,
        )),
    }
}

fn optional_attribute<T: TCFType>(
    element: &AXUIElement,
    attribute: &AXAttribute<T>,
) -> PlatformResult<Option<T>> {
    match element.attribute(attribute) {
        Ok(value) => Ok(Some(value)),
        Err(error) if optional_accessibility_error(&error) => Ok(None),
        Err(error) => Err(map_accessibility_error(
            "failed to read macOS accessibility attribute",
            error,
        )),
    }
}

fn is_size_settable(window: &AXUIElement) -> bool {
    let attribute = AXAttribute::<CFType>::new(&CFString::from_static_string(kAXSizeAttribute));
    window.is_settable(&attribute).unwrap_or(false)
}

fn ax_point(element: &AXUIElement, name: &'static str) -> PlatformResult<CGPoint> {
    let value = ax_value(element, name, kAXValueTypeCGPoint)?;
    let ax_value = value.as_CFTypeRef() as AXValueRef;
    let mut point = CGPoint::new(0.0, 0.0);

    // SAFETY: The AXValue type has been checked as CGPoint, and the output pointer targets a
    // valid CGPoint on the stack.
    if unsafe {
        AXValueGetValue(
            ax_value,
            kAXValueTypeCGPoint,
            (&mut point as *mut CGPoint).cast::<c_void>(),
        )
    } {
        Ok(point)
    } else {
        Err(PlatformError::Native(format!(
            "failed to decode {name} as CGPoint"
        )))
    }
}

fn ax_size(element: &AXUIElement, name: &'static str) -> PlatformResult<CGSize> {
    let value = ax_value(element, name, kAXValueTypeCGSize)?;
    let ax_value = value.as_CFTypeRef() as AXValueRef;
    let mut size = CGSize::new(0.0, 0.0);

    // SAFETY: The AXValue type has been checked as CGSize, and the output pointer targets a valid
    // CGSize on the stack.
    if unsafe {
        AXValueGetValue(
            ax_value,
            kAXValueTypeCGSize,
            (&mut size as *mut CGSize).cast::<c_void>(),
        )
    } {
        Ok(size)
    } else {
        Err(PlatformError::Native(format!(
            "failed to decode {name} as CGSize"
        )))
    }
}

fn set_ax_point(element: &AXUIElement, name: &'static str, point: CGPoint) -> PlatformResult<()> {
    set_ax_value(
        element,
        name,
        kAXValueTypeCGPoint,
        (&point as *const CGPoint).cast::<c_void>(),
    )
}

fn set_ax_size(element: &AXUIElement, name: &'static str, size: CGSize) -> PlatformResult<()> {
    set_ax_value(
        element,
        name,
        kAXValueTypeCGSize,
        (&size as *const CGSize).cast::<c_void>(),
    )
}

fn set_ax_value(
    element: &AXUIElement,
    name: &'static str,
    value_type: u32,
    value: *const c_void,
) -> PlatformResult<()> {
    let attribute = CFString::from_static_string(name);
    // SAFETY: The caller passes a pointer to the CoreGraphics type that matches value_type.
    let ax_value = unsafe { AXValueCreate(value_type, value) };
    if ax_value.is_null() {
        return Err(PlatformError::Native(format!(
            "failed to create AXValue for {name}"
        )));
    }

    // SAFETY: AXValueCreate follows the Create Rule, so CFType takes ownership of the retain.
    let ax_value = unsafe { CFType::wrap_under_create_rule(ax_value as CFTypeRef) };
    // SAFETY: The element and attribute are valid CoreFoundation references, and ax_value is an
    // AXValue created for this call.
    let error = unsafe {
        AXUIElementSetAttributeValue(
            element.as_concrete_TypeRef(),
            attribute.as_concrete_TypeRef(),
            ax_value.as_CFTypeRef(),
        )
    };

    if error == accessibility_sys::kAXErrorSuccess {
        Ok(())
    } else {
        Err(ax_error("failed to set macOS window frame", error))
    }
}

fn ax_value(
    element: &AXUIElement,
    name: &'static str,
    expected_type: u32,
) -> PlatformResult<CFType> {
    let attribute = AXAttribute::<CFType>::new(&CFString::from_static_string(name));
    let value = element
        .attribute(&attribute)
        .map_err(|error| map_accessibility_error("failed to read macOS AXValue", error))?;

    // SAFETY: AXValueGetTypeID is a pure CoreFoundation type-id lookup.
    if value.type_of() != unsafe { AXValueGetTypeID() } {
        return Err(PlatformError::Native(format!(
            "{name} was not returned as an AXValue"
        )));
    }

    let ax_value = value.as_CFTypeRef() as AXValueRef;
    // SAFETY: The CF object has been checked as an AXValue.
    let actual_type = unsafe { AXValueGetType(ax_value) };
    if actual_type != expected_type {
        return Err(PlatformError::Native(format!(
            "{name} returned AXValue type {actual_type}, expected {expected_type}"
        )));
    }

    Ok(value)
}

fn optional_accessibility_error(error: &AccessibilityError) -> bool {
    matches!(error, AccessibilityError::NotFound)
        || matches!(error, AccessibilityError::Ax(error) if optional_ax_error(*error))
}

fn optional_ax_error(error: AXError) -> bool {
    error == kAXErrorAttributeUnsupported || error == kAXErrorNoValue
}

fn map_accessibility_error(context: &'static str, error: AccessibilityError) -> PlatformError {
    match error {
        AccessibilityError::NotFound => PlatformError::NotFound(context),
        AccessibilityError::Ax(error) => ax_error(context, error),
        error => PlatformError::Native(format!("{context}: {error}")),
    }
}

fn ax_error(context: &'static str, error: AXError) -> PlatformError {
    if error == kAXErrorAPIDisabled {
        accessibility_authorization::permission_denied()
    } else {
        PlatformError::Native(format!("{context}: {}", error_string(error)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constrained_windows_anchor_to_the_matching_half_edge() {
        let work_area = Rect::new(0.0, 0.0, 1000.0, 800.0);
        let actual = Rect::new(200.0, 100.0, 600.0, 800.0);

        let left = align_constrained_rect(actual, Rect::new(0.0, 0.0, 500.0, 800.0), work_area);
        let right = align_constrained_rect(actual, Rect::new(500.0, 0.0, 500.0, 800.0), work_area);

        assert_eq!(left, Rect::new(0.0, 0.0, 600.0, 800.0));
        assert_eq!(right, Rect::new(400.0, 0.0, 600.0, 800.0));
    }

    #[test]
    fn fixed_size_move_uses_the_requested_position() {
        let work_area = Rect::new(0.0, 0.0, 1000.0, 800.0);
        let actual = Rect::new(100.0, 100.0, 200.0, 100.0);
        let target = Rect::new(800.0, 100.0, 200.0, 100.0);

        assert_eq!(
            align_constrained_rect(actual, target, work_area),
            Rect::new(800.0, 100.0, 200.0, 100.0)
        );
    }

    #[test]
    fn oversized_constrained_windows_anchor_at_the_destination_work_area_start() {
        let work_area = Rect::new(0.0, 0.0, 1000.0, 800.0);
        let actual = Rect::new(-500.0, 0.0, 1200.0, 100.0);

        assert_eq!(
            align_constrained_rect(actual, Rect::new(0.0, 0.0, 500.0, 800.0), work_area),
            Rect::new(0.0, 350.0, 1200.0, 100.0)
        );
    }
}
