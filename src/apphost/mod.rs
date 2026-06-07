//! In-process application host — owns every PTY-backed [`AppInstance`] behind a
//! stable, `AppId`-addressed API.
//!
//! This is the seam the apphost/frontend split is built on (see
//! `docs/superpowers/specs/2026-06-07-apphost-frontend-split-design.md`). In
//! Phase 1 the host lives in the same process as the frontend; a later phase
//! moves an identical API behind a socket without the frontend noticing.

mod api;
mod host;
pub mod proto;

pub use api::AppHost;
pub use host::{AppId, LocalAppHost};
