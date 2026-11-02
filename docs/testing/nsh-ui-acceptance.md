# NSH UI migration acceptance

Latest behavior: [cleanup performance and controls](cleanup-performance-and-controls.md).
The desktop recovery/backup and ineffective automatic-acceleration controls
were removed at the user's request. Recovery evidence below is historical.

Recorded on 2026-10-30 / 2026-10-31 (Asia/Shanghai), in the actual workspace
`F:\proj\Rusten`. The production entry point is NSH v0.3.0; running
`cargo run --bin cleaner-ui` here opens the six-page native application.
The migration is integrated into this directory, not only into the migration
worktree. Changes are uncommitted.

The latest [themes and space analysis verification](themes-and-space-analysis.md)
supersedes the earlier screenshots and executable hashes below. It records
the three reactive themes, distinct title-bar colors and grouped treemap.

The follow-up [feature parity audit](nsh-ui-feature-parity.md) compares the
pre-migration controls with their migrated commands and state. It restores the
persistent scan/cleanup toolbar, estimates, Home categories, source/directory
scopes, cleanup result details, large-file selection totals, recovery status,
and system-action confirmation warnings. Opening six pages alone was not an
adequate functional acceptance criterion.

## Automated verification

Run `pwsh -File scripts/test-cleaner-ui.ps1` from the repository root.
The script checks dependencies, formatting, fixture safety, Windows platform
regressions, workspace tests, strict Clippy, both builds, layout, and the
production component's real filesystem workflows. Evidence is under
`target/ui-acceptance`; the latest complete run is `target/cleanup-full-acceptance.log`.

| Check | Result |
| --- | --- |
| Direct GPUI dependency removed | PASS |
| Formatting | PASS |
| Workspace tests | 278 passed, 0 failed, 7 ignored across 56 suites |
| Windows input / navigation / accessibility regressions | 16 / 5 / 6 passed |
| Strict owned-workspace Clippy | PASS: `cargo clippy --workspace --all-targets --no-deps -j1 -- -D warnings` |
| Debug / Release builds | PASS |
| Owned fixture guard, production-root workflows, three-size layout | PASS |

Ignored tests are not passes. The patched upstream platform and external UFFS
path dependencies are excluded from workspace membership; platform regression
suites run separately. No dependency-wide lint suppression was added.

Automated workflow assertions use the production NSH root and actual Windows
services on a unique owned fixture:

- Scan reconciles exactly 4 files / 48 logical bytes.
- Review cancellation retains the source; backup cleanup removes only the
  selected file; restoration reproduces exact original bytes.
- Unselected files retain exact bytes. Replacing a reviewed file produces
  `SkippedChanged` and preserves the replacement.
- Backup-disabled recycle cleanup removes only the selected source. No test
  empties the system Recycle Bin.
- Settings and recovery records survive controller restart.
- A 100 MiB file appears in Large Files; cancelling its review retains it.
- On all six pages at three sizes, the scan/cleanup toolbar and estimate remain
  visible. Starting a scan from Settings yields an executable selection;
  cancel retains files, while confirming the global cleanup executes the plan.
- Switching to all selected sources retains the source detail panel. Source
  pagination, category filtering and directory boundary checks have regressions.
- Recovery after restarting at 900×620 retains the action inside the content
  area, even with a long original path. The regression failed before the fix
  with the button at x=929 pt and width=55 pt (`parity-recovery-red.log`).
  Accessibility-tree presence alone did not detect this clipping; the test
  now checks the mounted button bounds as well as completing restoration.
- Directory paging is 80 + 11; filtering reaches `item-084` beyond page one.
  Zoom, fit, pan, metric switches and invalid indexed paths are exercised.

## Native Windows verification

Orca's Windows UI Automation and screenshots drove real native windows.
`tests/native.rs::native_owned_fixture` hosts the same production shell with
isolated scan and state roots. `.owner` and normalized-descendant guards
protect fixture operations; successful exit removes the fixture.

| Binary profile | Client sizes | Native result |
| --- | --- | --- |
| Debug | 900×620, 1180×760, 1600×1000 | Six pages render and navigate; close returns exit 0 |
| Release | 900×620, 1180×760, 1600×1000 | Six pages render and navigate; close returns exit 0 |
| Production Debug | 1180×760 | Exact requested Cargo command opens NSH; six pages and exit 0 |
| Production Release | 1180×760 | Release executable opens NSH; six pages and exit 0 |

Evidence JSON records PID, HWND, window size and accessibility tree. PNG copies
and contact sheets live beside those JSON files. Final matrix filenames use
`parity-debug-900x620-final`, `parity-debug-1180x760-final`,
`parity-debug-1600x1000-final` and analogous `parity-release-<size>-final`
prefixes; native exit logs use `native-<prefix>.log`.

The earlier detailed native Debug workflow used fixture
`rusten-ui-25416-1793372791611555800` and evidence `debug-1180-*.json/png`:

