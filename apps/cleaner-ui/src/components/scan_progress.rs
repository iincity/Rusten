use crate::state::ScanViewState;
use gpui::{div, prelude::*, px, rgb};

pub fn render(state: &ScanViewState) -> impl IntoElement {
    div().w_full().h(px(2.0)).bg(rgb(0xe5e7eb)).child(
        div()
            .h(px(2.0))
            .w(gpui::relative(state.progress_percent as f32 / 100.0))
            .bg(rgb(0x2563eb)),
    )
}
