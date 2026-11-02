use std::{cell::RefCell, rc::Rc, time::Duration};

use nsh_core::prelude::*;

use crate::{
    commands::{AnalysisCommand, UiCommand},
    controller::{UiController, UiSnapshot},
    pages,
    routes::ViewId,
    theme,
};

#[component]
pub fn App() -> impl IntoElement {
    shell(UiController::new())
}

pub fn configured_app(config: crate::controller::ControllerConfig) -> impl IntoElement {
    shell(UiController::with_config(config))
}

fn shell(mut controller: UiController) -> impl IntoElement {
    nsh_core::element::desktop::set_title_bar(
        nsh_core::desktop::TitleBarStyle::Hidden,
        Some(px(32.0)),
    );
    controller.dispatch(UiCommand::StartScan);
    let controller = Rc::new(RefCell::new(controller));
    let snapshot = signal(controller.borrow().snapshot());
    provide_context(computed(move || snapshot.get().settings.theme));
    effect(move || {
        nsh_core::element::ambient::set_color_scheme(
            if snapshot.get().settings.theme == theme::ThemeMode::Dark {
                nsh_core::platform::Appearance::Dark
            } else {
                nsh_core::platform::Appearance::Light
            },
        );
    });
    poll_controller(controller.clone(), snapshot);
    on_key("escape", {
        let c = controller.clone();
        move |_| {
            let mut c = c.borrow_mut();
            let current = c.snapshot();
            if current.exit_confirmation {
                c.dispatch(UiCommand::CancelExit);
            } else if current.cleanup_confirmation.is_some() {
                c.dispatch(UiCommand::CancelCleanup);
            } else if current.cancel_scan_confirmation {
                c.dispatch(UiCommand::DismissCancelScan);
            } else if current.space_analysis.selection.is_some() {
                c.dispatch(UiCommand::Analysis(AnalysisCommand::SetSelection(None)));
            }
            snapshot.set(c.snapshot());
        }
    });
    let modal_controller = controller.clone();
    let key_controller = controller.clone();
    let toolbar_controller = controller.clone();
    let nav = [
        ("首页", ViewId::Home),
        ("C盘专清", ViewId::Dashboard),
        ("空间分析", ViewId::SpaceAnalysis),
        ("垃圾清理", ViewId::Cleanup),
        ("大文件", ViewId::LargeFiles),
        ("设置", ViewId::Settings),
    ];
    let content = view()
        .w_full()
        .flex_1()
        .min_h(px(0.0))
        .flex_row()
        .bg(theme::color(theme::BACKGROUND))
        .child(
            view()
                .w(px(208.0))
                .h_full()
                .flex_col()
                .gap_2()
                .p_3()
                .bg(theme::color(theme::PANEL))
                .child(
                    text("Rusten")
                        .text_size(px(20.0))
                        .text_color(theme::color(theme::TEXT)),
                )
                .children(nav.into_iter().map(|(label, target)| {
                    let navigation = controller.clone();
                    view()
                        .w_full()
                        .h(px(40.0))
                        .flex_row()
                        .items_center()
                        .px_3()
                        .rounded_md()
                        .role(AccessibilityRole::Button)
                        .label(label)
                        .focusable(true)
                        .accessibility_state(move || AccessibilityState {
                            selected: snapshot.get().state.selected_view == target,
                            ..Default::default()
                        })
                        .bg(move || {
                            let state = snapshot.get();
                            rgb(if state.state.selected_view == target {
                                theme::SURFACE
                            } else {
                                theme::PANEL
                            }
                            .value(state.settings.theme))
                        })
                        .on_press(move |_: &()| {
                            let mut c = navigation.borrow_mut();
                            c.dispatch(UiCommand::Navigate(target));
                            snapshot.set(c.snapshot());
                        })
                        .child(
                            text(label)
                                .text_size(px(14.0))
                                .text_color(theme::color(theme::TEXT)),
                        )
                })),
        )
        .child(
            view()
                .flex_1()
                .h_full()
                .flex_col()
                .child(dynamic(move || {
                    pages::toolbar::render(snapshot.get(), toolbar_controller.clone())
                }))
                .child(scroll().flex_1().min_h(px(0.0)).label("页面内容").child(
                    view().p_5().child(dynamic(move || {
                        match snapshot.get().state.selected_view {
                            ViewId::Home | ViewId::Dashboard => {
                                pages::home::render(snapshot, controller.clone()).into_element()
                            }
                            ViewId::SpaceAnalysis => crate::space_analysis::view::render(
                                snapshot.get(),
                                controller.clone(),
                            )
                            .into_element(),
                            ViewId::Cleanup => {
                                pages::cleanup::render(snapshot, controller.clone()).into_element()
                            }
                            ViewId::LargeFiles => {
                                pages::large_files::render(snapshot, controller.clone())
                                    .into_element()
                            }
                            ViewId::Settings => {
                                pages::settings::render(snapshot, controller.clone()).into_element()
                            }
                        }
                    })),
                ))
                .child(
                    view()
                        .h(px(32.0))
                        .flex_row()
                        .items_center()
                        .px_4()
                        .bg(theme::color(theme::PANEL))
                        .child(
                            text(move || {
                                let current = snapshot.get();
                                if let Some(error) = current.error {
                                    return error;
                                }
                                let scan = &current.state.scan;
                                format!("{} 个文件  ·  {}%", scan.files, scan.progress_percent)
                            })
                            .text_size(px(13.0))
                            .text_color(theme::color(theme::MUTED)),
                        ),
                ),
        )
        .child(dynamic(move || {
            crate::components::active_confirmation(snapshot.get(), modal_controller.clone())
        }))
        .child(dynamic(move || {
            let current = snapshot.get();
            if current.state.selected_view == ViewId::SpaceAnalysis
                && current.cleanup_confirmation.is_none()
                && !current.cancel_scan_confirmation
                && !current.exit_confirmation
            {
                analysis_keys(snapshot, key_controller.clone());
            }
            view()
        }));
    view()
        .w_full()
        .h_full()
        .flex_col()
        .label("应用主题背景")
        .bg(theme::color(theme::BACKGROUND))
        .child(
            view()
                .h(px(32.0))
                .w_full()
                .flex_row()
                .items_center()
                .px_3()
                .pr(px(150.0))
                .window_drag(true)
                .label("主题标题栏")
                .bg(theme::color(theme::TITLE_BAR))
                .border_b_1()
                .border_color(theme::color(theme::BORDER))
                .child(
                    text("Rusten")
                        .text_size(px(12.0))
                        .text_color(theme::color(theme::TEXT)),
                ),
        )
        .child(content)
}

