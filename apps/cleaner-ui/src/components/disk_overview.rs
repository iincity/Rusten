#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiskOverview {
    pub label: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub reclaimable_bytes: u64,
}

pub fn render(disk: &DiskOverview) -> impl gpui::IntoElement {
    use gpui::prelude::*;
    gpui::div()
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .p_2()
        .text_xs()
        .child(disk.label.clone())
        .child(gpui::div().child(format!(
            "{} / {}",
            crate::components::format_bytes(disk.used_bytes),
            crate::components::format_bytes(disk.total_bytes)
        )))
}
