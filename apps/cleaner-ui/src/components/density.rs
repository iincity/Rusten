pub const SIDEBAR_WIDTH: f32 = 164.0;
pub const CATEGORY_WIDTH: f32 = 112.0;
pub const CATEGORY_HEIGHT: f32 = 68.0;
pub const LARGE_FILE_ROW_HEIGHT: f32 = 28.0;
pub const DIRECTORY_ROW_HEIGHT: f32 = 26.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_layout_dimensions_are_compact() {
        assert_eq!(SIDEBAR_WIDTH, 164.0);
        assert_eq!(CATEGORY_WIDTH, 112.0);
        assert_eq!(CATEGORY_HEIGHT, 68.0);
        assert_eq!(LARGE_FILE_ROW_HEIGHT, 28.0);
        assert_eq!(DIRECTORY_ROW_HEIGHT, 26.0);
    }
}
