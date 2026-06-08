# Publish Process

`scripts/publish.bat` is the local helper for creating and pushing a release tag.

## What it does

- Runs `cargo build --release`
- Computes today's tag in the form `vYYYY.MM.DD`
- Creates the tag locally
- Pushes the tag to `origin`

## When to use it

Use the script when you are ready to trigger the GitHub Actions release workflow from the current commit.

## Notes

The tag is generated from the current date at runtime, so the script always targets the day it is run.