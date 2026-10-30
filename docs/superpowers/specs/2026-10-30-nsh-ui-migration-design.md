# Rusten NSH UI Migration Design

Date: 2026-10-30
Status: Approved design, pending implementation plan

## Goal

Replace the complete `cleaner-ui` rendering layer with Longbridge NSH while
preserving Rusten's existing application, domain, persistence, and Windows
platform behavior. The Space Analysis page will use the visual structure and
interaction model of `huacnlee/disktree`, adapted to Rusten's existing scan
index and cleanup safety boundaries.

The migration is complete only after the native Windows interface, displayed
scan results, cleanup flow, and recovery flow pass automated and hands-on
end-to-end verification.

## Scope

### Included

- Replace the GPUI application shell and every current page with Rust NSH
  components.
- Keep the current Windows product behavior for scanning, cleanup, recovery,
  settings, progress reporting, cancellation, and persistence.
- Port Disktree's bounded treemap layout and desktop interaction model to the
  Space Analysis page.
- Add native Windows UI verification using isolated, known filesystem fixtures.
- Remove direct `gpui` and `gpui-component` dependencies from `cleaner-ui` once
  all replacement views work.

### Excluded

- TSX, QuickJS, runtime-delivered screens, plugins, or hot-update support.
- macOS, Linux, web, or mobile packaging.
- Disktree's scanner, permanent-delete implementation, and empty-trash action.
- A separate touch layout or a light theme.
- Changes to cleanup policy or expansion of the set of files Rusten may clean.

## Dependency Strategy

`cleaner-ui` will be a standalone `nsh-platform-windows` application. It will
not embed NSH inside the current GPUI window and will not retain a transitional
GPUI host in the final state.

The initial implementation target is NSH `v0.3.0`, matching Disktree `0.1.6`.
NSH dependencies must be pinned consistently in `Cargo.toml` and `Cargo.lock`.
The first implementation checkpoint is a minimal Windows debug and release
window build. Large-scale page migration must not start until this checkpoint
passes on the repository's Rust toolchain.

Disktree itself is not added as a dependency. Its current package declares
macOS and Linux platform integrations rather than a Windows application target.
Only the relevant MIT-licensed layout and interaction ideas or suitably adapted
source are brought into Rusten; NSH remains under its Apache-2.0 license.

## Architecture

The NSH root `App` component owns the window shell, navigation, global theme,
task status, and modal presentation. Six page components cover Home, Space
Analysis, Cleanup, Large Files, Recovery, and Settings.

An intentionally small UI state layer exposes NSH signals and translates user
actions into the existing application commands. It does not contain scanning,
cleanup, recovery, or persistence policy.

The dependency direction remains:

```text
NSH control or keyboard action
  -> UI command
  -> application service
  -> domain / platform-windows / persistence
  -> background message
  -> NSH signals
  -> page render
```

Filesystem enumeration, hashing, cleanup, recovery, and persistence remain off
the UI thread. Background work reports structured progress and terminal results
through channels. Signal updates occur on the NSH application thread.

Existing safety ownership is unchanged:

- `application` builds and executes cleanup plans.
- `domain` owns normalized paths, file identities, candidates, and risk.
- `platform-windows` owns permissions, identity revalidation, and Recycle Bin
  operations.
- `persistence` owns settings, operation records, and recovery metadata.

## Application Shell And Visual System

The product is a dense Windows maintenance tool rather than a marketing
interface. The shell uses a fixed navigation rail, a compact page header with
the primary action, a flexible content area, and a bottom task/status region.
At narrower supported window sizes, content reflows without hiding destructive
actions or their labels.

The visual language follows Disktree's dark work surface: black and neutral
dark-gray layers, restrained dividers, and a limited set of vivid category
colors. Amber indicates warning or pending removal; red is reserved for
destructive actions and failures. State is never communicated by color alone.

Windows system fonts are used, with body text no smaller than 13 px and tabular
figures for sizes, counts, and progress. Controls use the behavior and semantic
structure supplied by `nsh/base`. Icon-only controls have visible tooltips and
accessible names. Keyboard focus is always visible.

