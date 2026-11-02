#![cfg_attr(windows, windows_subsystem = "windows")]

use nsh_platform_windows as platform;

fn main() -> impl std::process::Termination {
    platform::set_ui_sets(vec![
        nsh_base::registry(),
        cleaner_ui::__nsh_native_module(),
    ]);
    let app_id = "com.rusten.cleaner";
    let exit = platform::run_app(
        platform::AppSource::Rust(platform::Root::of::<cleaner_ui::App>().app_id(app_id)),
        platform::AppOptions {
            app_id: app_id.into(),
            title: "Rusten".into(),
            size: (1180, 760),
            appearance: platform::Appearance::Light,
            close_handler: Some(cleaner_ui::request_window_close),
            ..Default::default()
        },
    );
    if exit != std::process::ExitCode::SUCCESS {
        log::error!("Rusten NSH initialization or application loop failed");
        platform::debug_output("Rusten NSH initialization or application loop failed");
    }
    exit
}
