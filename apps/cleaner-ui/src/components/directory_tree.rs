#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryRow {
    pub path: String,
    pub logical_bytes: u64,
    pub expanded: bool,
    pub selected: bool,
    pub depth: usize,
}

pub fn row(directory: DirectoryRow) -> impl gpui::IntoElement {
    use crate::components::density::DIRECTORY_ROW_HEIGHT;
    use gpui::prelude::*;

    gpui::div()
        .w_full()
        .h(gpui::px(DIRECTORY_ROW_HEIGHT))
        .min_h(gpui::px(DIRECTORY_ROW_HEIGHT))
        .min_w(gpui::px(0.0))
        .flex()
        .items_center()
        .px_1()
        .text_xs()
        .border_b_1()
        .border_color(gpui::rgb(0xf0f2f4))
        .ml(gpui::px(directory.depth as f32 * 8.0))
        .child(
            gpui::div()
                .w(gpui::px(14.0))
                .flex_shrink_0()
                .child(if directory.expanded { "v" } else { ">" }),
        )
        .child(
            gpui::div()
                .flex_1()
                .min_w(gpui::px(0.0))
                .truncate()
                .child(directory.path),
        )
        .child(
            gpui::div()
                .w(gpui::px(56.0))
                .flex_shrink_0()
                .text_color(gpui::rgb(0x66727e))
                .child(crate::components::format_bytes(directory.logical_bytes)),
        )
        .child(
            gpui::div()
                .w(gpui::px(44.0))
                .flex_shrink_0()
                .text_color(gpui::rgb(0x66727e))
                .child(if directory.selected {
                    "已选中"
                } else {
                    "未选中"
                }),
        )
}
