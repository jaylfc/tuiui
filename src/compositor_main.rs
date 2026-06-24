//! tuiui compositor entry point — Wayland compositor backend (stub implementation).
//!
//!   tuiui-compositor

use std::io;

fn run() -> io::Result<()> {
    tuiui::run_compositor()
}

fn main() -> io::Result<()> {
    run()
}
