#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LargeFileRow {
    pub path: String,
    pub identity: domain::FileIdentity,
    pub size_bytes: u64,
    pub selected: bool,
}

pub fn row(file: LargeFileRow) -> impl gpui::IntoElement {
    use crate::components::density::LARGE_FILE_ROW_HEIGHT;
    use gpui::prelude::*;

    gpui::div()
        .w_full()
        .h(gpui::px(LARGE_FILE_ROW_HEIGHT))
        .min_h(gpui::px(LARGE_FILE_ROW_HEIGHT))
        .flex()
        .items_center()
        .gap_1()
        .px_1()
        .py(gpui::px(2.0))
        .text_xs()
        .border_b_1()
        .border_color(gpui::rgb(0xf0f2f4))
        .child(if file.selected { "☑" } else { "☐" })
        .child(
            gpui::div()
                .flex_1()
                .min_w(gpui::px(0.0))
                .truncate()
                .child(file.path),
        )
        .child(gpui::div().child(crate::components::format_bytes(file.size_bytes)))
}
