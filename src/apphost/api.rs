//! The `AppHost` trait — the stable seam between the frontend (`SessionCore`)
//! and whatever owns the running apps. `LocalAppHost` implements it in-process;
//! Phase 2b adds a `RemoteAppHost` that implements the identical API over a
//! socket. Every method returns owned/copyable data (no lock guards) so it can
//! cross a process boundary unchanged.

use super::AppId;
use crate::buffer::CellBuffer;
use crate::kittygfx::Placement;
use std::path::Path;

pub trait AppHost: Send {
    /// Spawn a child in a PTY and return its handle. Propagates spawn failure.
    fn spawn(
        &mut self,
        cmd: &str,
        args: &[String],
        cwd: Option<&Path>,
        cols: i32,
        rows: i32,
    ) -> std::io::Result<AppId>;

    /// Forward bytes to the app's PTY (no-op if unknown).
    fn input(&mut self, id: AppId, bytes: &[u8]);

    /// Resize the app's PTY/terminal (no-op if unknown).
    fn resize(&mut self, id: AppId, cols: i32, rows: i32);

    /// Scroll the app's scrollback view by `lines` (+ = back into history).
    /// Default no-op for hosts/fakes without scrollback.
    fn scroll(&mut self, id: AppId, lines: i32) { let _ = (id, lines); }

    /// The wire-protocol version of the host actually serving the apps.
    /// In-process hosts (and fakes) are always current; `RemoteAppHost`
    /// reports what the running apphost declared (0 = predates the field).
    fn proto_version(&self) -> u32 { crate::apphost::proto::PROTO_VERSION }

    /// Terminate the child (no-op if unknown).
    fn kill(&mut self, id: AppId);

    /// Whether the child is still running. Unknown ids report `false`.
    fn is_alive(&mut self, id: AppId) -> bool;

    /// Current terminal grid for the app, or `None` if unknown.
    fn snapshot(&self, id: AppId) -> Option<CellBuffer>;

    /// Image placements the app currently declares (cell coords on its grid).
    fn placements(&self, id: AppId) -> Vec<Placement>;

    /// PNG bytes for one of the app's transmitted images, if present.
    fn image_png(&self, id: AppId, image_id: u32) -> Option<Vec<u8>>;

    /// All currently-hosted app handles (order unspecified).
    fn list(&self) -> Vec<AppId>;

    /// Store opaque frontend metadata (window geometry/title/z) for restore.
    fn set_meta(&mut self, id: AppId, meta: Vec<u8>);

    /// The last metadata stored for the app, if any (owned copy).
    fn meta(&self, id: AppId) -> Option<Vec<u8>>;

    /// Drop the host's tracking of the app (does not kill).
    fn remove(&mut self, id: AppId);

    /// Bell rings since the last call (drained; default none for test fakes).
    fn take_bells(&mut self, _id: AppId) -> u32 { 0 }

    /// The app's latest OSC-52 clipboard store, if any (drained; default none).
    fn take_clipboard(&mut self, _id: AppId) -> Option<String> { None }

    /// The app's current terminal mouse mode (default = no mouse).
    fn mouse_mode(&self, id: AppId) -> crate::mouse::AppMouse { let _ = id; crate::mouse::AppMouse::default() }

    /// Stop the underlying app host process, if any (default no-op for the
    /// in-process host). The frontend calls this on full shutdown.
    fn shutdown_host(&mut self) {}

    /// Test hook: inject a placement + image into the app's graphics state.
    /// Default no-op; `LocalAppHost` overrides it. Used only by integration tests.
    #[doc(hidden)]
    fn inject_test_image(&self, _id: AppId, _png: &[u8]) {}
}
