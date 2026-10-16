#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreemapItem {
    pub label: String,
    pub bytes: u64,
    pub target_path: String,
}

pub fn tile(item: TreemapItem, maximum: u64) -> impl gpui::IntoElement {
    use crate::components::density::{TREEMAP_BASE_WIDTH, TREEMAP_EXTRA_WIDTH, TREEMAP_HEIGHT};
    use gpui::prelude::*;

    let ratio = item.bytes as f32 / maximum.max(1) as f32;
    gpui::div()
        .w(gpui::px(TREEMAP_BASE_WIDTH + ratio * TREEMAP_EXTRA_WIDTH))
        .h(gpui::px(TREEMAP_HEIGHT))
        .p_1()
        .text_xs()
        .truncate()
        .bg(gpui::rgb(0xeaf2ff))
        .border_1()
        .border_color(gpui::rgb(0xc7d8f5))
        .child(format!(
            "{} · {}",
            item.label,
            crate::components::format_bytes(item.bytes)
        ))
}
