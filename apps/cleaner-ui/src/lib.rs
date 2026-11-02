mod app;
mod window_close;
pub use app::App;
pub use app::configured_app;
pub use window_close::request_window_close;
pub mod commands;
mod components;
pub mod controller;
mod pages;
pub mod routes;
mod runtime;
pub use runtime::{ScanProgress, ScanReport};
mod screens;
pub mod space_analysis;
pub mod state;
pub mod theme;

use nsh_core::prelude::*;

#[native_module("rusten/native")]
fn native(_registry: &mut Registry) {}
