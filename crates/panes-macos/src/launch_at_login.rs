use objc2_service_management::{SMAppService, SMAppServiceStatus};
use panes_platform::{LaunchAtLoginStatus, PlatformError, PlatformResult};

pub(crate) fn status() -> PlatformResult<LaunchAtLoginStatus> {
    // SAFETY: SMAppService's main-app singleton and status getter do not
    // retain caller-owned pointers.
    let service = unsafe { SMAppService::mainAppService() };
    Ok(status_from_native(unsafe { service.status() }))
}

pub(crate) fn set_enabled(enabled: bool) -> PlatformResult<()> {
    // SAFETY: SMAppService owns the returned main-app service object.
    let service = unsafe { SMAppService::mainAppService() };
    let status = status_from_native(unsafe { service.status() });
    if !status.is_available() {
        return Err(PlatformError::Unsupported(
            "macOS launch at login requires a signed Panes.app bundle",
        ));
    }
    if enabled && status.is_configured() || !enabled && !status.has_registration() {
        return Ok(());
    }

    let result = if enabled {
        // SAFETY: the receiver is the retained main-app service object.
        unsafe { service.registerAndReturnError() }
    } else {
        // SAFETY: the receiver is the retained main-app service object.
        unsafe { service.unregisterAndReturnError() }
    };
    result.map_err(|error| {
        PlatformError::Native(format!(
            "failed to {} macOS launch at login: {error}",
            if enabled { "enable" } else { "disable" }
        ))
    })
}

fn status_from_native(status: SMAppServiceStatus) -> LaunchAtLoginStatus {
    match status {
        SMAppServiceStatus::NotRegistered => LaunchAtLoginStatus::Disabled,
        SMAppServiceStatus::Enabled => LaunchAtLoginStatus::Enabled,
        SMAppServiceStatus::RequiresApproval => LaunchAtLoginStatus::RequiresApproval,
        SMAppServiceStatus::NotFound => LaunchAtLoginStatus::Unavailable,
        _ => LaunchAtLoginStatus::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_documented_service_status() {
        assert_eq!(
            status_from_native(SMAppServiceStatus::NotRegistered),
            LaunchAtLoginStatus::Disabled
        );
        assert_eq!(
            status_from_native(SMAppServiceStatus::Enabled),
            LaunchAtLoginStatus::Enabled
        );
        assert_eq!(
            status_from_native(SMAppServiceStatus::RequiresApproval),
            LaunchAtLoginStatus::RequiresApproval
        );
        assert_eq!(
            status_from_native(SMAppServiceStatus::NotFound),
            LaunchAtLoginStatus::Unavailable
        );
    }
}
