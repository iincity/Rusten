# Windows Fixture Sets

The performance fixture is generated on a real NTFS volume, never inside the source tree.

Required cases:

- 100,000 and 1,000,000 ordinary files across nested directories;
- a path longer than 260 characters and names containing non-ASCII characters;
- hard-link pairs, symbolic links, Junctions and a mount point;
- an open/locked file and an access-denied directory;
- a read-only volume and a nearly-full volume for failure tests.

Run the ignored benchmark only after selecting a disposable fixture root:

```text
cargo test -p platform-windows --test performance_scan_tests -- --ignored --nocapture
```

The fixture must be deleted manually after the run. It must not contain user data.
