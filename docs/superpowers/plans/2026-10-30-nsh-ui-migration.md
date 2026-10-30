# NSH UI Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (- [ ]) syntax for tracking.

**Goal:** Replace Rusten's complete GPUI interface with a native NSH Windows interface, including a Disktree-style Space Analysis view and verified real scan, cleanup, and recovery flows.

**Architecture:** Run the final application directly on nsh-platform-windows. Extract the business workflow currently mixed into CleanerApp into a renderer-independent controller, expose bounded ScanIndex queries for Space Analysis, and render controller snapshots through NSH signals. Keep all cleanup execution inside the existing application and Windows platform services.

**Tech Stack:** Rust 2024, NSH v0.3.0 (nsh-core, nsh-base, nsh-platform-windows, nsh-engine, nsh-testing), existing Rusten application/domain/platform/persistence crates, Windows Shell Recycle Bin APIs.

**Spec:** docs/superpowers/specs/2026-10-30-nsh-ui-migration-design.md

## Global Constraints

- Support Windows 10 version 1809 and newer only in this migration.
- Pin every NSH Git dependency to tag v0.3.0; do not depend on Disktree as a crate.
- Use Rust NSH components only: no TSX, QuickJS, plugins, or runtime-delivered screens.
- Preserve existing scan, cleanup, recovery, settings, identity-revalidation, and persistence behavior.
- Never port Disktree's permanent-delete or empty-trash UI actions.
- Keep filesystem scanning and cleanup off the NSH application thread.
- Bound directory pages to 80 items and treemap previews to 240 visible nodes.
- Treat marking as reversible UI state; deletion starts only through the existing cleanup plan and confirmation flow.
- Destructive tests may touch only normalized descendants of a unique runtime-created fixture root.
- A build, test, UI, scan reconciliation, cleanup, recovery, or persistence check that was not run is not a pass.

## Final File Map

