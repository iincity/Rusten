use super::model::{MapPreview, Tile};

pub fn layout_nested_treemap(
    preview: &MapPreview,
    root: &str,
    width: f32,
    height: f32,
) -> Vec<Tile> {
    fn branch(preview: &MapPreview, scope: &str, bounds: Tile, level: u8, out: &mut Vec<Tile>) {
        let mut groups = std::collections::BTreeMap::<String, Vec<usize>>::new();
        for (i, node) in preview.nodes.iter().enumerate() {
            let key = std::path::Path::new(node.path.display())
                .strip_prefix(scope)
                .ok()
                .and_then(|path| path.components().next())
                .map_or_else(
                    || node.path.display().to_owned(),
                    |part| {
                        std::path::Path::new(scope)
                            .join(part)
                            .to_string_lossy()
                            .into_owned()
                    },
                );
            groups.entry(key).or_default().push(i);
        }
        let groups: Vec<_> = groups.into_iter().collect();
        let mut grouped = preview.clone();
        grouped.nodes = groups
            .iter()
            .map(|(_, indices)| {
                let mut node = preview.nodes[indices[0]].clone();
                node.weight = indices.iter().map(|&i| preview.nodes[i].weight).sum();
                node
            })
            .collect();
        for mut tile in layout_treemap(&grouped, bounds.width, bounds.height) {
            let (scope, indices) = &groups[tile.node];
            tile.x += bounds.x;
            tile.y += bounds.y;
            if indices.len() > 1 && level < 4 {
                let mut child = preview.clone();
                child.nodes = indices.iter().map(|&i| preview.nodes[i].clone()).collect();
                let start = out.len();
                let inset = 3.0_f32.min(tile.width / 4.0).min(tile.height / 4.0);
                branch(
                    &child,
                    scope,
                    Tile {
                        x: tile.x + inset,
                        y: tile.y + inset,
                        width: tile.width - inset * 2.0,
                        height: tile.height - inset * 2.0,
                        ..tile
                    },
                    level + 1,
                    out,
                );
                for leaf in &mut out[start..] {
                    leaf.node = indices[leaf.node];
                }
            } else if indices.len() == 1 {
                tile.node = indices[0];
                out.push(tile);
            } else {
                let mut child = preview.clone();
                child.nodes = indices.iter().map(|&i| preview.nodes[i].clone()).collect();
                out.extend(
                    layout_treemap(&child, tile.width, tile.height)
                        .into_iter()
                        .map(|mut leaf| {
                            leaf.node = indices[leaf.node];
                            leaf.x += tile.x;
                            leaf.y += tile.y;
                            leaf
                        }),
                );
            }
        }
    }
    let mut tiles = Vec::new();
    branch(
        preview,
        root,
        Tile {
            node: 0,
            x: 0.0,
            y: 0.0,
            width,
            height,
        },
        0,
        &mut tiles,
    );
    tiles
}

/// Lay out the preview using a squarified treemap. Invalid viewports and
/// zero-weight entries produce empty, finite tiles rather than NaNs.
pub fn layout_treemap(preview: &MapPreview, width: f32, height: f32) -> Vec<Tile> {
    if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
        return Vec::new();
    }
    let mut weights: Vec<(usize, f64)> = preview
        .nodes
        .iter()
        .enumerate()
        .filter_map(|(i, node)| (node.weight > 0).then_some((i, node.weight as f64)))
        .collect();
    if weights.is_empty() {
        return Vec::new();
    }
    weights.sort_by(|a, b| b.1.total_cmp(&a.1));
    let total: f64 = weights.iter().map(|(_, w)| *w).sum();
    if !total.is_finite() || total <= 0.0 {
        return Vec::new();
    }
    let area = width as f64 * height as f64;
    let mut areas: Vec<(usize, f64)> = weights
        .into_iter()
        .map(|(i, w)| (i, w / total * area))
        .collect();
    let (mut x, mut y, mut w, mut h) = (0.0f64, 0.0f64, width as f64, height as f64);
    let mut tiles = Vec::with_capacity(areas.len());
    while !areas.is_empty() {
        let side = w.min(h);
        let mut row = Vec::new();
        while !areas.is_empty()
            && (row.is_empty()
                || worst_ratio(&row, side)
                    >= worst_ratio_with(
                        row.iter()
                            .map(|(_, a)| *a)
                            .chain(std::iter::once(areas[0].1)),
                        side,
                    ))
        {
            row.push(areas.remove(0));
        }
        let row_area: f64 = row.iter().map(|(_, a)| *a).sum();
        if w >= h {
            let row_width = (row_area / h).clamp(0.0, w);
            let mut cy = y;
            for (i, a) in row {
                let tile_h = if row_width > 0.0 {
                    (a / row_width).clamp(0.0, h)
                } else {
                    0.0
                };
                tiles.push(Tile {
                    node: i,
                    x: x as f32,
                    y: cy as f32,
                    width: row_width as f32,
                    height: tile_h as f32,
                });
                cy += tile_h;
            }
            x += row_width;
            w = (w - row_width).max(0.0);
        } else {
            let row_height = (row_area / w).clamp(0.0, h);
            let mut cx = x;
            for (i, a) in row {
                let tile_w = if row_height > 0.0 {
                    (a / row_height).clamp(0.0, w)
                } else {
                    0.0
                };
                tiles.push(Tile {
                    node: i,
                    x: cx as f32,
                    y: y as f32,
                    width: tile_w as f32,
                    height: row_height as f32,
                });
                cx += tile_w;
            }
            y += row_height;
            h = (h - row_height).max(0.0);
        }
    }
    tiles
}

fn worst_ratio(row: &[(usize, f64)], side: f64) -> f64 {
    worst_ratio_with(row.iter().map(|(_, a)| *a), side)
}

fn worst_ratio_with(areas: impl Iterator<Item = f64>, side: f64) -> f64 {
    let values: Vec<f64> = areas.collect();
    if values.is_empty() || side <= 0.0 {
        return f64::INFINITY;
    }
    let sum: f64 = values.iter().sum();
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(0.0, f64::max);
    (side * side * max / (sum * sum)).max(sum * sum / (side * side * min))
}