fn poll_controller(controller: Rc<RefCell<UiController>>, snapshot: Signal<UiSnapshot>) {
    let close_window = std::cell::Cell::new(None);
    let _timer = set_interval(Duration::from_millis(50), move || {
        let mut controller = controller.borrow_mut();
        if let Some(hwnd) = crate::window_close::take_request() {
            close_window.set(Some(hwnd));
            controller.dispatch(UiCommand::RequestExit);
        }
        controller.drain();
        if controller.exit_ready()
            && let Some(hwnd) = close_window.take()
        {
            crate::window_close::finish_close(hwnd);
        }
        let next = controller.snapshot();
        if snapshot.get() != next {
            if next.state.selected_view == ViewId::SpaceAnalysis
                && snapshot.get().space_analysis != next.space_analysis
                && !reduce_motion()
            {
                animate_layout(nsh_core::LayoutAnimation {
                    create: Some(nsh_core::LayoutTransition::new(
                        nsh_core::LayoutCurve::EaseOut,
                        320.0,
                    )),
                    update: Some(nsh_core::LayoutTransition::new(
                        nsh_core::LayoutCurve::EaseOut,
                        320.0,
                    )),
                    delete: Some(nsh_core::LayoutTransition::new(
                        nsh_core::LayoutCurve::EaseOut,
                        160.0,
                    )),
                });
            }
            snapshot.set(next);
        }
    });
}

fn analysis_keys(snapshot: Signal<UiSnapshot>, controller: Rc<RefCell<UiController>>) {
    for (key, command) in [
        ("right", AnalysisCommand::SelectNext(true)),
        ("down", AnalysisCommand::SelectNext(true)),
        ("left", AnalysisCommand::SelectNext(false)),
        ("up", AnalysisCommand::SelectNext(false)),
        ("enter", AnalysisCommand::OpenSelection),
        ("space", AnalysisCommand::ToggleSelectionMark),
        ("ctrl+=", AnalysisCommand::Zoom(true)),
        ("ctrl+shift+=", AnalysisCommand::Zoom(true)),
        ("ctrl+-", AnalysisCommand::Zoom(false)),
        ("ctrl+0", AnalysisCommand::Fit),
    ] {
        let c = controller.clone();
        on_key(key, move |_| {
            if snapshot.get().state.selected_view == ViewId::SpaceAnalysis
                && snapshot.get().cleanup_confirmation.is_none()
                && !snapshot.get().exit_confirmation
            {
                let mut c = c.borrow_mut();
                c.dispatch(UiCommand::Analysis(command.clone()));
                snapshot.set(c.snapshot());
            }
        });
    }
    on_key("alt+up", {
        let c = controller.clone();
        move |_| {
            if snapshot.get().state.selected_view == ViewId::SpaceAnalysis
                && snapshot.get().cleanup_confirmation.is_none()
                && !snapshot.get().exit_confirmation
            {
                c.borrow_mut().dispatch(UiCommand::OpenAnalysisParent);
            }
        }
    });
}
