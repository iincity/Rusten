use crate::{
    commands::{AnalysisCommand, UiCommand},
    components::{action, format_bytes, section},
    controller::{UiController, UiSnapshot},
    theme,
};
use nsh_core::prelude::*;
use std::{cell::RefCell, rc::Rc};

pub(crate) fn render(state: &UiSnapshot, controller: Rc<RefCell<UiController>>) -> View {
    let details = &state.space_analysis.cleanup_source;
    if details.category.is_none() && details.total == 0 && details.directory_total == 0 {
        return view();
    }
    let name = state
        .state
        .cleanup
        .categories
        .iter()
        .find(|c| Some(&c.key) == details.category.as_ref())
        .map_or("全部已选来源", |c| c.name.as_str());
    let pages = details
        .total
        .max(details.directory_total)
        .div_ceil(80)
        .max(1);
    view()
        .flex_col()
        .gap_2()
        .p_3()
        .bg(theme::color(theme::PANEL))
        .label("清理来源明细")
        .child(section("清理来源明细与目录限定"))
        .child(
            text(format!(
                "{name} · {} 项 · {} · 第 {} / {} 页",
                details.total,
                format_bytes(details.bytes),
                details.page + 1,
                pages
            ))
            .text_color(theme::color(theme::TEXT)),
        )
        .child(
            text(
                "勾选目录可限定立即清理的范围；未勾选时处理全部已选来源。点击立即清理后直接执行。",
            )
            .text_size(px(12.0))
            .text_color(theme::color(theme::MUTED)),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_2()
                .child(action("全部已选来源", {
                    let c = controller.clone();
                    move || {
                        c.borrow_mut()
                            .dispatch(UiCommand::InspectCleanupCategory(String::new()));
                    }
                }))
                .child(
                    action("上一页来源明细", {
                        let c = controller.clone();
                        let page = details.page;
                        move || {
                            c.borrow_mut().dispatch(UiCommand::Analysis(
                                AnalysisCommand::SetCleanupSourcePage(page.saturating_sub(1)),
                            ));
                        }
                    })
                    .disabled(details.page == 0),
                )
                .child(
                    action("下一页来源明细", {
                        let c = controller.clone();
                        let page = details.page;
                        move || {
                            c.borrow_mut().dispatch(UiCommand::Analysis(
                                AnalysisCommand::SetCleanupSourcePage(page + 1),
                            ));
                        }
                    })
                    .disabled(details.page + 1 >= pages),
                ),
        )
        .child(
            scroll().max_h(px(160.0)).child(
                view()
                    .flex_col()
                    .gap_2()
                    .children(details.directories.iter().map(|(path, bytes, selected)| {
                        let c = controller.clone();
                        let key = path.clone();
                        let selected = *selected;
                        view()
                            .role(AccessibilityRole::Checkbox)
                            .label(format!("限定目录 {path}"))
                            .focusable(true)
                            .disabled(state.cleanup_busy)
                            .accessibility_state(move || AccessibilityState {
                                checked: Some(if selected {
                                    nsh_core::Checked::True
                                } else {
                                    nsh_core::Checked::False
                                }),
                                ..Default::default()
                            })
                            .on_press(move |_: &()| {
                                c.borrow_mut()
                                    .dispatch(UiCommand::ToggleDirectorySelection(key.clone()));
                            })
                            .child(
                                text(format!(
                                    "{} {path} · {}",
                                    if selected { "☑" } else { "☐" },
                                    format_bytes(*bytes)
                                ))
                                .text_color(theme::color(theme::TEXT)),
                            )
                    }))
                    .children(details.items.iter().map(|(path, bytes)| {
                        text(format!("{path} · {}", format_bytes(*bytes)))
                            .text_size(px(12.0))
                            .text_color(theme::color(theme::MUTED))
                    })),
            ),
        )
}
