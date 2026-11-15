use wry::{PermissionKind, PermissionResponse};

/// Keep geolocation out of the Lite binary while leaving the webview's other
/// permission requests on their normal platform path.
pub(crate) fn response(kind: PermissionKind) -> PermissionResponse {
    if kind == PermissionKind::Geolocation {
        #[cfg(feature = "geolocation")]
        {
            return PermissionResponse::Default;
        }

        #[cfg(not(feature = "geolocation"))]
        {
            return PermissionResponse::Deny;
        }
    }

    PermissionResponse::Default
}