- apps/cleaner-ui/src/main.rs: Windows NSH launcher and startup failure handling.
- apps/cleaner-ui/src/lib.rs: NSH registration and public root component.
- apps/cleaner-ui/src/app.rs: root shell, navigation, status, and modal routing only.
- apps/cleaner-ui/src/controller.rs: renderer-independent commands, workers, cleanup/recovery workflow, and snapshots.
- apps/cleaner-ui/src/state.rs: display state and command transitions without renderer types.
- apps/cleaner-ui/src/theme.rs: dark semantic colors and layout tokens.
- apps/cleaner-ui/src/components.rs: shared NSH controls and formatting.
- apps/cleaner-ui/src/pages/*.rs: Home, Cleanup, Large Files, Recovery, and Settings.
- apps/cleaner-ui/src/space_analysis/{mod,model,layout,view}.rs: bounded projection, geometry, and explorer UI.
- apps/cleaner-ui/tests/{fixture,ui,layout}.rs: path guard, workflow, and layout tests.
- scripts/test-cleaner-ui.ps1: repeatable acceptance orchestration.
- crates/domain/src/scan_index.rs: bounded read-only directory/file query API.

---

### Task 1: Prove NSH v0.3.0 On Windows

**Files:**
- Modify: Cargo.toml
- Modify: apps/cleaner-ui/Cargo.toml
- Modify: Cargo.lock
- Create: apps/cleaner-ui/src/lib.rs
- Create: apps/cleaner-ui/src/nsh_probe.rs
- Create: apps/cleaner-ui/src/bin/cleaner-ui-nsh.rs

**Interfaces:**
- Consumes: NSH Root::of, AppSource::Rust, AppOptions, and run_app.
- Produces: cleaner_ui::App, cleaner_ui::__nsh_native_module(), and a temporary migration binary retained through Task 6.

- [ ] **Step 1: Add the root component before declaring NSH**

Create lib.rs:

~~~rust
mod nsh_probe;
pub use nsh_probe::App;

use nsh_core::prelude::*;

#[native_module("rusten/native")]
fn native(_registry: &mut Registry) {}
~~~

Create nsh_probe.rs:

~~~rust
use nsh_core::prelude::*;

#[component]
pub fn App() -> impl IntoElement {
    view()
        .flex_1()
        .items_center()
        .justify_center()
        .bg(rgb(0x09090b))
        .child(Text::new("Rusten NSH compatibility check"))
}
~~~

- [ ] **Step 2: Verify the expected missing dependency failure**

Run: cargo check -p cleaner-ui --lib

Expected: FAIL because nsh_core is not declared.

- [ ] **Step 3: Pin dependencies and add the launcher**

Add to workspace dependencies:

~~~toml
nsh-core = "=0.3.0"
nsh-base = { git = "https://github.com/longbridge/nsh.git", tag = "v0.3.0", default-features = false }
nsh-platform-windows = { git = "https://github.com/longbridge/nsh.git", tag = "v0.3.0", default-features = false }
nsh-engine = { git = "https://github.com/longbridge/nsh.git", tag = "v0.3.0", default-features = false }
nsh-testing = { git = "https://github.com/longbridge/nsh.git", tag = "v0.3.0", default-features = false }
log = "0.4"

[patch.crates-io]
nsh-core = { git = "https://github.com/longbridge/nsh.git", tag = "v0.3.0" }
~~~

Add core/base/platform/log to normal cleaner-ui dependencies and engine/testing to dev-dependencies.

Create cleaner-ui-nsh.rs:

~~~rust
#![cfg_attr(windows, windows_subsystem = "windows")]

use nsh_platform_windows as platform;

fn main() -> impl std::process::Termination {
    platform::set_ui_sets(vec![
        nsh_base::registry(),
        cleaner_ui::__nsh_native_module(),
    ]);
    let app_id = "com.rusten.cleaner";
    platform::run_app(
        platform::AppSource::Rust(
            platform::Root::of::<cleaner_ui::App>().app_id(app_id),
        ),
        platform::AppOptions {
            app_id: app_id.into(),
            title: "Rusten".into(),
            size: (1180, 760),
            appearance: platform::Appearance::Dark,
            ..Default::default()
        },
    )
}
~~~

- [ ] **Step 4: Enforce the compatibility gate**

Run:

~~~powershell
cargo check -p cleaner-ui --lib
cargo build -p cleaner-ui --bin cleaner-ui-nsh
cargo build -p cleaner-ui --bin cleaner-ui-nsh --release
~~~

Expected: PASS. Launch target\debug\cleaner-ui-nsh.exe and confirm a visible, nonblank window containing the probe text. Stop and resolve compatibility before Task 2 if any check fails.

- [ ] **Step 5: Commit**

~~~bash
git add Cargo.toml Cargo.lock apps/cleaner-ui
git commit -m "build(ui): prove NSH Windows compatibility"
~~~

---

### Task 2: Add Bounded ScanIndex Queries And Treemap Geometry

**Files:**
- Modify: crates/domain/src/scan_index.rs
- Modify: crates/domain/src/lib.rs
- Create: apps/cleaner-ui/src/space_analysis/mod.rs
- Create: apps/cleaner-ui/src/space_analysis/model.rs
- Create: apps/cleaner-ui/src/space_analysis/layout.rs
- Modify: apps/cleaner-ui/src/lib.rs

**Interfaces:**
- Produces: DirectoryEntry, DirectoryPage, ScanIndex::directory_page, ScanIndex::files_under, Metric, MapPreview, Tile, project_directory, layout_treemap, and marked_cleanup_candidates.

- [ ] **Step 1: Add failing bounded-query tests**

Add tests with 100 direct files:

~~~rust
fn populated_directory_index(file_count: usize) -> ScanIndex {
    let root = normalize_path(r"C:\Fixture").unwrap();
    let mut index = ScanIndex::new(root);
    for number in 0..file_count {
        index.add_file(FileRecord::new(
            normalize_path(&format!(r"C:\Fixture\file-{number:03}.bin")).unwrap(),
            index.root_directory_id(),
            FileIdentity { volume_serial: 1, file_index: number as u128 + 1 },
            number as u64 + 1,
            number as u64 + 1,
        ));
    }
    index
}

#[test]
fn directory_page_is_sorted_and_bounded() {
    let index = populated_directory_index(100);
    let page = index.directory_page(index.root_directory_id(), 80, 80, None);
    assert_eq!(page.total, 100);
    assert_eq!(page.offset, 80);
    assert_eq!(page.entries.len(), 20);
}

#[test]
fn directory_filter_searches_beyond_the_current_page() {
    let index = populated_directory_index(100);
    let page = index.directory_page(
        index.root_directory_id(), 0, 80, Some("file-099")
    );
    assert_eq!(page.total, 1);
    assert_eq!(page.entries[0].name, "file-099.bin");
}
~~~

Run: cargo test -p domain scan_index::tests::directory_ -- --nocapture

Expected: FAIL because the APIs do not exist.

- [ ] **Step 2: Implement the bounded query API**

Add:

~~~rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryEntry {
    pub name: String,
    pub path: NormalizedPath,
    pub directory_id: Option<DirectoryId>,
    pub file_identity: Option<FileIdentity>,
    pub logical_size: u64,
    pub allocated_size: u64,
    pub last_write_time: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryPage {
    pub entries: Vec<DirectoryEntry>,
    pub offset: usize,
    pub total: usize,
}
~~~

Implement:

~~~rust
pub fn directory_page(
    &self,
    directory: DirectoryId,
    offset: usize,
    limit: usize,
    query: Option<&str>,
) -> DirectoryPage;

pub fn files_under(
    &self,
    directory: DirectoryId,
    limit: usize,
) -> Vec<FileRecord>;
~~~

Clamp page limits to 1..=80, search every direct child, place directories before files, sort names case-insensitively, and clone only returned records. files_under returns largest records first and no more than limit.

- [ ] **Step 3: Add failing projection and geometry tests**

Add the same populated_directory_index helper to the Space Analysis model test
module, using its imported domain types, then add:

~~~rust
#[test]
fn preview_is_bounded_and_preserves_weight() {
    let index = populated_directory_index(500);
    let preview = project_directory(
        &index,
        index.root_directory_id(),
        Metric::Logical,
        240,
    );
    assert!(preview.nodes.len() <= 240);
    assert_eq!(preview.total_weight, preview.nodes.iter().map(|n| n.weight).sum());
}

#[test]
fn tiles_are_finite_and_inside_the_viewport() {
    let index = populated_directory_index(20);
    let preview = project_directory(
        &index,
        index.root_directory_id(),
        Metric::Logical,
        240,
    );
    let tiles = layout_treemap(&preview, 1200.0, 700.0);
    assert!(tiles.iter().all(|t| t.x.is_finite()
        && t.y.is_finite()
        && t.width >= 0.0
        && t.height >= 0.0
        && t.x + t.width <= 1200.01
        && t.y + t.height <= 700.01));
}
~~~

Run: cargo test -p cleaner-ui space_analysis --lib

Expected: FAIL because the model is missing.

- [ ] **Step 4: Implement the bounded Disktree-derived model**

Define:

~~~rust
pub const MAP_NODE_LIMIT: usize = 240;
pub const DIRECTORY_PAGE_SIZE: usize = 80;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Metric { Logical, Allocated, Files }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileCategory {
    Code, Git, Cache, Build, Toolchain, Media, Document, Archive, Other,
}

#[derive(Clone, Debug)]
pub struct SpaceNode {
    pub path: NormalizedPath,
    pub name: String,
    pub directory: Option<DirectoryId>,
    pub file_identity: Option<FileIdentity>,
    pub weight: u64,
    pub aggregate: bool,
    pub category: FileCategory,
}

#[derive(Clone, Debug)]
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

#[derive(Clone, Debug)]
pub struct SpaceAnalysisSnapshot {
    pub directory: DirectoryId,
    pub page: DirectoryPage,
    pub preview: MapPreview,
    pub selection: Option<AnalysisSelection>,
    pub marked: BTreeSet<NormalizedPath>,
    pub filter: String,
    pub page_number: usize,
    pub zoom: f32,
    pub pan: (f32, f32),
}
~~~

Port only Disktree's platform-independent classification and squarified layout. Preserve Rusten paths and identities. Aggregate omitted weight into one noninteractive node. Return finite, nonnegative geometry for empty and zero-weight input. marked_cleanup_candidates must call existing explicit_file_candidate and prepare_cleanup_plan_with_explicit, never a deletion API.

- [ ] **Step 5: Verify and commit**

Run:

~~~powershell
cargo test -p domain
cargo test -p cleaner-ui space_analysis --lib
~~~

Expected: PASS.

~~~bash
git add crates/domain apps/cleaner-ui/src/space_analysis apps/cleaner-ui/src/lib.rs
git commit -m "feat(ui): add bounded space analysis model"
~~~

---

### Task 3: Extract The Renderer-Independent Controller

**Files:**
- Create: apps/cleaner-ui/src/controller.rs
- Modify: apps/cleaner-ui/src/state.rs
- Modify: apps/cleaner-ui/src/commands.rs
- Modify: apps/cleaner-ui/src/runtime.rs
- Modify: apps/cleaner-ui/src/lib.rs
- Modify: apps/cleaner-ui/src/app.rs

**Interfaces:**
- Produces: ControllerConfig, UiController, UiSnapshot, WorkerMessage, dispatch, drain, snapshot, and scan_index without GPUI or NSH types.

- [ ] **Step 1: Write failing controller tests**

~~~rust
#[test]
fn navigation_does_not_need_a_renderer() {
    let mut controller = UiController::with_config(ControllerConfig {
        initial_scan_root: normalize_path(r"C:\").unwrap(),
        data_root: std::env::temp_dir().join("rusten-controller-navigation"),
    });
    controller.dispatch(UiCommand::Navigate(ViewId::Recovery));
    assert_eq!(controller.snapshot().state.selected_view, ViewId::Recovery);
}

#[test]
fn worker_failure_becomes_visible_state() {
    let mut controller = UiController::with_config(ControllerConfig {
        initial_scan_root: normalize_path(r"C:\").unwrap(),
        data_root: std::env::temp_dir().join("rusten-controller-failure"),
    });
    controller.apply(WorkerMessage::ScanFailed("fixture failure".into()));
    assert_eq!(controller.snapshot().error.as_deref(), Some("fixture failure"));
}
~~~

Run: cargo test -p cleaner-ui controller::tests --lib

Expected: FAIL because UiController does not exist.

- [ ] **Step 2: Move workflow ownership out of CleanerApp**

Create:

~~~rust
#[derive(Clone, Debug)]
pub struct UiSnapshot {
    pub state: AppState,
    pub settings: SettingsModel,
    pub space_analysis: SpaceAnalysisSnapshot,
    pub cleanup_confirmation: Option<CleanupConfirmation>,
    pub cleanup_progress: Option<CleanupProgressView>,
    pub recovery_items: Vec<RecoveryItem>,
    pub recovery_quota: BackupQuota,
    pub error: Option<String>,
}

pub enum WorkerMessage {
    ScanProgress(ScanProgress),
    ScanCompleted(ScanReport),
    ScanFailed(String),
    CleanupProgress(CleanupProgress),
    CleanupCompleted(CleanupResult),
    CleanupFailed(String),
    RecoveryLoaded(RecoverySnapshot),
}

pub struct ControllerConfig {
    pub initial_scan_root: NormalizedPath,
    pub data_root: PathBuf,
}
~~~

Implement:

~~~rust
impl UiController {
    pub fn new() -> Self;
    pub fn with_config(config: ControllerConfig) -> Self;
    pub fn dispatch(&mut self, command: UiCommand);
    pub fn drain(&mut self) -> bool;
    pub fn snapshot(&self) -> UiSnapshot;
    pub fn scan_index(&self) -> Option<&ScanIndex>;
}
~~~

Move scan generation/cancellation, cleanup plan/run, selected paths, settings persistence, recovery loading, and worker message application from CleanerApp. Keep runtime helpers and all validation. Standard worker channels carry WorkerMessage; renderer context objects never cross them.

SpaceAnalysisSnapshot contains the current directory, bounded DirectoryPage,
MapPreview, selection, marked normalized paths, filter, page, zoom, and pan.
ControllerConfig::default resolves the current production roots.
with_config changes only scan and persistence roots; it does not change cleanup
validation or expose a direct filesystem mutation operation.

- [ ] **Step 3: Keep the old app functional during migration**

Replace business fields in the GPUI CleanerApp with UiController. Existing listeners call dispatch. One existing foreground task calls drain and notifies GPUI only when it returns true. Rendering consumes snapshot. Move existing non-render tests from app.rs to controller.rs without weakening assertions.

- [ ] **Step 4: Verify and commit**

Run:

~~~powershell
cargo test -p cleaner-ui --lib
cargo test -p cleaner-ui controller::tests --lib
cargo build -p cleaner-ui
~~~

Expected: PASS, including existing cleanup plan, path scope, state, progress, and identity tests.

~~~bash
git add apps/cleaner-ui/src
git commit -m "refactor(ui): extract renderer-independent controller"
~~~

---

### Task 4: Build The NSH Shell And Five Standard Pages

**Files:**
- Replace: apps/cleaner-ui/src/nsh_probe.rs with apps/cleaner-ui/src/app.rs
- Modify: apps/cleaner-ui/src/lib.rs
- Modify: apps/cleaner-ui/src/theme.rs
- Create: apps/cleaner-ui/src/components.rs
- Create: apps/cleaner-ui/src/pages/mod.rs
- Create: apps/cleaner-ui/src/pages/home.rs
- Create: apps/cleaner-ui/src/pages/cleanup.rs
- Create: apps/cleaner-ui/src/pages/large_files.rs
- Create: apps/cleaner-ui/src/pages/recovery.rs
- Create: apps/cleaner-ui/src/pages/settings.rs
- Create: apps/cleaner-ui/tests/layout.rs
- Create: apps/cleaner-ui/tests/ui.rs

**Interfaces:**
- Consumes: UiController snapshots and commands.
- Produces: App, AppSignals, six-route shell, five complete standard pages, shared dialogs, and semantic theme.

- [ ] **Step 1: Add failing shell and route tests**

Before starting the engine, set RUSTEN_TEST_SCAN_ROOT and
RUSTEN_TEST_DATA_ROOT to a read-only fixture and isolated state directory.
ControllerConfig::default honors these variables only in debug/test builds.
Start nsh_testing::Headless with a 1180x760 EngineConfig and
Root::of::<cleaner_ui::App>(). Assert:

~~~rust
for label in ["首页", "空间分析", "垃圾清理", "大文件", "恢复", "设置"] {
    ui.wait_for(&format!("label:{label}")).unwrap();
}
ui.press("label:垃圾清理").unwrap();
ui.wait_for("label:扫描").unwrap();
ui.press("label:恢复").unwrap();
ui.wait_for("text:恢复配额").unwrap();
ui.press("label:设置").unwrap();
ui.wait_for("label:自动清理").unwrap();
~~~

Run: cargo test -p cleaner-ui --test ui -- --nocapture

Expected: FAIL on probe content.

- [ ] **Step 2: Implement theme, signals, and shell**

Use these semantic colors:

~~~rust
pub const BACKGROUND: u32 = 0x09090b;
pub const PANEL: u32 = 0x111114;
pub const SURFACE: u32 = 0x19191e;
pub const BORDER: u32 = 0x2a2a31;
pub const TEXT: u32 = 0xf4f4f5;
pub const MUTED: u32 = 0xa1a1aa;
pub const ACCENT: u32 = 0x3b82f6;
pub const WARNING: u32 = 0xf59e0b;
pub const DANGER: u32 = 0xef4444;
~~~

AppSignals owns a Signal<UiSnapshot>, route, and modal state. One NSH async task calls controller.drain and replaces the snapshot only on change. No render function enumerates files.

Implement #[component] App with fixed navigation, compact header, flexible content, bottom task status, and one modal host. Collapse ViewId::Dashboard into Home. Use nsh/base Button, Checkbox, Switch, dialog, tooltip, and text input behavior. Body text is at least 13 px; icon controls have accessible labels and tooltips; focus is visibly at least 2 px.

- [ ] **Step 3: Implement Home and Large Files**

Home displays disk totals, scan state, estimated cleanable bytes, grouped summaries, Scan, Pause/Resume, Cancel, and Quick Clean Preview.

Large Files displays bounded/virtual rows with stable height, name, path, size, and modified time. Selection dispatches ToggleLargeFileSelection(FileIdentity). Hover and selection must not resize rows.

- [ ] **Step 4: Implement Cleanup, Recovery, and Settings**

Cleanup renders collapsible source/risk groups, group/item selection, shared confirmation, progress, Stop, and at most 100 issue rows. Disable conflicting selection during plan creation or execution. Preserve existing system-storage warnings.

Recovery renders recoverable operations, quota, expiry, corrupt-record count, restore confirmation, and purge confirmation.

Settings renders only current SettingsModel fields using Switch controls. A failed save restores the prior value and supplies an inline error.

- [ ] **Step 5: Add layout assertions**

Observe NSH mount frames at 900x620, 1180x760, and 1600x1000. Assert navigation, header, content, and status regions have positive size; meaningful frames remain within the viewport; long fixture text remains contained; modal close returns focus to the invoking control.

- [ ] **Step 6: Verify and commit**

Run:

~~~powershell
cargo test -p cleaner-ui --test ui -- --nocapture
cargo test -p cleaner-ui --test layout -- --nocapture
cargo test -p cleaner-ui --lib
cargo build -p cleaner-ui --bin cleaner-ui-nsh
~~~

Expected: PASS and a visible five-page-plus-placeholder Space Analysis shell.

~~~bash
git add apps/cleaner-ui/src apps/cleaner-ui/tests
git commit -m "feat(ui): migrate cleaner workflows to NSH"
~~~

---

### Task 5: Implement The Disktree-Style Space Analysis View

**Files:**
- Create: apps/cleaner-ui/src/space_analysis/view.rs
- Modify: apps/cleaner-ui/src/space_analysis/mod.rs
- Modify: apps/cleaner-ui/src/controller.rs
- Modify: apps/cleaner-ui/src/commands.rs
- Modify: apps/cleaner-ui/src/app.rs
- Modify: apps/cleaner-ui/tests/ui.rs
- Modify: apps/cleaner-ui/tests/layout.rs

**Interfaces:**
- Consumes: MapPreview, Tile, bounded directory pages, selection, and shared cleanup dialogs.
- Produces: breadcrumbs, paging, filter, selection, pan/zoom/fit, marking, review, and safe cleanup handoff.

- [ ] **Step 1: Add failing explorer tests**

Use a fixture containing folder/nested.bin, victim.bin, and keep.bin:

~~~rust
ui.press("label:空间分析").unwrap();
ui.wait_for("label:选择 victim.bin").unwrap();
ui.press("label:选择 victim.bin").unwrap();
ui.press("label:标记").unwrap();
ui.wait_for("label:取消标记").unwrap();
ui.press("label:审查 1 个标记项目").unwrap();
ui.wait_for("text:移动到回收站？").unwrap();
~~~

Also test selected-folder Enter, parent breadcrumb, filter beyond page one, previous/next paging, Ctrl+Plus, Ctrl+Minus, Ctrl+0, arrow selection, Space mark, Escape back, and Alt+Up.

Run: cargo test -p cleaner-ui --test ui space_analysis -- --nocapture

Expected: FAIL because the view is absent.

- [ ] **Step 2: Implement explorer UI**

Render path input, Rescan, Parent, breadcrumbs, metric and depth menus, a 240-node positioned treemap, and an 80-row sidebar page. Category color is accompanied by labels/borders/check state. Aggregate nodes are labeled and expose no mark action.

Enable Logical for every completed or partial scan. Enable Allocated and Files
only when the ScanIndex projection reports reliable nonmissing inputs; otherwise
render the menu item disabled with an accessible reason and do not rescan.

Implement click selection, selected-folder re-entry, pointer pan, zoom clamped to 0.5..=4.0, fit, arrow selection, Enter, Space, Escape, and Alt+Up. Use 320 ms insert/move/resize and 160 ms removal transitions only when reduced motion is off.

The sidebar displays path, type, logical/allocated bytes, modification time, and direct children. Filtering searches all direct children, not just the displayed page.

- [ ] **Step 3: Connect marks to normal cleanup planning**

Add:

~~~rust
pub enum AnalysisCommand {
    SetDirectory(DirectoryId),
    SetSelection(Option<AnalysisSelection>),
    ToggleMark(NormalizedPath),
    ReviewMarks,
    SetPage(usize),
    SetFilter(String),
}
~~~

Add UiCommand::Analysis(AnalysisCommand). AnalysisCommand::ReviewMarks creates
explicit recoverable candidates and stores the normal CleanupPlan. It never
executes cleanup. The shared confirmation and progress UI performs execution.

- [ ] **Step 4: Verify Space Analysis layout and behavior**

Run:

~~~powershell
cargo test -p cleaner-ui space_analysis --lib
cargo test -p cleaner-ui --test ui -- --nocapture
cargo test -p cleaner-ui --test layout -- --nocapture
~~~

Expected: PASS at all three viewports with nonzero toolbar/map/sidebar/status frames and contained longest-path text.

- [ ] **Step 5: Commit**

~~~bash
git add apps/cleaner-ui/src/space_analysis apps/cleaner-ui/src/controller.rs apps/cleaner-ui/src/commands.rs apps/cleaner-ui/src/app.rs apps/cleaner-ui/tests
git commit -m "feat(ui): add NSH space analysis explorer"
~~~

---

### Task 6: Cut Over Production And Remove GPUI

**Files:**
- Modify: apps/cleaner-ui/src/main.rs
- Delete: apps/cleaner-ui/src/bin/cleaner-ui-nsh.rs
- Delete: GPUI-only files in apps/cleaner-ui/src/components/
- Delete: apps/cleaner-ui/src/metrics.rs if unused
- Modify: apps/cleaner-ui/Cargo.toml
- Modify: Cargo.toml
- Modify: Cargo.lock
- Create: scripts/test-cleaner-ui.ps1

**Interfaces:**
- Produces: cleaner-ui.exe as the only UI executable with no direct GPUI dependency.

- [ ] **Step 1: Add a failing dependency guard**

~~~powershell
param(
    [switch]$DependencyCheckOnly,
    [switch]$SkipBuild,
    [string]$EvidenceDir = 'target/ui-acceptance'
)

$tree = cargo tree -p cleaner-ui --depth 1
if ($tree -match '(?m)^[^\r\n]*(gpui|gpui-component) v') {
    throw 'cleaner-ui still directly depends on GPUI'
}
if ($DependencyCheckOnly) { exit 0 }
~~~

Run: pwsh -File scripts/test-cleaner-ui.ps1 -DependencyCheckOnly

Expected: FAIL while direct GPUI dependencies remain.

- [ ] **Step 2: Replace main and remove GPUI**

Move the proven Windows launcher into main.rs. Return ExitCode::FAILURE and log a clear message on initialization error. Preserve windows_subsystem and the embedded icon build script.

Delete the migration binary and GPUI-only rendering/list/drag/cache files. Remove gpui and gpui-component from cleaner-ui. Remove workspace declarations only if no other member uses them. Regenerate Cargo.lock; do not hand-edit it.

- [ ] **Step 3: Verify production cutover**

Run:

~~~powershell
pwsh -File scripts/test-cleaner-ui.ps1 -DependencyCheckOnly
cargo build -p cleaner-ui
cargo build -p cleaner-ui --release
cargo test -p cleaner-ui --lib --tests
~~~

Expected: PASS. Both target\debug\cleaner-ui.exe and target\release\cleaner-ui.exe show NSH.

- [ ] **Step 4: Commit**

~~~bash
git add -A Cargo.toml Cargo.lock apps/cleaner-ui scripts/test-cleaner-ui.ps1
git commit -m "refactor(ui): replace GPUI with NSH"
~~~

---

### Task 7: Test Real Scan, Cleanup, Identity Rejection, And Recovery

**Files:**
- Create: apps/cleaner-ui/tests/fixture.rs
- Modify: apps/cleaner-ui/tests/ui.rs
- Modify: apps/cleaner-ui/src/controller.rs
- Modify: apps/cleaner-ui/src/runtime.rs
- Modify: apps/cleaner-ui/Cargo.toml

**Interfaces:**
- Produces: UiFixture, destructive path guard, deterministic expected totals, and a production-root headless workflow.

- [ ] **Step 1: Implement and test the fixture guard**

~~~rust
pub struct UiFixture {
    pub root: PathBuf,
    pub tree: PathBuf,
    pub state: PathBuf,
    pub expected_files: u64,
    pub expected_logical_bytes: u64,
}

impl UiFixture {
    pub fn create() -> Self;
    pub fn assert_owned_path(&self, path: &Path);
}
~~~

Use .nsh/test-data/rusten-ui-<pid>-<nanoseconds>. Write known nested text, binary, cache, large, victim, changed, and keep files. assert_owned_path canonicalizes both paths, rejects equality with the fixture parent, and requires the candidate to start with the canonical fixture root. Drop removes only the validated fixture root.

Test acceptance of tree/victim.bin and rejection of C:\, user profile, workspace root, unresolved paths, and a sibling with the same string prefix.

Run: cargo test -p cleaner-ui --test fixture -- --nocapture

Expected: PASS. Do not run destructive workflow tests if this fails.

- [ ] **Step 2: Inject roots without adding a deletion bypass**

Allow tests to inject scan and persistence roots before UiController starts. The test must use the production scanner, cleanup plan, revalidation, Recycle Bin or backup action, operation log, and recovery service. Do not add a direct remove_file test command.

- [ ] **Step 3: Drive and reconcile the real workflow**

Through accessible UI actions:

1. Start scan and wait for Complete.
2. Assert displayed file count, logical bytes, directories, category sizes, large-file rows, and treemap labels match fixture expectations.
3. Verify Pause, Resume, Cancel, Rescan, and missing-path error display.
   Where the Windows test account permits a deterministic denied path, also
   verify the partial scan keeps successful results and reports the skipped
   path. If ACL setup is unavailable, record this row as unverified so final
   acceptance cannot silently count it as a pass.
4. Mark victim.bin, review, confirm cleanup, and wait for completion.
5. Verify victim.bin left its original path and keep.bin remains byte-for-byte unchanged.
6. Replace changed.bin after preview and verify SkippedChanged while the replacement remains.
7. Enable backup before preview so victim.bin uses BackupThenDelete. Verify the
   operation is recorded recoverable, restore it through the visible Recovery
   page, and verify its original path and exact bytes.
8. Separately exercise one Shell Recycle Bin candidate and verify the source
   leaves its original path and the operation is reported recoverable; do not
   represent Shell-managed restoration as an in-app restore.
9. Restart against the same isolated state root and verify settings and recovery records persist.

Print the exact fixture root and expected/observed totals.

- [ ] **Step 4: Run serially and commit**

Run: cargo test -p cleaner-ui --test ui -- --nocapture --test-threads=1

Expected: PASS.

~~~bash
git add apps/cleaner-ui/tests apps/cleaner-ui/src/controller.rs apps/cleaner-ui/src/runtime.rs apps/cleaner-ui/Cargo.toml
git commit -m "test(ui): cover real scan cleanup and recovery"
~~~

---

### Task 8: Native Windows Acceptance And Final Verification

**Files:**
- Modify: scripts/test-cleaner-ui.ps1
- Create: docs/testing/nsh-ui-acceptance.md
- Runtime output only: target/ui-acceptance/
- Modify source files only when a check exposes a defect.

**Interfaces:**
- Produces: repeatable commands, screenshots, hashes, logs, and scan/cleanup reconciliation evidence.

- [ ] **Step 1: Complete the acceptance script**

The normal path runs these commands, stops on the first nonzero result, and writes logs under the evidence directory:

~~~powershell
cargo fmt --all -- --check
cargo test --workspace -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
cargo build -p cleaner-ui
cargo build -p cleaner-ui --release
cargo test -p cleaner-ui --test ui -- --nocapture --test-threads=1
~~~

It prints debug/release executable paths and hashes plus the fixture root. It never deletes outside the validated fixture directory.

- [ ] **Step 2: Document and execute the native matrix**

docs/testing/nsh-ui-acceptance.md requires:

- Debug and release launches with nonblank window/canvas checks.
- Screenshots of Home, Space Analysis, Cleanup, Large Files, Recovery, and Settings.
- 900x620, 1180x760, and 1600x1000 checks for overlap, clipping, overflow, hidden commands, and bad scrolling.
- Mouse and keyboard selection, focus, dialog focus return, zoom, pan, fit, breadcrumbs, paging, and reduced motion.
- Visible scan totals reconciled with fixture expectations.
- UI-confirmed cleanup, selected/unselected filesystem verification, changed-identity rejection, recovery record/restore result, and restart persistence.

Launch the debug executable and drive the whole matrix through the Windows UI. Repeat launch, scan, and confirmation-critical checks with release. Save screenshots and a summary containing hashes, fixture root, viewport, expected/observed totals, cleanup result, recovery result, and image names under target/ui-acceptance.

Expected: every required row is PASS. Missing evidence is a failure.

- [ ] **Step 3: Run final automated verification**

~~~powershell
pwsh -File scripts/test-cleaner-ui.ps1
cargo tree -p cleaner-ui --depth 1
git diff --check
git status --short
~~~

Expected: all commands pass, cleaner-ui has no direct GPUI dependency, and generated evidence is not staged.

- [ ] **Step 4: Fix failures and re-run the entire affected level**

For a unit failure, rerun the focused test then workspace tests. For a headless UI failure, rerun that test and all cleaner-ui tests. For a native visual or workflow failure, rebuild debug and release and repeat the complete native matrix so evidence hashes match final binaries.

- [ ] **Step 5: Commit only actual fixes or procedure files**

~~~bash
git add scripts/test-cleaner-ui.ps1 docs/testing/nsh-ui-acceptance.md
git commit -m "test(ui): add native NSH acceptance procedure"
~~~

If verification changes source, stage those exact files and use:

~~~bash
git commit -m "fix(ui): resolve NSH acceptance findings"
~~~

Do not create an empty commit. Final reporting must include commands and outcomes, executable hashes, fixture root, expected/observed scan totals, cleanup and recovery results, screenshot locations, and remaining risks. Do not claim completion while any required item is unverified.

