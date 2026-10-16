use domain::NormalizedPath;

use super::{PlatformError, metadata::read_metadata};

pub fn is_hardlink_pair(
    left: &NormalizedPath,
    right: &NormalizedPath,
) -> Result<bool, PlatformError> {
    let left = read_metadata(left)?;
    let right = read_metadata(right)?;
    Ok(left.link_count() > 1 && left.identity() == right.identity())
}
