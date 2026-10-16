#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResultSummary {
    pub released_bytes: u64,
    pub skipped: u64,
    pub failed: u64,
    pub recoverable: u64,
}

pub fn render(summary: &ResultSummary) -> impl gpui::IntoElement {
    use gpui::prelude::*;
    gpui::div()
        .flex()
        .gap_1()
        .text_xs()
        .child(format!(
            "已清理 {}",
            crate::components::format_bytes(summary.released_bytes)
        ))
        .child(format!("跳过 {}", summary.skipped))
        .child(format!("失败 {}", summary.failed))
}
