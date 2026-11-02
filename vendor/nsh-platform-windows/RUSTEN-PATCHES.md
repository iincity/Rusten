# Rusten Windows platform patches

Source: longbridge/nsh v0.3.0, commit 76fa8a7f (original Apache-2.0 license
included). Cargo patches only the Windows platform crate; other NSH crates
remain pinned to the same upstream tag. This directory must be tracked.

Native acceptance exposed that upstream Windows Tab navigation only visited
text inputs. The local patch includes enabled focusable controls, activates
the focused control with Enter/Space, keeps focus inside the newest modal,
and restores button focus when a modal closes. Pointer activation records
focus, and focused text inputs handle editing keys before global shortcuts.
The existing accessibility focus/state and scroll-reveal helpers are reused.

`src/input.rs::tests::tab_stops_include_controls_and_respect_modal_scope`
failed before the patch (no controls collected) and passes after it. Input,
navigation, and accessibility unit suites are run separately from workspace
tests because this dependency is excluded from workspace membership.

Native screenshots also exposed a fully transparent nonanimated dialog: modal
insertion applies the default animation's initial fade before its explicit
`None` configuration arrives. Showing a nonanimated modal now resets fade and
translation. The regression `nonanimated_modal_resets_initial_presentation_fade`
fails with opacity 0 before the fix and passes with opacity 1 afterward.

The manifest expands upstream workspace inheritance into explicit dependencies
so the crate builds independently. The upstream font test asset and its OFL
license are local; its include path is adjusted. No runtime JS feature is
enabled by Rusten.

Remove this patch when a pinned upstream release supplies equivalent native
keyboard behavior and passes the same tests and native acceptance matrix.

Native close testing exposed a second upstream Windows lifecycle issue: the
window disappeared but `run_app` never returned. Diagnostic thread stacks
showed the last Direct2D/Direct3D device release in a thread-local destructor
under `LdrShutdownThread`, waiting inside the Intel driver. The standalone
runner now clears device caches and releases its graphics device before OLE
uninitialization and thread exit. This avoids releasing the driver under the
Windows loader lock. Embedded AppView lifecycle is unchanged. The interactive
`native_owned_fixture` test checks successful return after closing the window.

Rusten uses the upstream custom title bar API and sets NSH's appearance
when its persisted theme changes, so native inputs, menus and caption
buttons use the corresponding light/dark scheme. No additional platform
patch is needed for theme colors. Native acceptance switches all three
themes and checks caption pixels.
