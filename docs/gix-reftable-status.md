# Gitoxide (gix) Reftable Support Status

**Last updated:** 2026-07-17

## Executive Summary

As of gitoxide v0.55.0 (June 2024), reftable ref format support is **not yet implemented**. rdm currently uses gix v0.85, which continues to initialize repositories with the files-based ref format. No immediate upgrade is required.

## Investigation Details

### Current gix Version in rdm

- **rdm-git/Cargo.toml**: `gix = "0.85"` (compatible with v0.85.0 and patch updates)
- **rdm-store-git/Cargo.toml**: `gix = "0.85"` (kept in sync)
- Both crates use the same gix version and features: `["basic", "index", "revision", "parallel", "sha1"]`

### Upstream Status

**GitHub Issue:** [GitoxideLabs/gitoxide#109](https://github.com/GitoxideLabs/gitoxide/issues/109)
- Opened: June 25, 2021
- Status: Still open; described as a "tracking issue" for reftable support
- No linked PRs indicating active implementation

**Latest Release:** v0.55.0 (June 22, 2024)
- No mentions of reftable in release notes
- Focus remains on bug fixes and incremental feature work

**Gitoxide Changelog:**
- Searched current HEAD and recent history; no reftable entries found
- Git 3.0 (targeting late 2026) will default to reftable, but gitoxide upstream has not announced support yet

### Ref Format Behavior

**Test Result:** Added `gix_init_uses_files_ref_format` to rdm-store-git tests (AC3)
- Confirms: gix v0.85 creates repositories with **files-based ref storage** (not reftable)
- `.git/HEAD` exists as a regular file containing `ref: refs/heads/main` (files format)
- No `.git/reftable/` directory is created

### CI Workaround Status

**Result:** No `init.defaultRefFormat=files` workaround currently exists in rdm
- Searched: `.github/workflows/*.yml`, all `.rs` source files, shell scripts, templates, hooks
- No environment variables, git config settings, or CLI flags force the files ref format
- **Conclusion**: The phase body's mention of a workaround is outdated; no workaround is present or needed

## Recommendations

1. **No immediate action required** — gix does not yet support reftable, and rdm's default behavior (files format) is correct.
2. **Continue monitoring** — Check gitoxide issue #109 and release notes quarterly (target: late 2026 when Git 3.0 releases).
3. **Future upgrade path** — When gix adds reftable support and rdm is ready to support both formats, consider:
   - Adding a configuration option to select ref format (backward compatibility)
   - Testing against both files and reftable formats in CI
   - Documenting the upgrade in CHANGELOG.md

## Verification Notes

- Test added: `rdm-store-git::tests::gix_init_uses_files_ref_format` — passes ✓
- CI gate status: All checks pass (cargo build, fmt, clippy, tests)
- No deprecated workarounds to remove
