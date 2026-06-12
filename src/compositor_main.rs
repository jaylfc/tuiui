//! tuiui compositor entry point — runs the Wayland compositor backend.
//!
//!   tuiui-compositor

use std::io;

#[cfg(feature = "wayland-compositor")]
fn run() -> io::Result<()> {
    tuiui::run_compositor()
}

#[cfg(not(feature = "wayland-compositor"))]
fn run() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "tuiui-compositor: requires the 'wayland-compositor' feature (install Linux release or build with --features wayland-compositor)",
    ))
}

fn main() -> io::Result<()> {
    run()
}