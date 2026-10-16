#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupCategory {
    pub name: String,
    pub estimated_bytes: u64,
    pub executable_bytes: Option<u64>,
    pub selected: bool,
}

pub fn render(category: CleanupCategory) -> impl gpui::IntoElement {
    use crate::components::density::{CATEGORY_HEIGHT, CATEGORY_WIDTH};
    use gpui::prelude::*;

    gpui::div()
        .w(gpui::px(CATEGORY_WIDTH))
        .h(gpui::px(CATEGORY_HEIGHT))
        .min_w(gpui::px(CATEGORY_WIDTH))
        .min_h(gpui::px(CATEGORY_HEIGHT))
        .flex()
        .flex_col()
        .text_xs()
        .p_1()
        .rounded_sm()
        .border_1()
        .border_color(if category.selected {
            gpui::rgb(0xb9e6a8)
        } else {
            gpui::rgb(0xe6efe2)
        })
        .bg(if category.selected {
            gpui::rgb(0xf7fff3)
        } else {
            gpui::rgb(0xffffff)
        })
        .child(
            gpui::div()
                .flex()
                .items_center()
                .gap_1()
                .child(if category.selected { "☑" } else { "☐" })
                .child(
                    gpui::div()
                        .flex_1()
                        .min_w(gpui::px(0.0))
                        .truncate()
                        .child(category.name),
                ),
        )
        .child(gpui::div().flex().items_center().gap_1().child(
            gpui::div().text_sm().text_color(gpui::rgb(0x2563eb)).child(
                match category.executable_bytes {
                    Some(0) if category.estimated_bytes > 0 => "仅统计占用".into(),
                    Some(bytes) => {
                        format!("可清理 {}", crate::components::format_bytes(bytes))
                    }
                    None => "统计中".into(),
                },
            ),
        ))
        .child(
            gpui::div()
                .text_xs()
                .text_color(gpui::rgb(0x66727e))
                .child(format!(
                    "占用 {}",
                    crate::components::format_bytes(category.estimated_bytes)
                )),
        )
}
