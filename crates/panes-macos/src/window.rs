use std::{cell::RefCell, collections::HashMap, ffi::c_void};

use accessibility::{AXAttribute, AXUIElement, Error as AccessibilityError};
use accessibility_sys::{
    AXError, AXIsProcessTrustedWithOptions, AXUIElementGetPid, AXValueGetType, AXValueGetTypeID,
    AXValueGetValue, AXValueRef, error_string, kAXErrorAPIDisabled, kAXErrorAttributeUnsupported,
    kAXErrorNoValue, kAXPositionAttribute, kAXSizeAttribute, kAXStandardWindowSubrole,
    kAXTrustedCheckOptionPrompt, kAXValueTypeCGPoint, kAXValueTypeCGSize, kAXWindowRole, pid_t,
};
use core_foundation::{
    base::{CFType, TCFType},
    boolean::CFBoolean,
    dictionary::CFDictionary,
    string::CFString,
};
use core_graphics::geometry::{CGPoint, CGSize};
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
}

pub(crate) fn front_window(cache: &WindowCache) -> PlatformResult<Option<WindowInfo>> {
    ensure_accessibility_permission()?;

    let system = AXUIElement::system_wide();
    let _ = system.set_messaging_timeout(1.0);
    let window = match system.attribute(&AXAttribute::focused_window()) {
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
    let id = window_id(window);

    Ok(WindowInfo {
        id,
        app_id: app
            .as_ref()
            .and_then(|app| app.bundleIdentifier())
            .map(|bundle_id| bundle_id.to_string())
            .unwrap_or_else(|| format!("pid:{pid}")),
        title: optional_cf_string(window, &AXAttribute::title())?.unwrap_or_default(),
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

fn window_id(window: &AXUIElement) -> WindowId {
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
