use std::{cell::RefCell, rc::Rc};

use nsh_core::prelude::*;

use crate::{
    components::{format_bytes, heading, section, value},
    controller::{UiController, UiSnapshot},
};

pub(crate) fn render(
    snapshot: Signal<UiSnapshot>,
    controller: Rc<RefCell<UiController>>,
) -> impl IntoElement {
    let scan = snapshot.get().state.scan.clone();
    view()
        .flex_col()
        .gap_4()
        .child(heading("首页", "系统空间与清理状态"))
        .child(
            view()
                .flex_col()
                .gap_2()
                .p_4()
                .rounded_md()
                .bg(crate::theme::color(crate::theme::PANEL))
                .child(section("磁盘概览"))
                .child(value("总容量", format_bytes(scan.total_bytes)))
                .child(value("可用空间", format_bytes(scan.available_bytes)))
                .child(value("已扫描文件", scan.files.to_string()))
                .child(
                    text(format!("扫描逻辑字节：{} B", scan.discovered_bytes))
                        .text_color(crate::theme::color(crate::theme::TEXT)),
                )
                .child(value("扫描警告", scan.warnings.to_string()))
                .child(value(
                    "预计可清理",
                    format_bytes(snapshot.get().state.cleanup.estimated_bytes),
                ))
                .child(value("扫描状态", format!("{:?}", scan.status))),
        )
        .child(super::cleanup::categories(snapshot.get(), controller))
}
