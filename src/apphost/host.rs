use crate::apphost::AppHost;
use crate::buffer::CellBuffer;
use crate::kittygfx::Placement;
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
}

impl Default for LocalAppHost {
    fn default() -> Self {
        Self::new()
    }
}

impl AppHost for LocalAppHost {
    fn spawn(
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

    fn input(&mut self, id: AppId, bytes: &[u8]) {
        if let Some(app) = self.apps.get_mut(&id) {
            app.write_input(bytes);
        }
    }

    fn resize(&mut self, id: AppId, cols: i32, rows: i32) {
        if let Some(app) = self.apps.get_mut(&id) {
            app.resize(cols, rows);
        }
    }

    fn scroll(&mut self, id: AppId, lines: i32) {
        if let Some(app) = self.apps.get_mut(&id) {
            app.scroll(lines);
        }
    }

    fn kill(&mut self, id: AppId) {
        if let Some(app) = self.apps.get_mut(&id) {
            app.kill();
        }
    }

    fn is_alive(&mut self, id: AppId) -> bool {
        self.apps.get_mut(&id).map(|a| a.is_alive()).unwrap_or(false)
    }

    fn take_bells(&mut self, id: AppId) -> u32 {
        self.apps.get(&id).map(|a| a.take_bells()).unwrap_or(0)
    }

    fn take_clipboard(&mut self, id: AppId) -> Option<String> {
        self.apps.get(&id).and_then(|a| a.take_clipboard())
    }

    fn snapshot(&self, id: AppId) -> Option<CellBuffer> {
        self.apps.get(&id).map(|a| a.snapshot())
    }

    fn placements(&self, id: AppId) -> Vec<Placement> {
        self.apps
            .get(&id)
            .map(|a| a.graphics().placements.clone())
            .unwrap_or_default()
    }

    fn image_png(&self, id: AppId, image_id: u32) -> Option<Vec<u8>> {
        self.apps.get(&id).and_then(|a| a.graphics().png(image_id).map(|b| b.to_vec()))
    }

    fn list(&self) -> Vec<AppId> {
        self.apps.keys().copied().collect()
    }

    fn set_meta(&mut self, id: AppId, meta: Vec<u8>) {
        self.meta.insert(id, meta);
    }

    fn meta(&self, id: AppId) -> Option<Vec<u8>> {
        self.meta.get(&id).cloned()
    }

    fn remove(&mut self, id: AppId) {
        self.apps.remove(&id);
        self.meta.remove(&id);
    }

    fn mouse_mode(&self, id: AppId) -> crate::mouse::AppMouse {
        self.apps.get(&id).map(|a| a.mouse_mode()).unwrap_or_default()
    }

    fn inject_test_image(&self, id: AppId, png: &[u8]) {
        if let Some(app) = self.apps.get(&id) {
            let mut g = app.graphics();
            g.insert_image_for_test(1, png.to_vec());
            g.push_placement_for_test(Placement {
                image_id: 1,
                col: 0,
                row: 0,
                cols: 2,
                rows: 1,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apphost::AppHost;

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

    #[test]
    fn snapshot_unknown_is_none() {
        let host = LocalAppHost::new();
        assert!(host.snapshot(AppId(999)).is_none());
    }

    #[test]
    fn is_alive_tracks_child_exit() {
        let mut host = LocalAppHost::new();
        let id = host.spawn("true", &[], None, 80, 24).unwrap();
        let mut alive_after = true;
        for _ in 0..50 {
            if !host.is_alive(id) {
                alive_after = false;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!alive_after, "child `true` should be reaped as not-alive");
    }

    #[test]
    fn snapshot_after_spawn_has_requested_dimensions() {
        let mut host = LocalAppHost::new();
        let id = host.spawn("cat", &[], None, 80, 24).unwrap();
        let snap = host.snapshot(id).expect("snapshot");
        assert_eq!(snap.width(), 80);
        assert_eq!(snap.height(), 24);
        host.kill(id);
    }

    #[test]
    fn meta_round_trips() {
        let mut host = LocalAppHost::new();
        let id = host.spawn("true", &[], None, 80, 24).unwrap();
        assert!(host.meta(id).is_none());
        host.set_meta(id, vec![1, 2, 3]);
        assert_eq!(host.meta(id), Some(vec![1, 2, 3]));
        host.set_meta(id, vec![9]);
        assert_eq!(host.meta(id), Some(vec![9]));
    }

    #[test]
    fn remove_clears_meta() {
        let mut host = LocalAppHost::new();
        let id = host.spawn("true", &[], None, 80, 24).unwrap();
        host.set_meta(id, vec![1]);
        host.remove(id);
        assert!(host.meta(id).is_none());
    }
}
