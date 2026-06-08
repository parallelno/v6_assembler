# Build Process

`scripts/build.bat` is the local build helper for the workspace.

## What it does

- Runs `cargo build --release`

## When to use it

Use the script when you want a release build of the workspace without packaging or tagging a release.

## Notes

The script is intentionally minimal so it mirrors the command used in the release pipeline.