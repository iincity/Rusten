#[cfg(test)]
mod tests {
    use domain::{DirectoryNode, FileIdentity, FileRecord, ScanIndex, normalize_path};
    use std::collections::HashSet;

    use super::{Metric, marked_cleanup_candidates, project_directory};
    use crate::space_analysis::layout::layout_treemap;

    #[test]
    fn cleanup_source_details_preserve_category_scope_and_bound_pages() {
        let index = populated_directory_index(0);
        let candidates = (0..90)
            .map(|n| {
                let file = FileRecord::new(
                    normalize_path(&format!(
                        r"C:\Fixture\{}\file-{n}.tmp",
                        if n % 2 == 0 { "a" } else { "b" }
                    ))
                    .unwrap(),
                    index.root_directory_id(),
                    FileIdentity {
                        volume_serial: 1,
                        file_index: n as u128 + 1,
                    },
                    n + 1,
                    n + 1,
                );
                let mut candidate = crate::runtime::explicit_file_candidate(&file);
                candidate.category = Some("cache".into());
                candidate
            })
            .collect::<Vec<_>>();
        let selected = std::collections::BTreeSet::from(["cache".into()]);
        let details = super::cleanup_source_details(
            &candidates,
            r"C:\Fixture",
            Some("cache".into()),
            &selected,
            &HashSet::new(),
            0,
        );
        assert_eq!(
            (
                details.total,
                details.bytes,
                details.items.len(),
                details.directory_total
            ),
            (90, 4095, 80, 2)
        );
        assert_eq!(details.items[0].1, 90);
        let second = super::cleanup_source_details(
            &candidates,
            r"C:\Fixture",
            None,
            &selected,
            &HashSet::new(),
            1,
        );
        assert_eq!((second.page, second.items.len()), (1, 10));
        let scoped = super::cleanup_source_details(
            &candidates,
            r"C:\Fixture",
            None,
            &selected,
            &HashSet::from([r"C:\Fixture\a".into()]),
            0,
        );
        assert_eq!(scoped.total, 45);
        assert!(
            scoped
                .items
                .iter()
                .all(|(p, _)| p.starts_with(r"C:\Fixture\a\"))
        );
        let sibling = super::cleanup_source_details(
            &candidates,
            r"C:\FixtureSibling",
            None,
            &selected,
            &HashSet::new(),
            0,
        );
        assert_eq!(sibling.total, 0);
        let other = super::cleanup_source_details(
            &candidates,
            r"C:\Fixture",
            Some("other".into()),
            &selected,
            &HashSet::new(),
            0,
        );
        assert_eq!(other.total, 0);
        let emptied = super::cleanup_source_details(
            &[],
            r"C:\Fixture",
            None,
            &selected,
            &HashSet::from([r"C:\Fixture\a".into()]),
            0,
        );
        assert_eq!(emptied.directories, vec![(r"C:\Fixture\a".into(), 0, true)]);
    }

    #[test]
    fn cleanup_source_details_exclude_files_outside_the_executable_plan() {
        let index = populated_directory_index(0);
        let mut eligible = crate::runtime::explicit_file_candidate(&FileRecord::new(
            normalize_path(r"C:\Fixture\eligible.log").unwrap(),
            index.root_directory_id(),
            FileIdentity {
                volume_serial: 1,
                file_index: 1,
            },
            10,
            10,
        ));
        eligible.category = Some("system.logs".into());
        let mut blocked = eligible.clone();
        blocked.path = normalize_path(r"C:\Fixture\blocked.log").unwrap();
        blocked.quick_clean_block_reason = Some("protected".into());
        blocked.mode = Some(domain::CleanupMode::Quick);
        let selected = std::collections::BTreeSet::from(["system.logs".into()]);

        let details = super::cleanup_source_details(
            &[eligible, blocked],
            r"C:\Fixture",
            Some("system.logs".into()),
            &selected,
            &HashSet::new(),
            0,
        );
        assert_eq!(details.total, 1);
        assert_eq!(details.items[0].0, r"C:\Fixture\eligible.log");
    }

    fn populated_directory_index(file_count: usize) -> ScanIndex {
        let root = normalize_path(r"C:\Fixture").unwrap();
        let mut index = ScanIndex::new(root);
        for number in 0..file_count {
            index.add_file(FileRecord::new(
                normalize_path(&format!(r"C:\Fixture\file-{number:03}.bin")).unwrap(),
                index.root_directory_id(),
                FileIdentity {
                    volume_serial: 1,
                    file_index: number as u128 + 1,
                },
                number as u64 + 1,
                number as u64 + 1,
            ));
        }
        index
    }

    #[test]
    fn preview_is_bounded_and_preserves_weight() {
        let index = populated_directory_index(500);
        let preview = project_directory(&index, index.root_directory_id(), Metric::Logical, 240);
        assert!(preview.nodes.len() <= 240);
        assert_eq!(
            preview.total_weight,
            preview.nodes.iter().map(|node| node.weight).sum::<u64>()
        );
    }

    #[test]
    fn large_directory_preview_avoids_repeated_page_sorting() {
        let index = populated_directory_index(16_000);
        let started = std::time::Instant::now();
        let preview = project_directory(&index, index.root_directory_id(), Metric::Logical, 240);
        let elapsed = started.elapsed();
        eprintln!("16000-file preview: {elapsed:?}");
        assert_eq!(preview.total_weight, 16_000 * 16_001 / 2);
        assert!(elapsed < std::time::Duration::from_secs(1), "{elapsed:?}");
    }

    #[test]
    fn omitted_preview_weight_is_one_aggregate_node() {
        let index = populated_directory_index(500);
        let preview = project_directory(&index, index.root_directory_id(), Metric::Logical, 10);
        assert_eq!(preview.nodes.len(), 10);
        assert_eq!(preview.omitted, 491);
        assert!(preview.nodes.last().unwrap().aggregate);
        assert_eq!(
            preview.total_weight,
            preview.nodes.iter().map(|node| node.weight).sum::<u64>()
        );
    }

    #[test]
    fn tiles_are_finite_and_inside_the_viewport() {
        let index = populated_directory_index(20);
        let preview = project_directory(&index, index.root_directory_id(), Metric::Logical, 240);
        let tiles = layout_treemap(&preview, 1200.0, 700.0);
        assert!(tiles.iter().all(|tile| tile.x.is_finite()
            && tile.y.is_finite()
            && tile.width.is_finite()
            && tile.height.is_finite()
            && tile.width >= 0.0
            && tile.height >= 0.0
            && tile.x + tile.width <= 1200.01
            && tile.y + tile.height <= 700.01));
    }

    #[test]
    fn zero_weight_and_empty_viewports_are_safe() {
        let index = populated_directory_index(0);
        let preview = project_directory(&index, index.root_directory_id(), Metric::Logical, 240);
        assert!(layout_treemap(&preview, 100.0, 100.0).is_empty());
        let populated = populated_directory_index(1);
        let preview = project_directory(
            &populated,
            populated.root_directory_id(),
            Metric::Logical,
            240,
        );
        assert!(layout_treemap(&preview, f32::INFINITY, 100.0).is_empty());
    }

    #[test]
    fn marked_directory_projects_all_descendant_files_into_cleanup_plan() {
        let root = normalize_path(r"C:\Fixture").unwrap();
        let child_path = normalize_path(r"C:\Fixture\cache").unwrap();
        let mut index = ScanIndex::new(root);
        let child = index
            .add_directory(DirectoryNode::new(
                child_path.clone(),
                index.root_directory_id(),
            ))
            .expect("directory added");
        index.add_file(FileRecord::new(
            normalize_path(r"C:\Fixture\cache\one.tmp").unwrap(),
            child,
            FileIdentity {
                volume_serial: 1,
                file_index: 7,
            },
            12,
            12,
        ));
        let mut marked = HashSet::new();
        marked.insert(child_path);
        let plan = marked_cleanup_candidates(&index, &marked);
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.estimated_bytes, 12);
        let preview = super::project_directory_at_depth(
            &index,
            index.root_directory_id(),
            Metric::Logical,
            240,
            2,
        );
        assert_eq!(preview.total_weight, 12);
        assert_eq!(preview.nodes[0].name, "one.tmp");
        assert!(preview.nodes[0].directory.is_none());
    }

    #[test]
    fn nested_map_keeps_sibling_directories_in_separate_regions() {
        let mut index = populated_directory_index(0);
        for (group, weight) in [("code", 20), ("media", 50)] {
            let dir = index
                .add_directory(DirectoryNode::new(
                    normalize_path(&format!(r"C:\Fixture\{group}")).unwrap(),
                    index.root_directory_id(),
                ))
                .unwrap();
            for number in 0..3 {
                index.add_file(FileRecord::new(
                    normalize_path(&format!(r"C:\Fixture\{group}\file-{number}.bin")).unwrap(),
                    dir,
                    FileIdentity {
                        volume_serial: 1,
                        file_index: weight as u128 * 10 + number,
                    },
                    weight,
                    weight,
                ));
            }
        }
        let preview = super::project_directory_at_depth(
            &index,
            index.root_directory_id(),
            Metric::Logical,
            240,
            2,
        );
        let tiles = crate::space_analysis::layout::layout_nested_treemap(
            &preview,
            r"C:\Fixture",
            900.0,
            400.0,
        );
        assert_eq!(tiles.len(), 6);
        for a in &tiles {
            assert!(
                a.x >= 0.0 && a.y >= 0.0 && a.x + a.width <= 900.01 && a.y + a.height <= 400.01
            );
            for b in &tiles {
                if a.node != b.node {
                    assert!(
                        a.x + a.width <= b.x + 0.01
                            || b.x + b.width <= a.x + 0.01
                            || a.y + a.height <= b.y + 0.01
                            || b.y + b.height <= a.y + 0.01
                    );
                }
            }
        }
        assert_eq!(
            crate::space_analysis::layout::layout_nested_treemap(
                &preview,
                r"C:\Fixture",
                f32::NAN,
                400.0
            )
            .len(),
            0
        );
    }
}
use std::collections::HashSet;

use application::CleanupPlan;
use domain::{DirectoryId, DirectoryPage, FileIdentity, NormalizedPath, ScanIndex};

pub const MAP_NODE_LIMIT: usize = 240;
pub const DIRECTORY_PAGE_SIZE: usize = 80;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Metric {
    Logical,
    Allocated,
    Files,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileCategory {
    Code,
    Git,
    Cache,
    Build,
    Toolchain,
    Media,
    Document,
    Archive,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpaceNode {
    pub logical_size: u64,
    pub allocated_size: u64,
    pub modified: u64,
    pub path: NormalizedPath,
    pub name: String,
    pub directory: Option<DirectoryId>,
    pub file_identity: Option<FileIdentity>,
    pub weight: u64,
    pub aggregate: bool,
    pub category: FileCategory,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapPreview {
    pub nodes: Vec<SpaceNode>,
    pub total_weight: u64,
    pub omitted: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tile {
    pub node: usize,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisSelection {
    pub path: NormalizedPath,
    pub directory: Option<DirectoryId>,
    pub file_identity: Option<FileIdentity>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpaceAnalysisSnapshot {
    pub cleanup_source: CleanupSourceDetails,
    pub directory: DirectoryId,
    pub breadcrumbs: Vec<(DirectoryId, String)>,
    pub metric: Metric,
    pub depth: u8,
    pub directory_path: String,
    pub directory_bytes: u64,
    pub child_directories: Vec<domain::DirectoryEntry>,
    pub page: DirectoryPage,
    pub preview: MapPreview,
    pub selection: Option<AnalysisSelection>,
    pub marked: HashSet<NormalizedPath>,
    pub filter: String,
    pub page_number: usize,
    pub zoom: f32,
    pub pan: (f32, f32),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CleanupSourceDetails {
    pub category: Option<String>,
    pub items: Vec<(String, u64)>,
    pub directories: Vec<(String, u64, bool)>,
    pub total: usize,
    pub directory_total: usize,
    pub bytes: u64,
    pub page: usize,
}

pub(crate) fn cleanup_source_details(
    candidates: &[domain::CleanupCandidate],
    scope: &str,
    category: Option<String>,
    selected_categories: &std::collections::BTreeSet<String>,
    selected_directories: &HashSet<String>,
    page: usize,
) -> CleanupSourceDetails {
    let mut details = CleanupSourceDetails {
        category,
        ..Default::default()
    };
    let mut items = Vec::new();
    let mut directories = std::collections::BTreeMap::<String, u64>::new();
    for item in candidates {
        let Some(key) = item.category.as_deref() else {
            continue;
        };
        if item.mode != Some(domain::CleanupMode::Manual)
            && !domain::can_enter_quick_clean(&item.clone().with_execution_facts(false, true, true))
        {
            continue;
        }
        if !details
            .category
            .as_deref()
            .map_or_else(|| selected_categories.contains(key), |active| active == key)
            || !crate::controller::path_is_within(scope, item.path.display())
        {
            continue;
        }
        let bytes = item.estimated_size.unwrap_or_default();
        let path = item.path.display().to_owned();
        let prefix = format!("{}\\", scope.trim_end_matches('\\'));
        let bucket = path
            .get(prefix.len()..)
            .and_then(|rest| rest.split_once('\\'))
            .map_or_else(|| scope.to_owned(), |(first, _)| format!("{prefix}{first}"));
        *directories.entry(bucket).or_default() += bytes;
        if selected_directories.is_empty()
            || selected_directories
                .iter()
                .any(|dir| crate::controller::path_is_within(dir, &path))
        {
            details.bytes += bytes;
            items.push((path, bytes));
        }
    }
    items.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    details.total = items.len();
    for path in selected_directories {
        if crate::controller::path_is_within(scope, path) {
            directories.entry(path.clone()).or_default();
        }
    }
    details.directory_total = directories.len();
    details.page = page
        .min(details.total.max(details.directory_total).saturating_sub(1) / DIRECTORY_PAGE_SIZE);
    details.items = items
        .into_iter()
        .skip(details.page * DIRECTORY_PAGE_SIZE)
        .take(DIRECTORY_PAGE_SIZE)
        .collect();
    details.directories = directories
        .into_iter()
        .skip(details.page * DIRECTORY_PAGE_SIZE)
        .take(DIRECTORY_PAGE_SIZE)
        .map(|(path, bytes)| {
            let selected = selected_directories.contains(&path);
            (path, bytes, selected)
        })
        .collect();
    details
}

pub fn project_directory(
    index: &ScanIndex,
    directory: DirectoryId,
    metric: Metric,
    limit: usize,
) -> MapPreview {
    project_directory_at_depth(index, directory, metric, limit, 1)
}

pub fn project_directory_at_depth(
    index: &ScanIndex,
    directory: DirectoryId,
    metric: Metric,
    limit: usize,
    depth: u8,
) -> MapPreview {
    let mut entries = Vec::new();
    let mut pending = vec![(directory, depth.clamp(1, 4))];
    while let Some((current, remaining)) = pending.pop() {
        for entry in index.directory_entries(current).iter() {
            if remaining > 1
                && let Some(child) = entry.directory_id
            {
                pending.push((child, remaining - 1));
            } else {
                entries.push(entry.clone());
            }
        }
    }
    let mut nodes: Vec<SpaceNode> = entries
        .into_iter()
        .map(|entry| {
            let weight = match metric {
                Metric::Logical => entry.logical_size,
                Metric::Allocated => entry.allocated_size,
                Metric::Files => entry
                    .directory_id
                    .and_then(|id| index.directory(id))
                    .map_or(1, |d| d.file_count()),
            };
            let category = category_for(&entry.name);
            SpaceNode {
                logical_size: entry.logical_size,
                allocated_size: entry.allocated_size,
                modified: entry.last_write_time,
                path: entry.path,
                name: entry.name,
                directory: entry.directory_id,
                file_identity: entry.file_identity,
                weight,
                aggregate: false,
                category,
            }
        })
        .collect();
    nodes.sort_by(|a, b| {
        b.weight
            .cmp(&a.weight)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let total_weight = nodes
        .iter()
        .fold(0u64, |sum, n| sum.saturating_add(n.weight));
    let limit = limit.min(MAP_NODE_LIMIT);
    // The aggregate itself occupies one visible slot, so it represents all
    // entries after the first `limit - 1` concrete nodes.
    let omitted = nodes.len().saturating_sub(limit.saturating_sub(1));
    if omitted > 0 && limit > 0 {
        let mut tail = nodes.split_off(limit - 1);
        let aggregate_weight = tail
            .iter()
            .fold(0u64, |sum, n| sum.saturating_add(n.weight));
        let path = index
            .directory(directory)
            .map(|d| d.path().clone())
            .unwrap_or_else(|| index.root().clone());
        nodes.push(SpaceNode {
            logical_size: 0,
            allocated_size: 0,
            modified: 0,
            path,
            name: format!("{omitted} more items"),
            directory: None,
            file_identity: None,
            weight: aggregate_weight,
            aggregate: true,
            category: FileCategory::Other,
        });
        tail.clear();
    } else {
        nodes.truncate(limit);
    }
    MapPreview {
        nodes,
        total_weight,
        omitted,
    }
}

fn category_for(name: &str) -> FileCategory {
    let lower = name.to_lowercase();
    if lower == ".git" {
        FileCategory::Git
    } else if [".tmp", ".cache"].iter().any(|s| lower.ends_with(s)) || lower == "cache" {
        FileCategory::Cache
    } else if ["target", "build", "dist"].contains(&lower.as_str()) {
        FileCategory::Build
    } else if [".rs", ".c", ".h", ".cpp", ".py", ".js", ".ts"]
        .iter()
        .any(|s| lower.ends_with(s))
    {
        FileCategory::Code
    } else if [".exe", ".dll", ".so", ".tool"]
        .iter()
        .any(|s| lower.ends_with(s))
    {
        FileCategory::Toolchain
    } else if [".png", ".jpg", ".jpeg", ".mp4", ".mp3"]
        .iter()
        .any(|s| lower.ends_with(s))
    {
        FileCategory::Media
    } else if [".pdf", ".doc", ".docx", ".txt", ".md"]
        .iter()
        .any(|s| lower.ends_with(s))
    {
        FileCategory::Document
    } else if [".zip", ".7z", ".rar", ".gz"]
        .iter()
        .any(|s| lower.ends_with(s))
    {
        FileCategory::Archive
    } else {
        FileCategory::Other
    }
}

/// Converts explicitly marked paths into the existing recoverable cleanup plan.
/// This only prepares a plan; execution remains in the established confirmation flow.
pub fn marked_cleanup_candidates(
    index: &ScanIndex,
    marked: &HashSet<NormalizedPath>,
) -> CleanupPlan {
    let candidates: Vec<_> = index
        .files()
        .into_iter()
        .filter(|file| {
            marked.iter().any(|path| {
                let parent = path.display().trim_end_matches(['\\', '/']);
                let child = file.path().display();
                child.eq_ignore_ascii_case(path.display())
                    || child.get(parent.len()..).is_some_and(|suffix| {
                        child[..parent.len()].eq_ignore_ascii_case(parent)
                            && (suffix.starts_with('\\') || suffix.starts_with('/'))
                    })
            })
        })
        .map(|file| crate::runtime::explicit_file_candidate(&file))
        .collect();
    crate::runtime::prepare_cleanup_plan_with_explicit(&candidates)
}
