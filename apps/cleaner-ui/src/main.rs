#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod commands;
mod components;
mod metrics;
mod routes;
mod runtime;
mod screens;
mod state;

fn main() {
    app::launch();
}
