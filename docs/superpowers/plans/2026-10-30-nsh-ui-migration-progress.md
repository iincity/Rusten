# NSH migration status

The migration is now integrated into the actual `F:\proj\Rusten` workspace.
`cargo run --bin cleaner-ui` launches the six-page native NSH application.
The former compatibility probe and GPUI production entry point are removed.

Current verification and limitations are recorded in
[the acceptance record](../../testing/nsh-ui-acceptance.md).
Earlier worktree-only progress is superseded by that record.

The follow-up [functional parity audit](../../testing/nsh-ui-feature-parity.md)
restores the global scan/cleanup controls and estimates, category/source
selection, result details and recovery status. Final acceptance includes
270 passing workspace tests, narrow-window recovery geometry, and native
Release backup/restore with an exact file-hash check.
