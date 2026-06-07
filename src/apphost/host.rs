use crate::ptyhost::AppInstance;
use std::collections::HashMap;
use std::path::Path;

/// Opaque handle to a hosted application. Stable for the app's lifetime; the
/// frontend stores it in `WinContent::App` and uses it for every host call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AppId(pub u64);

/// Owns the live [`AppInstance`] map plus an opaque per-app metadata blob.
///
/// The metadata is never interpreted here — it is window state the frontend
/// stashes so a future restarted frontend can rebuild its windows (Phase 3).
pub struct LocalAppHost {
    apps: HashMap<AppId, AppInstance>,
    meta: HashMap<AppId, Vec<u8>>,
    next: u64,
}

impl LocalAppHost {
    pub fn new() -> Self {
        LocalAppHost { apps: HashMap::new(), meta: HashMap::new(), next: 1 }
    }

    /// Spawn a child in a PTY and return its handle. Propagates spawn failure.
    pub fn spawn(
        &mut self,
        cmd: &str,
        args: &[String],
        cwd: Option<&Path>,
        cols: i32,
        rows: i32,
    ) -> std::io::Result<AppId> {
        // AppInstance::spawn signature: (cmd, args, cols, rows, cwd)
        let app = AppInstance::spawn(cmd, args, cols, rows, cwd)?;
        let id = AppId(self.next);
        self.next += 1;
        self.apps.insert(id, app);
        Ok(id)
    }

    /// All currently-hosted app handles (order unspecified).
    pub fn list(&self) -> Vec<AppId> {
        self.apps.keys().copied().collect()
    }

    /// Drop the app instance and any metadata. Does not kill — call `kill`
    /// first if the child should be terminated.
    pub fn remove(&mut self, id: AppId) {
        self.apps.remove(&id);
        self.meta.remove(&id);
    }
}

impl Default for LocalAppHost {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_then_list_then_remove() {
        let mut host = LocalAppHost::new();
        let id = host
            .spawn("true", &[], None, 80, 24)
            .expect("spawn true");
        assert_eq!(host.list(), vec![id]);
        host.remove(id);
        assert!(host.list().is_empty());
    }

    #[test]
    fn ids_are_unique_and_increasing() {
        let mut host = LocalAppHost::new();
        let a = host.spawn("true", &[], None, 80, 24).unwrap();
        let b = host.spawn("true", &[], None, 80, 24).unwrap();
        assert_ne!(a, b);
    }
}