- Scan: 4 files / 48 bytes / 0 warnings; Analysis renders the indexed entries.
- Mark and review a file. The confirmation is visible, with Cancel initially
  focused. Tab moves to Confirm, Shift+Tab returns to Cancel, and Return
  cancels without changing `restore-me` bytes.
- Confirm cleanup: 1 succeeded, 0 failed/skipped; selected source absent,
  `never-change` and `identity-before` files unchanged.
- Recovery page lists the 10-byte item; confirming Restore recreates exact
  `restore-me` bytes.
- Settings change writes `auto_clean=true`, `backup_enabled=true` to the
  fixture's config file.
- Add a 100 MiB file and rescan: 5 files / 104857648 bytes. Large Files shows
  the item; Escape cancels its review and retains its exact size.

The follow-up parity workflow used the 900×620 native fixture
`rusten-ui-19684-1793380665423994200` (`parity-native-*.json/png`):

- Start Scan from Settings: four files, 48 logical bytes and 64 B estimated.
- Select only the nested directory: one selected file and 16 B estimated.
- Cancel global Immediate Cleanup: all four files remain.
- Confirm global Immediate Cleanup: only `nested/readme.txt` is removed;
  `victim.bin`, `keep.bin`, and `changed.bin` retain exact original bytes.
- Result: one successful item, zero skipped/failed, 16 B released; elapsed
  time and disk-space change are shown. This rule uses Windows Recycle Bin,
  so it does not create an application backup record.
- Add a 100 MiB owned file and rescan: its path, size, date and selection
  button fit the narrow Large Files page. Six-page navigation and native
  shutdown then return `1 passed; 0 failed` and exit 0.

The final Release 900×620 backup/restore workflow used fixture
`rusten-ui-51880-1793381967574928500`. A selected 100 MiB file was backed up
and removed. The wrapped original path, item status and Restore button all
fit the Recovery page (`parity-restore-recovery.png`). Native Restore and its
confirmation recreated the file with SHA256
`20492a4d0d84f8beb1767f6616229f85d44c2827b64bdbfb260ee12fa1109e0e`.
All four unselected fixture files retained exact original bytes. The page
then showed zero recoverable items and the restored item state
(`parity-restore-restored.png`). Both the CLI workflow and native test host
exited 0; assertions are in `parity-native-restore-result.json` and
`parity-native-restore-actions.log`.

This workflow exposed and fixed three upstream Windows issues: Tab skipped
buttons, nonanimated confirmation dialogs had opacity zero, and closing the
window left the process hung in graphics resource destruction under the
Windows loader lock. The local pinned platform patch and regression evidence
are described in `vendor/nsh-platform-windows/RUSTEN-PATCHES.md`. Before the
shutdown fix, older interactive logs end with deliberately terminated test
hosts; they are diagnostic failures, not successful native test exits. Final
matrix logs explicitly require `1 passed; 0 failed` and process exit 0.

## Reproduce native checks

```powershell
$env:RUSTEN_NATIVE_SIZE='900x620' # also 1180x760 or 1600x1000
cargo test -p cleaner-ui --test native -j1 -- --ignored --nocapture --test-threads=1
cargo test -p cleaner-ui --release --test native -j1 -- --ignored --nocapture --test-threads=1
cargo run --bin cleaner-ui
```

Inspect Home, Space Analysis, Cleanup, Large Files, Recovery, Settings, then
close the title-bar window and require the native test to finish successfully.
Cleanup/restore checks must use the owned test host, not personal scan data.

## Limits and source state

Timed native Pause/Resume, a deterministic ACL-denied partial scan, and OS
reduced-motion preference changes were not exercised. Controller cancellation
and token transitions are automated tests. The latest theme checks ran on
Windows 10 build 19045; other GPU/driver combinations were not exercised.
At 1600×1000 the desktop taskbar
can overlap the window's bottom edge in captured screenshots; headless layout
assertions additionally verify full client geometry.

During the original migration, installed Git crashed (access violation -1073741819).
Dulwich verified the root's initial clean state, migration ancestry, and final
changed paths; a source whitespace scan passed. No commit or branch merge was
performed. Pre-integration source backups and a manifest are retained under
`target/pre-nsh-integration-20261030-223105`.

Final executable SHA256 values are recorded in
`target/ui-acceptance/executables.json` after the final build.

```text
debug 9431306A3EEC4D8F6EAA89552F221080134BFE29EB94766B18E69B16B12B4356
release 6DDC4E20292A184ACDE452CE51826E01EBB14BFB3CFCF5C2C05DE31C90583C69
```

Final normal acceptance script exited 0 after the graphics shutdown fix.
Both production smoke results record the canonical root executable path,
six-page navigation, and exit code 0 (`parity-production-*-final-result.json`).
Current screenshots: `target/ui-acceptance/parity-*-final-<page>.png`; earlier
contact sheets: `target/ui-acceptance/*-final-contact.png`. Detailed earlier native
workflow assertions: `native-workflow-assertions.json`. Failed diagnostic
fixture hosts were removed only after validating their exact `.owner` markers.
