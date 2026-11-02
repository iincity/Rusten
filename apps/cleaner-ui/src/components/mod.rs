pub(crate) fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

pub(crate) fn format_modified(filetime: u64) -> String {
    if filetime == 0 {
        return "未知".into();
    }
    let seconds = (filetime / 10_000_000) as i64 - 11_644_473_600;
    // Gregorian civil date from days since the Unix epoch, including pre-1970 dates.
    let days = seconds.div_euclid(86_400) + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month + 2) / 5 + 1;
    let month = month + if month < 10 { 3 } else { -9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let time = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        time / 3600,
        time / 60 % 60
    )
}

use nsh_base::dialog;
use nsh_core::prelude::*;

pub(crate) fn confirmation(
    title: &'static str,
    confirm_label: &'static str,
    confirm: impl Fn() + 'static,
    cancel: impl Fn() + Clone + 'static,
    busy: bool,
    compact: bool,
    details: impl IntoElement,
) -> impl IntoElement {
    let on_cancel = cancel.clone();
    let cancel_focus = element_ref();
    dialog()
        .animation(ModalAnimation::None)
        .close_on_escape(false)
        .close_on_backdrop_press(false)
        .on_cancel(move |_: &()| on_cancel())
        .on_open(move |_: &()| cancel_focus.set_accessibility_focus())
        .child(
            view()
                .flex_1()
                .items_center()
                .justify_center()
                .p_4()
                .bg(rgba(0x00000099))
                .child(
                    view()
                        .w_full()
                        .max_w(px(if compact { 440.0 } else { 620.0 }))
                        .max_h(percentage(0.9))
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(crate::theme::color(crate::theme::BORDER))
                        .bg(crate::theme::color(crate::theme::PANEL))
                        .role(AccessibilityRole::Alert)
                        .label(title)
                        .child(
                            view()
                                .p(px(if compact { 24.0 } else { 16.0 }))
                                .gap_3()
                                .child(
                                    text(title)
                                        .text_size(px(18.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(crate::theme::color(crate::theme::TEXT)),
                                )
                                .child(scroll().max_h(px(360.0)).child(details)),
                        )
                        .child(
                            view()
                                .flex_row()
                                .justify_end()
                                .items_center()
                                .px(px(if compact { 24.0 } else { 16.0 }))
                                .py_4()
                                .border_t_1()
                                .border_color(crate::theme::color(crate::theme::BORDER))
                                .gap_2()
                                .child(
                                    action("取消", cancel)
                                        .element_ref(&cancel_focus)
                                        .min_w(px(88.0))
                                        .h(px(36.0))
                                        .items_center()
                                        .justify_center()
                                        .disabled(busy),
                                )
                                .child(
                                    command_button(confirm_label, confirm, compact)
                                        .min_w(px(96.0))
                                        .h(px(36.0))
                                        .items_center()
                                        .justify_center()
                                        .disabled(busy),
                                ),
                        ),
                ),
        )
}

pub(crate) fn active_confirmation(
    state: crate::controller::UiSnapshot,
    controller: std::rc::Rc<std::cell::RefCell<crate::controller::UiController>>,
) -> nsh_core::element::AnyElement {
    use crate::commands::UiCommand;
    let busy = state.exit_pending;
    let (title, label, confirm, cancel, details) = if state.exit_confirmation {
        (
            "确认退出 Rusten？",
            "确认退出",
            UiCommand::ConfirmExit,
            UiCommand::CancelExit,
            view().child(
                text("当前扫描或清理将停止。\n已完成的清理不会撤销。")
                    .text_size(px(14.0))
                    .text_color(crate::theme::color(crate::theme::MUTED)),
            ),
        )
    } else if let Some(plan) = state.cleanup_confirmation {
        (
            "确认清理所选文件？",
            "确认清理",
            UiCommand::ConfirmCleanup,
            UiCommand::CancelCleanup,
            view()
                .gap_2()
                .child(
                    text(format!(
                        "{} 项 · {} · {} 项不可恢复",
                        plan.item_count,
                        format_bytes(plan.estimated_bytes),
                        plan.irreversible_count
                    ))
                    .text_color(crate::theme::color(crate::theme::WARNING)),
                )
                .children(plan.warnings.into_iter().map(|warning| {
                    text(warning).text_color(crate::theme::color(crate::theme::WARNING))
                }))
                .child(
                    text("所选文件将永久删除，不创建备份，无法在应用内恢复。请核对路径后确认。")
                        .text_color(crate::theme::color(crate::theme::MUTED)),
                )
                .child(
                    view()
                        .flex_row()
                        .gap_2()
                        .child(
                            action("上一页确认路径", {
                                let c = controller.clone();
                                move || {
                                    c.borrow_mut().dispatch(UiCommand::SetConfirmationPage(
                                        plan.page.saturating_sub(1),
                                    ));
                                }
                            })
                            .disabled(plan.page == 0),
                        )
                        .child(
                            text(format!(
                                "第 {} / {} 页",
                                plan.page + 1,
                                plan.item_count.div_ceil(80)
                            ))
                            .text_color(crate::theme::color(crate::theme::TEXT)),
                        )
                        .child(
                            action("下一页确认路径", {
                                let c = controller.clone();
                                move || {
                                    c.borrow_mut()
                                        .dispatch(UiCommand::SetConfirmationPage(plan.page + 1));
                                }
                            })
                            .disabled((plan.page + 1) * 80 >= plan.item_count),
                        ),
                )
                .children(
                    plan.paths
                        .into_iter()
                        .map(|path| text(path).text_color(crate::theme::color(crate::theme::TEXT))),
                ),
        )
    } else if state.cancel_scan_confirmation {
        (
            "取消当前扫描？",
            "确认取消扫描",
            UiCommand::CancelScan,
            UiCommand::DismissCancelScan,
            view().child(
                text("已扫描结果会保留，取消后可重新扫描。")
                    .text_color(crate::theme::color(crate::theme::TEXT)),
            ),
        )
    } else {
        return view().into_element();
    };
    let c = controller.clone();
    confirmation(
        title,
        label,
        move || {
            c.borrow_mut().dispatch(confirm.clone());
        },
        move || {
            controller.borrow_mut().dispatch(cancel.clone());
        },
        busy,
        state.exit_confirmation,
        details,
    )
    .into_element()
}

pub(crate) fn action(label: &'static str, on_press: impl Fn() + 'static) -> View {
    command_button(label, on_press, false)
}

fn command_button(label: &'static str, on_press: impl Fn() + 'static, primary: bool) -> View {
    view()
        .role(AccessibilityRole::Button)
        .label(label)
        .focusable(true)
        .min_h(px(32.0))
        .focus(|style| {
            style
                .ring(px(2.0))
                .ring_color(crate::theme::color(crate::theme::ACCENT))
        })
        .px_3()
        .py_2()
        .rounded_md()
        .bg(crate::theme::color(if primary {
            crate::theme::ACCENT
        } else {
            crate::theme::SURFACE
        }))
        .on_press(move |_: &()| on_press())
        .child(
            text(label)
                .text_size(px(14.0))
                .text_color(crate::theme::color(if primary {
                    crate::theme::ON_ACCENT
                } else {
                    crate::theme::TEXT
                })),
        )
}

pub(crate) fn heading(title: &'static str, subtitle: &'static str) -> impl IntoElement {
    view()
        .gap_1()
        .child(
            text(title)
                .text_size(px(24.0))
                .text_color(crate::theme::color(crate::theme::TEXT)),
        )
        .child(
            text(subtitle)
                .text_size(px(14.0))
                .text_color(crate::theme::color(crate::theme::MUTED)),
        )
}

pub(crate) fn section(title: &'static str) -> impl IntoElement {
    text(title)
        .text_size(px(16.0))
        .text_color(crate::theme::color(crate::theme::TEXT))
}

pub(crate) fn value(label: &'static str, value: String) -> impl IntoElement {
    view()
        .flex_row()
        .justify_between()
        .child(
            text(label)
                .text_size(px(14.0))
                .text_color(crate::theme::color(crate::theme::MUTED)),
        )
        .child(
            text(value)
                .text_size(px(14.0))
                .text_color(crate::theme::color(crate::theme::TEXT)),
        )
}

pub(crate) fn row() -> View {
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .px_3()
        .py_2()
        .bg(crate::theme::color(crate::theme::PANEL))
}

#[cfg(test)]
mod tests {
    use super::format_bytes;

    #[test]
    fn formats_windows_modification_time_as_utc_calendar_date() {
        assert_eq!(super::format_modified(0), "未知");
        assert_eq!(
            super::format_modified(116_444_736_000_000_000),
            "1970-01-01 00:00 UTC"
        );
        assert_eq!(
            super::format_modified((11_644_473_600 + 951_782_400) * 10_000_000),
            "2000-02-29 00:00 UTC"
        );
    }

    #[test]
    fn formats_binary_size_units_through_tb() {
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1 KB");
        assert_eq!(format_bytes(1024 * 1024), "1 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1 GB");
        assert_eq!(format_bytes(1024_u64.pow(4)), "1 TB");
    }
}
