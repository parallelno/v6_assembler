# Scripts Folder

The `scripts/` folder contains small batch files for common local workflows.

## Contents

1. [Build Process](build-process.md) — builds the workspace in release mode
2. [Publish Process](publish-process.md) — builds, tags, and pushes a release tag

## Files

- [build.bat](../../scripts/build.bat) — runs `cargo build --release`
- [publish.bat](../../scripts/publish.bat) — builds, creates a date-based tag, and pushes it to `origin`