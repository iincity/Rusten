use std::{cell::RefCell, rc::Rc};

use nsh_core::prelude::*;

use crate::{
    commands::UiCommand,
    components::{heading, row},
    controller::{UiController, UiSnapshot},
};

pub(crate) fn render(
    snapshot: Signal<UiSnapshot>,
    controller: Rc<RefCell<UiController>>,
) -> impl IntoElement {
    let state = snapshot.get();
    let settings = state.settings;
    view()
        .flex_col()
        .gap_3()
        .child(heading("设置", "外观与清理选项"))
        .child(
            row()
                .flex_wrap()
                .gap_2()
                .child(text("主题").text_color(crate::theme::color(crate::theme::TEXT)))
                .child(view().flex_row().gap_2().children(
                    crate::theme::ThemeMode::ALL.into_iter().map(|mode| {
                        let c = controller.clone();
                        view()
                            .role(AccessibilityRole::Button)
                            .label(format!("{}主题", mode.label()))
                            .focusable(true)
                            .accessibility_state(move || AccessibilityState {
                                selected: settings.theme == mode,
                                ..Default::default()
                            })
                            .disabled(state.settings_pending)
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .bg(crate::theme::color(if settings.theme == mode {
                                crate::theme::SURFACE
                            } else {
                                crate::theme::PANEL
                            }))
                            .border_1()
                            .border_color(crate::theme::color(crate::theme::BORDER))
                            .on_press(move |_: &()| {
                                c.borrow_mut().dispatch(UiCommand::SetTheme(mode));
                            })
                            .child(
                                view()
                                    .w(px(14.0))
                                    .h(px(14.0))
                                    .rounded_sm()
                                    .bg(rgb(crate::theme::BACKGROUND.value(mode))),
                            )
                            .child(
                                text(mode.label())
                                    .text_color(crate::theme::color(crate::theme::TEXT)),
                            )
                    }),
                )),
        )
        .child(
            text("启动后自动离线扫描；点击立即清理将直接处理已选项目。")
                .text_color(crate::theme::color(crate::theme::MUTED)),
        )
        .child(if let Some(error) = snapshot.get().error.clone() {
            text(error)
                .text_size(px(14.0))
                .text_color(crate::theme::color(crate::theme::DANGER))
        } else {
            text("")
        })
}