Dialogs share one focus-managed presentation for scan cancellation, cleanup
review, restore confirmation, and dangerous operations. Loading, empty,
partial, paused, canceled, failed, and completed states are represented
explicitly. Motion is short and functional and respects the Windows reduced
motion preference.

The minimum supported OS is Windows 10 version 1809, aligned with the declared
NSH Windows backend support.

## Page Design

### Home

Show disk capacity, available space, estimated cleanable space, current scan
state, and grouped cleanup summaries. Scanning and quick-clean preview remain
the primary actions. No promotional panels or unrelated dashboard features are
introduced.

### Cleanup

Show cleanup candidates in collapsible source and risk groups. Group and item
selection remain available. Before execution, show the exact selected paths,
estimated bytes, and relevant recovery or irreversible-risk wording. Execution
uses the existing cleanup plan and confirmation flow.

### Large Files

Use a bounded or virtualized table with name, path, size, and modification time
when available. Selection enters the same cleanup review pipeline used by Space
Analysis. Large-file cleanup does not gain a separate deletion implementation.

### Recovery

Show recoverable operations, quota consumption, expiry, and corrupt-record
warnings. Restore and purge operations keep their current service validation
and persistence semantics.

### Settings

Use native switches, menus, and path controls. Writes retain existing atomic
persistence behavior. A failed save is shown next to the affected setting and
must not appear successful.

## Space Analysis

Space Analysis reads Rusten's existing `ScanIndex`. It does not start or retain
a second Disktree index. A single completed or partially completed scan remains
the source for cleanup categories, large files, directory listings, and the
treemap.

The main area is a nested, type-colored treemap. The surrounding controls
provide path entry, rescan, parent navigation, breadcrumbs, and view options.
The sidebar shows the selected file or directory's path, type, size,
modification time when known, and direct children.

Interaction follows the approved Disktree model:

- Click selects an item; clicking the selected directory again or pressing
  Enter opens it.
- Breadcrumbs and Alt+Up navigate upward.
- Arrow keys move selection.
- Pointer drag pans the map.
- Wheel or Ctrl+Plus/Ctrl+Minus zooms; Ctrl+0 fits the map.
- Space marks or unmarks the pointed or selected item.
- Escape returns from review or details before navigating farther back.

Directory responses are bounded to 80 rows per page. Treemap previews use a
fixed node budget and aggregate undisplayed weight rather than sending the
complete index into the render tree. Aggregate tiles cannot be marked or
cleaned. Layout work that could scale with the complete scan remains outside
the UI thread.

Logical size is always available. Allocated size and file-count weighting are
enabled only when `ScanIndex` already provides reliable inputs. An unavailable
metric is disabled with a reason; the UI does not launch a second scan merely
to populate it.

Marking is reversible UI state and never deletes data. Review converts marked
leaf paths or expandable directory selections into Rusten cleanup candidates
and a normal `CleanupPlan`. Execution must then pass existing path
normalization, identity revalidation, confirmation, Recycle Bin handling,
progress reporting, and recovery recording. Disktree's permanent deletion and
empty-trash paths are deliberately not ported.

Unreadable and skipped paths are summarized without discarding successfully
indexed results. The treemap, list, and status region distinguish incomplete
scan data from a complete scan.

## Error Handling And Safety

- Background failures become structured UI states; they do not terminate the
  application with a panic.
- Partial scans remain inspectable and report skipped counts and summarized
  reasons.
- Every cleanup item is revalidated immediately before execution. A changed or
  missing identity is skipped and reported per item.
- Cleanup selection and execution are disabled while conflicting operations are
  active.
- Settings retain the existing atomic-save behavior.
- NSH initialization failure is logged clearly and exits nonzero.
- No permanent-delete bypass is added.

Destructive test helpers must resolve and normalize their fixture root before
performing work. They may only operate on descendants created for that test.
System roots, user profile roots, existing user files, and unresolved paths are
never acceptable destructive-test targets.

## Migration Sequence

