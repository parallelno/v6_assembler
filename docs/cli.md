# CLI Usage

```
v6asm <source.asm> [options]
v6asm --init <name>
```

## Arguments

| Argument | Description |
|----------|-------------|
| `<source>` | Assembly source file (`.asm`) to compile |
| `--init <name>` | Scaffold a new `.asm` file with a starter template |
| `-o`, `--output <path>` | Output ROM path (default: `<source>.rom`) |
| `--cpu <cpu>` | Target CPU: `i8080` (default) or `z80` |
| `--rom-align <n>` | ROM size alignment in bytes (default: `1`) |
| `-q`, `--quiet` | Suppress `.print` output |
| `-v`, `--verbose` | Extra diagnostics |
| `--lst` | Generate a listing file (`.lst`) alongside the ROM |
| `--symbols` | Generate a debug symbols file (`.symbols.json`) alongside the ROM |

## Examples

```bash
v6asm main.asm                        # compile, output main.rom
v6asm main.asm -o out/program.rom     # custom output path
v6asm main.asm --cpu z80 --lst        # Z80 mode + listing
v6asm main.asm --symbols              # generate debug symbols
v6asm --init main                     # create main.asm from template
```

## Output Artifacts

- `<name>.rom` — Vector 06c executable loaded by the emulator.
- `<name>.lst` — optional listing file (enabled with `--lst`) showing addresses, emitted bytes, and source lines. See [Listing File Format](listing.md) for details.
- `<name>.symbols.json` — optional debug symbols file (enabled with `--symbols`) containing source-level symbol information for debuggers and editors. Maps labels, constants, variables, macros, and functions back to source locations, and records per-line address and data mappings.
