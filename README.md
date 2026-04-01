# v6asm

> A two-pass Intel 8080 / Z80-compatible assembler for the **Vector-06c** home computer, written in Rust.

[![CI/CD](https://github.com/parallelno/v6asm/actions/workflows/ci.yml/badge.svg)](https://github.com/parallelno/v6asm/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

---

## Overview

`v6asm` is a command-line toolchain for developing software for the **Vector-06c** (Вектор-06Ц) — a Soviet-era Z80-compatible home computer. It assembles `.asm` source files into `.rom` binaries and can optionally build bootable **FDD disk images** ready to load in an emulator.

The workspace contains two standalone CLI tools:

| Tool | Purpose |
|------|---------|
| `v6asm` | Assembler — compiles `.asm` source into a `.rom` binary |
| `v6fdd` | FDD image utility — packs files into a `.fdd` disk image |

---

## Features

- **Intel 8080 instruction set** with optional **Z80 mnemonic compatibility** (`LD`, `JP`, `CALL`, port I/O forms, etc.)
- **Two-pass assembly** with full forward-reference resolution
- **Rich expression engine** — arithmetic, bitwise, logical, shift, comparison, unary `<`/`>` (low/high byte), operator precedence
- **Preprocessor directives**: `.include`, `.macro`/`.endmacro`, `.loop`/`.endloop`, `.if`/`.endif`, `.optional`/`.endoptional`, `.incbin`, `.filesize`, `.print`, `.error`, `.setting`
- **Local labels** (`@name`) with automatic scope management
- **Mutable variables** (`.var`) alongside immutable constants (`=` / `EQU`)
- **FDD image builder** — creates or patches 820 KB disk images from the built-in `rds308.fdd` template or a custom one
- **`--init` scaffolding** — generate a ready-to-build `.asm` file from a starter template
- Prebuilt binaries for **Linux**, **Windows**, and **macOS**

---

## Installation

### Download a prebuilt release

Grab the latest archive for your platform from the [Releases](https://github.com/parallelno/v6asm/releases) page and extract it. The archive contains:

```
v6asm       (v6asm.exe on Windows)
v6fdd       (v6fdd.exe on Windows)
docs/
README.md
```

Add the extracted directory to your `PATH` or copy the binaries to a directory that is already on it.

### Build from source

Prerequisites: [Rust toolchain](https://rustup.rs/) (stable).

```bash
git clone https://github.com/parallelno/v6asm.git
cd v6asm
cargo build --release
```

Binaries are written to `target/release/v6asm` and `target/release/v6fdd`.

---

## Quick Start

```bash
# Create a new .asm file from the starter template
v6asm --init main

# Assemble it
v6asm main.asm

# Custom output path
v6asm main.asm -o out/program.rom

# Z80 mode + listing
v6asm main.asm --cpu z80 --lst
```

After a successful build you will find:

- `main.rom` — the assembled binary
- `main.lst` — optional listing file (if `--lst` is passed)

---

## v6asm — Assembler CLI

```
USAGE:
    v6asm <source.asm> [options]
    v6asm --init <name>

OPTIONS:
    -o, --output <path>   Output ROM path (default: <source>.rom)
        --cpu <cpu>       Target CPU: i8080 (default) or z80
        --rom-align <n>   ROM size alignment in bytes (default: 1)
    -q, --quiet           Suppress .print output
    -v, --verbose         Extra diagnostics
        --lst             Generate a listing file (.lst) alongside the ROM
        --init <name>     Create a new .asm file from the starter template
```

---

## v6fdd — FDD Image Utility

```
USAGE:
    v6fdd -i <file> [-i <file>...] -o <output.fdd> [-t <template.fdd>]

OPTIONS:
    -t, --template   Template FDD image to start from (uses built-in rds308 if omitted)
    -i, --input      File to add to the disk (repeatable)
    -o, --output     Output FDD image path (required)
```

Example:

```bash
v6fdd -t rds308.fdd -i myprogram.rom -i extra.dat -o out/disk.fdd
```

---

## Assembly Language Reference

Full documentation is in [`docs/assembler.md`](docs/assembler.md). The highlights are below.

### Numeric Literals

| Format | Example | Notes |
|--------|---------|-------|
| Decimal | `42`, `-5` | |
| Hex `$` | `$FF`, `$1234` | |
| Hex `0x` | `0xFF`, `0x1234` | |
| Binary `%` | `%1010`, `%11_00` | Underscores allowed |
| Binary `0b` | `0b1010` | |
| Character | `'A'`, `'\n'` | Standard C escapes |

### Comments

```asm
mvi a, $10   ; single-line (semicolon)
mvi b, $20   // single-line (double-slash)
/* multi-line
   comment */
```

### Constants and Variables

```asm
MAX_ITEMS = 32          ; immutable constant (forward refs work)
MAX_ITEMS EQU 32        ; same, alternative syntax

Counter .var 10         ; mutable variable — can be reassigned
Counter = Counter - 1
```

### Local Labels

Labels prefixed with `@` are scoped between surrounding global labels.

```asm
fill_memory:
    lxi b, 0x1000
@loop:
    mvi m, 0
    dcx b
    jnz @loop           ; resolves to the @loop above

next_routine:           ; new scope — @loop here is independent
```

### Key Directives

| Directive | Description |
|-----------|-------------|
| `.org $addr` | Set the program counter |
| `.include "file.asm"` | Inline another source file (recursive, up to 16 levels) |
| `.incbin "file"[,off[,len]]` | Embed raw bytes from a file |
| `.filesize Name, "file"` | Define a constant equal to a file's byte size |
| `.byte` / `DB` | Emit one or more bytes |
| `.word` / `DW` | Emit one or more 16-bit little-endian words |
| `.dword` / `DD` | Emit one or more 32-bit little-endian dwords |
| `.storage N[, fill]` | Reserve N bytes (write `fill` or leave uninitialized) |
| `.align N` | Pad with zeros to the next multiple of N |
| `.if expr` / `.endif` | Conditional assembly |
| `.loop N` / `.endloop` | Repeat a block N times (max 100 000) |
| `.optional` / `.endoptional` | Omit block if its symbols are never referenced |
| `.macro Name(p1,p2=default)` / `.endmacro` | Define a parameterized macro |
| `.var Name value` | Declare a mutable variable |
| `.print …` | Emit a compile-time diagnostic |
| `.error …` | Halt assembly with a fatal message |

### Example Program

```asm
            OPCODE_EI  = 0xFB
            OPCODE_RET = 0xC9

.org 0x100
start:
            lxi  sp, 0x8000
            mvi  a, OPCODE_EI
            sta  0x38
            mvi  a, OPCODE_RET
            sta  0x39
            ei
            call set_palette
end:
            di
            hlt

PALETTE_LEN = 16
set_palette:
            lxi  h, palette + PALETTE_LEN - 1
            mvi  b, 0x0F
@loop:
            mov  a, m
            out  0x0C
            dcx  h
            dcr  b
            jnz  @loop
            ret
palette:
            DB  b11_111_000, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, b00_000_000
```

---

## Workspace Structure

```
Cargo.toml              ← workspace manifest
crates/
  v6_core/              ← shared library
    src/
      assembler.rs      ← two-pass assembler orchestrator
      diagnostics.rs    ← error types and source locations
      encoding.rs       ← character encoding helpers
      expr.rs           ← expression evaluator (Pratt parser)
      fdd/              ← FDD image read/write
      instructions/     ← Intel 8080 opcode table; Z80 compat mapping
      lexer.rs          ← tokenizer
      output.rs         ← ROM binary + listing emitter
      parser.rs         ← directive/instruction parser
      preprocessor.rs   ← macro expansion, .include, .loop, .if
      project.rs        ← CPU mode types
      symbols.rs        ← symbol table (labels, consts, macros)
  v6asm/                ← assembler CLI binary
    src/
      main.rs
      templates/
        main.asm        ← embedded template for --init
  v6fdd/                ← FDD utility CLI binary
    src/
      main.rs
docs/
  assembler.md          ← full assembler language specification
```

---

## Running Tests

```bash
cargo test --workspace
```

---

## License

This project is released under the [MIT License](LICENSE).
