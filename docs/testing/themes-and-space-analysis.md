# Themes and Space Analysis

The subsequent [cleanup and control audit](cleanup-performance-and-controls.md)
removes desktop recovery/backup and ineffective automatic-cleanup controls.
The recovery workflows below describe the preceding version.

The Settings theme selector offers Light (`light`), Dark (`dark`) and Mint
(`mint`). Mint is the default for existing configurations without a theme,
new installations and unknown theme values. Selection updates live views and
is saved through the existing asynchronous settings writer. A failed save
rolls settings back. Saving preserves scan settings, backup quota and retention.

Background, navigation, toolbar, buttons, inputs, text, dialogs and status
colors use the same reactive palette. Windows appearance follows the theme.
The title uses NSH's existing custom-title-bar support, with a matching
surface and readable title text. The caption has a distinct gray, charcoal
or mint shade and a fine separator in each theme. Native caption buttons, drag, double-click
maximize and resizing remain platform-controlled, including Windows 10.

Space Analysis now follows the composition of
[huacnlee/disktree](https://github.com/huacnlee/disktree), inspected from its
public `src/ui.rs`: compact path/metric/depth controls, category legend,
large treemap, adjacent directory summary/list and disk-space status. Deeper
previews group descendants into directory regions instead of mixing all
leaves in one flat layout. The existing bounded preview, paging, selection,
mark/review, navigation, zoom, pan and cleanup scope commands remain in use.
Cleanup source details and categories are below the primary explorer.

This adapts the explorer to Rusten's application navigation and three themes;
it does not reproduce unrelated Disktree deletion or platform-specific menus.
The GitHub attachment image could not be downloaded in this environment;
the upstream Rust UI source was available for comparison.

Verification evidence lives in `target/ui-acceptance`: theme contrast and
nested-region tests, `themes.rs` live color/persistence checks, existing real
cleanup/restore workflows, and native screenshots for all three themes.
`themes-native-actions.log` records native navigation and persisted choices;
caption/background pixel assertions verify the visible result.

Windows 10 rejected DWM custom caption colors in native pixel testing, so
the implementation uses NSH's custom title bar on both Windows 10 and 11.

## Verification on 2026-10-31

`pwsh -File scripts/test-cleaner-ui.ps1` exited 0. Workspace tests reported
273 passed, 0 failed and 7 ignored across 55 suites. Formatting, platform
regressions, strict Clippy, Debug/Release builds and owned-filesystem
scan/cleanup/restore checks passed. Full log: `target/theme-space-acceptance.log`.

The 1180x760 native fixture verified Mint -> Dark -> Light -> Mint, selected
theme accessibility state, live background/caption pixels, persisted config,
page navigation and nested map labels. Closing the caption ended the native
host with exit 0 and one passing test. Screenshots are
`target/ui-acceptance/themes-<theme>-<page>.png`; action results are in
`themes-native-actions.log`, with the host log in `themes-1180-native.log`.

Caption RGB values are Light `#e7ebf0`, Dark `#202127` and Mint `#d8ebdf`.
All are distinct from their page and panel backgrounds. Palette tests also
require readable text contrast on each caption.

The same native theme sequence passed at 900x620, including pixel checks,
page navigation, persistence and normal exit (`themes-900-actions.log`,
`themes-900-native.log`, `themes-900-<theme>-<page>.png`). A separate Release
900x620 fixture selected, backed up, cleaned and restored a 100 MiB file;
the restored SHA256 was
`20492a4d0d84f8beb1767f6616229f85d44c2827b64bdbfb260ee12fa1109e0e`.
All four unselected fixture files remained byte-identical. The native host
exited 0; see `themes-release-restore-actions.log` and
`parity-native-restore-result.json` in the evidence directory.

The exact root command `cargo run --bin cleaner-ui` and the built Release
executable both passed native navigation of Home, Space Analysis, Cleanup,
Large Files, Recovery and Settings, retaining scan/cleanup controls and
estimates, then exited 0. Canonical executable paths and results are recorded
in `themes-production-<debug|release>-final-result.json`. Production smoke
checks did not scan or clean personal files.

Windows 11, other GPUs and caption minimize/maximize gestures were not
retested in this follow-up. Native close and live caption colors were tested
on Windows 10 build 19045. Earlier failed diagnostic attempts are not counted
as passing evidence.
