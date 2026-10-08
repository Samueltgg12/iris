# CI mode overlay path is /tmp/iris-ci-PID-scsiID.overlay

**Keywords:** ci,overlay,scratch,/tmp,iris-ci,wd33c93a,cow,snapshot,debugging
**Category:** snapshot

# CI Mode Overlay Path is /tmp-Based, Not Image-Sibling

When iris is invoked with `--ci`, the COW overlay file does NOT live next to the base image (`<base>.overlay`). It goes to `<host temp dir>/iris-ci-<pid>-scsi<id>.overlay` (`std::env::temp_dir()`: `$TMPDIR` or `/tmp` on Unix, `%TEMP%` on Windows; a literal `/tmp` broke Windows, where it resolves to a nonexistent `\tmp` on the current drive).

CHD disks don't use this overlay at all: `ChdHd` writes a `.diff.chd` beside the base instead, whatever `--ci` says. This isolates concurrent CI runs from each other and from any interactive session sharing the same base image.

## Where it's set
`src/machine.rs:197`:
```rust
let ci_overlay = std::env::temp_dir()
    .join(format!("iris-ci-{}-scsi{}.overlay", ci_pid, id))
    .to_string_lossy()
    .into_owned();
hpc3.add_scsi_device_with_overlay(id as usize, &path, dev.cdrom, discs, dev.overlay, &ci_overlay)
```

`src/dev/wd33c93a.rs:255-258` honors the override:
```rust
let overlay_path = overlay_path_override
    .map(|s| s.to_string())
    .unwrap_or_else(|| format!("{}.overlay", path));
```

## Implications
- `rm -f irix65_4g.raw.overlay` before launching `--ci` is a no-op.
- To inspect the live overlay during a `--ci` run, find it via `lsof -p <iris-pid> | grep overlay`.
- After the iris process exits, the CI overlay file remains under `/tmp` until the next reboot or manual cleanup.
- `save_snapshot` correctly captures the CI overlay regardless of path (it routes through `cow_disk::export_overlay`, which uses `self.overlay_path`).

## Verification
```
lsof -p $(pgrep -f 'target/release/iris.*--ci') | grep overlay
```
Should show: `/private/tmp/iris-ci-<pid>-scsi1.overlay`
