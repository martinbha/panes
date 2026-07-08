use std::{cell::RefCell, collections::HashMap, ffi::c_void};

use accessibility::{AXAttribute, AXUIElement, Error as AccessibilityError};
use accessibility_sys::{
    AXError, AXIsProcessTrustedWithOptions, AXUIElementGetPid, AXUIElementSetAttributeValue,
    AXValueCreate, AXValueGetType, AXValueGetTypeID, AXValueGetValue, AXValueRef, error_string,
    kAXErrorAPIDisabled, kAXErrorAttributeUnsupported, kAXErrorNoValue,
    kAXFocusedApplicationAttribute, kAXPositionAttribute, kAXSizeAttribute,
    kAXStandardWindowSubrole, kAXTrustedCheckOptionPrompt, kAXValueTypeCGPoint, kAXValueTypeCGSize,
    kAXWindowRole, pid_t,
};
use core_foundation::{
    array::CFArray,
    base::{CFType, CFTypeRef, TCFType},
    boolean::CFBoolean,
    dictionary::CFDictionary,
    number::CFNumber,
    string::{CFString, CFStringRef},
};
use core_graphics::{
    geometry::{CGPoint, CGSize},
    window::{
        self as cg_window, CGWindowID, kCGNullWindowID, kCGWindowLayer,
        kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly, kCGWindowName,
        kCGWindowNumber, kCGWindowOwnerPID,
    },
};
use objc2_app_kit::NSRunningApplication;
use panes_core::{Rect, WindowId};
use panes_platform::{PlatformError, PlatformResult, WindowInfo};

use crate::screen;

const ACCESSIBILITY_PERMISSION_ERROR: &str = "Enable Accessibility access for panes in System Settings > Privacy & Security > Accessibility, then restart panes";

#[derive(Default)]
pub(crate) struct WindowCache {
    windows: RefCell<HashMap<WindowId, AXUIElement>>,
}

impl WindowCache {
    pub(crate) fn remember(&self, id: WindowId, window: AXUIElement) {
        self.windows.borrow_mut().insert(id, window);
    }

    pub(crate) fn get(&self, id: WindowId) -> Option<AXUIElement> {
        self.windows.borrow().get(&id).cloned()
    }
}

pub(crate) fn front_window(cache: &WindowCache) -> PlatformResult<Option<WindowInfo>> {
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
    let window = match application.attribute(&AXAttribute::focused_window()) {
        Ok(window) => window,
        Err(error) if optional_accessibility_error(&error) => return Ok(None),
        Err(error) => {
            return Err(map_accessibility_error(
                "failed to read focused macOS window",
                error,
            ));
        }
    };

    if !is_standard_window(&window)? {
        return Ok(None);
    }

    let info = window_info(&window)?;
    cache.remember(info.id, window);

    Ok(Some(info))
}

pub(crate) fn set_window_rect(
    cache: &WindowCache,
    window_id: WindowId,
    rect: Rect,
) -> PlatformResult<Rect> {
    ensure_accessibility_permission()?;
    let window = cache.get(window_id).ok_or(PlatformError::NotFound(
        "macOS window is not cached; read the front window before moving it",
    ))?;

    if optional_cf_boolean(&window, &AXAttribute::minimized())?.unwrap_or(false) {
        return Err(PlatformError::Unsupported(
            "minimized macOS windows cannot be moved",
        ));
    }

    if !is_size_settable(&window) {
        return Err(PlatformError::Unsupported(
            "macOS window does not allow resizing",
        ));
    }

    let native_rect = screen::coordinate_space()?.panes_rect_to_native(rect);
    let size = CGSize::new(native_rect.size.width, native_rect.size.height);
    let position = CGPoint::new(native_rect.origin.x, native_rect.origin.y);

    // Apply size, then position, then size again. macOS clamps a resize whose
    // edges would overflow the screen from the window's *current* origin, so a
    // lone size-then-move can leave a dimension clamped to the old position.
    // Writing the size a second time — after the window sits at the target
    // origin, where the full size fits — lets the requested frame stick. The
    // final `window_rect` re-read still reports whatever actually held, so any
    // legitimate app constraint (min/max sizes, Terminal's character grid)
    // remains truthful in history. Mirrors Rectangle's `AccessibilityElement.setFrame`.
    set_ax_size(&window, kAXSizeAttribute, size)?;
    set_ax_point(&window, kAXPositionAttribute, position)?;
    set_ax_size(&window, kAXSizeAttribute, size)?;

    window_rect(&window)
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

fn ensure_accessibility_permission() -> PlatformResult<()> {
    let prompt_key = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
    let options = CFDictionary::from_CFType_pairs(&[(prompt_key, CFBoolean::true_value())]);

    // SAFETY: The options dictionary contains the documented prompt key with a CFBoolean value.
    if unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) } {
        Ok(())
    } else {
        Err(PlatformError::PermissionDenied(
            ACCESSIBILITY_PERMISSION_ERROR,
        ))
    }
}

