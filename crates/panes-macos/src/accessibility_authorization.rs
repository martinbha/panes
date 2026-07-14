use std::process::Command;

use accessibility_sys::{
    AXIsProcessTrusted, AXIsProcessTrustedWithOptions, kAXTrustedCheckOptionPrompt,
};
use core_foundation::{
    base::TCFType, boolean::CFBoolean, dictionary::CFDictionary, string::CFString,
};
use panes_platform::{PlatformError, PlatformResult};

pub(crate) const PERMISSION_GUIDANCE: &str = "Grant Accessibility permission to Panes in System Settings > Privacy & Security > Accessibility";

const ACCESSIBILITY_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility";

pub(crate) fn is_trusted() -> bool {
    // SAFETY: AXIsProcessTrusted has no arguments and only reads the current process trust state.
    unsafe { AXIsProcessTrusted() }
}

pub(crate) fn prompt() -> bool {
    let prompt_key = unsafe { CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt) };
    let options = CFDictionary::from_CFType_pairs(&[(prompt_key, CFBoolean::true_value())]);

    // SAFETY: The options dictionary contains the documented prompt key with a CFBoolean value.
    unsafe { AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) }
}

pub(crate) fn open_settings() -> PlatformResult<()> {
    let status = Command::new("/usr/bin/open")
        .arg(ACCESSIBILITY_SETTINGS_URL)
        .status()
        .map_err(|error| {
            PlatformError::Native(format!(
                "failed to open macOS Accessibility settings: {error}"
            ))
        })?;

    if status.success() {
        Ok(())
    } else {
        Err(PlatformError::Native(format!(
            "failed to open macOS Accessibility settings: open exited with {status}"
        )))
    }
}

pub(crate) const fn permission_denied() -> PlatformError {
    PlatformError::PermissionDenied(PERMISSION_GUIDANCE)
}
