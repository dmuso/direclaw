# DireClaw {{TAG}}

## Install

1. Download the matching archive for your platform from this release.
2. Extract the archive: `tar -xzf direclaw-{{TAG}}-<target>.tar.gz`.
3. Move `direclaw` onto your `PATH` (for example `/usr/local/bin/direclaw`).
4. Verify install: `direclaw --help`.

## SHA256 Verification

1. Download `checksums.txt` from this release.
2. Run: `shasum -a 256 direclaw-{{TAG}}-<target>.tar.gz`.
3. Confirm the digest matches the corresponding line in `checksums.txt`.

## Known Limits

- `update apply` is intentionally unsupported in this build; use manual binary replacement.
- DireClaw v1 supports Slack as the only channel adapter.

## Notes

Release version: `{{VERSION}}`