fn window_info(window: &AXUIElement) -> PlatformResult<WindowInfo> {
    let pid = element_pid(window)?;
    let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid);
    let title = optional_cf_string(window, &AXAttribute::title())?.unwrap_or_default();
    let id = window_id(window, pid, &title);

    Ok(WindowInfo {
        id,
        app_id: app
            .as_ref()
            .and_then(|app| app.bundleIdentifier())
            .map(|bundle_id| bundle_id.to_string())
            .unwrap_or_else(|| format!("pid:{pid}")),
        title,
        rect: window_rect(window)?,
        is_resizable: is_size_settable(window),
        is_minimized: optional_cf_boolean(window, &AXAttribute::minimized())?.unwrap_or(false),
        is_hidden: app.as_ref().is_some_and(|app| app.isHidden()),
    })
}

fn is_standard_window(window: &AXUIElement) -> PlatformResult<bool> {
    if optional_cf_string(window, &AXAttribute::role())?.as_deref() != Some(kAXWindowRole) {
        return Ok(false);
    }

    if let Some(subrole) = optional_cf_string(window, &AXAttribute::subrole())?
        && subrole != kAXStandardWindowSubrole
    {
        return Ok(false);
    }

    Ok(true)
}

fn window_rect(window: &AXUIElement) -> PlatformResult<Rect> {
    let position = ax_point(window, kAXPositionAttribute)?;
    let size = ax_size(window, kAXSizeAttribute)?;
    let native_rect = Rect::new(position.x, position.y, size.width, size.height);

    Ok(screen::coordinate_space()?.native_rect_to_panes(native_rect))
}

fn window_id(window: &AXUIElement, pid: pid_t, title: &str) -> WindowId {
    cg_window_id(pid, title)
        .map(|id| WindowId(u64::from(id)))
        .unwrap_or_else(|| ax_pointer_window_id(window))
}

fn ax_pointer_window_id(window: &AXUIElement) -> WindowId {
    WindowId(window.as_concrete_TypeRef() as usize as u64)
}

fn cg_window_id(pid: pid_t, title: &str) -> Option<CGWindowID> {
    let options = kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements;
    let raw_windows = cg_window::copy_window_info(options, kCGNullWindowID)?;
    // SAFETY: CGWindowListCopyWindowInfo returns an array of dictionaries keyed by CFString with
    // CoreFoundation property-list values.
    let windows: CFArray<CFDictionary<CFString, CFType>> =
        unsafe { CFArray::wrap_under_get_rule(raw_windows.as_concrete_TypeRef()) };
    let mut first_pid_window = None;

    for window in &windows {
        if cg_i64(&window, unsafe { kCGWindowOwnerPID }) != Some(i64::from(pid)) {
            continue;
        }

        if cg_i64(&window, unsafe { kCGWindowLayer }).unwrap_or_default() != 0 {
            continue;
        }

        let Some(id) =
            cg_i64(&window, unsafe { kCGWindowNumber }).and_then(|id| id.try_into().ok())
        else {
            continue;
        };
        if first_pid_window.is_none() {
            first_pid_window = Some(id);
        }

        if !title.is_empty()
            && cg_string(&window, unsafe { kCGWindowName }).as_deref() == Some(title)
        {
            return Some(id);
        }
    }

    first_pid_window
}

fn cg_i64(window: &CFDictionary<CFString, CFType>, key: CFStringRef) -> Option<i64> {
    window
        .find(cf_string_key(key))?
        .downcast::<CFNumber>()?
        .to_i64()
}

fn cg_string(window: &CFDictionary<CFString, CFType>, key: CFStringRef) -> Option<String> {
    window
        .find(cf_string_key(key))?
        .downcast::<CFString>()
        .map(|value| value.to_string())
}

fn cf_string_key(key: CFStringRef) -> CFString {
    // SAFETY: CoreGraphics exposes these dictionary keys as process-lifetime CFString constants.
    unsafe { CFString::wrap_under_get_rule(key) }
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
        PlatformError::PermissionDenied(ACCESSIBILITY_PERMISSION_ERROR)
    } else {
        PlatformError::Native(format!("{context}: {}", error_string(error)))
    }
}