1. Pin NSH and prove a minimal Windows debug and release window.
2. Introduce the NSH root component, theme, signals, command bridge, and task
   channel while leaving business services unchanged.
3. Migrate Home, Cleanup, Large Files, Recovery, and Settings.
4. Port the bounded treemap geometry and Space Analysis interactions against
   `ScanIndex`.
5. Connect marked items to the existing cleanup confirmation, revalidation,
   Recycle Bin, progress, and recovery pipeline.
6. Remove GPUI rendering, drag, scroll, and render-cache code and its direct
   dependencies.
7. Run automated tests, native Windows UI automation, screenshot review, and
   real fixture scan/cleanup/recovery verification.

Each stage must leave the workspace buildable. The minimal NSH window is a hard
compatibility gate: if it fails, dependency or toolchain compatibility is
resolved before any broad UI rewrite.

## Verification And Final Acceptance

Compilation alone is insufficient. Final completion requires every item below
to pass.

### Static And Automated Checks

- `cargo test --workspace` passes.
- `cargo clippy --workspace --all-targets` has no newly introduced warning.
- Windows debug and release builds pass.
- `cleaner-ui` has no direct GPUI dependency or GPUI rendering path.
- Existing scan, pause, resume, cancel, cleanup, recovery, and persistence tests
  continue to pass.
- An integration test covers treemap marking through `CleanupPlan` creation.
- Identity-change coverage proves a replaced target is rejected before cleanup.

### Native Interface Verification

Launch both debug and release applications on Windows and verify that their
windows become visible and nonblank. Visit Home, Space Analysis, Cleanup, Large
Files, Recovery, and Settings and capture screenshots. Inspect supported small,
default, and large window sizes for overlaps, clipping, text overflow, hidden
controls, and incoherent scroll behavior.

Exercise keyboard navigation, visible focus, dialog focus entry and return,
treemap selection, zoom, pan, fit, breadcrumbs, paging, and reduced motion.
The final evidence must include screenshots and an interaction record sufficient
to identify which executable and fixture were tested.

### Real Scan Display Verification

Create an isolated fixture with a known directory hierarchy, file count, file
types, modification metadata, and exact logical sizes. Start the scan through
the visible UI and wait for completion. Reconcile the UI's file count,
directory hierarchy, category sizes, total size, large-file rows, and treemap
contents with independently computed fixture expectations.

Also verify pause, resume, cancel, rescan, missing-path handling, and a safely
constructed inaccessible or skipped-path case where the environment supports
it. Tests must distinguish unsupported setup from application failure rather
than silently omitting the case.

### Real Cleanup And Recovery Verification

From Space Analysis, select and mark known fixture items. Enter review and
confirmation through the UI, execute real cleanup, and observe progress,
completion, released-space reporting, and issue reporting.

At filesystem level, verify selected items no longer exist at their original
paths and unselected controls are byte-for-byte unchanged. Verify that changing
a target's identity after preview causes the operation to reject that target.
Restore a test item through the visible Recovery page and verify its path and
contents. Restart the application and verify settings and recovery records
persist.

These operations use unique, runtime-created fixture paths only. Cleanup and
restore evidence records the exact fixture root so no result can be confused
with user data.

### Completion Rule

The migration is not complete while any required build, automated test, UI
display check, scan-data reconciliation, cleanup check, identity-revalidation
check, restore check, or persistence check fails. Any environment limitation
that prevents a required check is reported as an unverified acceptance item,
not treated as a pass.

## Known Risks

- NSH `v0.3.0` is relatively new; its Windows backend and the repository's Rust
  toolchain must pass the initial build gate.
- Current `ScanIndex` may not contain allocated size or all modification times;
  those displays degrade explicitly rather than expanding scan scope.
- Windows Recycle Bin behavior is system-integrated. End-to-end tests therefore
  use uniquely named fixtures and verify exact paths and contents.
- Large-index memory remains owned by the existing scan implementation. Bounded
  UI responses prevent render amplification but do not by themselves reduce the
  retained scan index.

