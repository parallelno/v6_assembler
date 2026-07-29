# Plan: Relocatable ELF Object Output for v6asm

**TL;DR**: Add an opt-in output mode (`v6asm --emit obj` / `-f elf`) that makes
v6asm emit a **relocatable ELF32 object** (`ET_REL`, `EM_V6C = 0x8080`, RELA
relocations) instead of a fully-located `.rom`. This lets the existing `.asm`
library — with all its macros, `@local` labels, `.loop`, `.if`, and
`.optional` blocks intact — be linked by `ld.lld` together with object files
produced by the V6C LLVM/Clang toolchain. The default behavior (flat `.rom`)
is unchanged; ELF emission is a new, additive backend.

This is the "best long-term" integration path: it preserves the entire v6asm
source library as-is and removes the per-file hand-porting / drift problem of
converting `.asm` → `.s` or C inline-asm.

---

## Why this is the right approach

- **Zero source churn.** Every existing `.asm` keeps working. v6asm already
  fully expands macros, `@locals`, `.loop`, `.if`, and `.optional` *before*
  byte emission, so the ELF backend only needs to consume the already-flat
  instruction/data/symbol stream — none of those features need an LLVM
  equivalent.
- **One-time tool investment** vs. ongoing per-file porting.
- **Linker-native.** Output drops straight into the existing
  `clang --target=i8080-unknown-v6c` + `ld.lld` + `v6c.ld` pipeline.

---

## Target ELF contract (must match the V6C LLVM/lld toolchain exactly)

Verified against the consumer side in `v6llvmc`:

| Field | Value | Source of truth |
|-------|-------|-----------------|
| Class | `ELFCLASS32` | i8080 is 16-bit; LLVM writer uses `Is64Bit=false` |
| Data | `ELFDATA2LSB` (little-endian) | 8080 is little-endian |
| OS/ABI | `ELFOSABI_NONE` (0) | `MCELFObjectTargetWriter(..., OSABI=0, ...)` |
| Type | `ET_REL` | relocatable object |
| Machine | `EM_V6C = 0x8080` | `V6CFixupKinds.h`, `C:\Work\Programming\v6llvmc\lld\ELF\Arch\V6C.cpp` |
| Reloc format | **RELA** (explicit addend) | `HasRelocationAddend=true` in `C:\Work\Programming\v6llvmc\llvm\lib\Target\V6C\MCTargetDesc\V6CAsmBackend.cpp` |
| `e_flags` | `0` | no V6C flags defined |

**Relocation types** (`C:\Work\Programming\v6llvmc\llvm\lib\Target\V6C\MCTargetDesc\V6CFixupKinds.h`):

| Name | Value | Field | Meaning |
|------|-------|-------|---------|
| `R_V6C_NONE` | 0 | — | none |
| `R_V6C_8` | 1 | 1 byte | 8-bit absolute value |
| `R_V6C_16` | 2 | 2 bytes LE | 16-bit absolute address |
| `R_V6C_LO8` | 3 | 1 byte | low byte of 16-bit address |
| `R_V6C_HI8` | 4 | 1 byte | high byte of 16-bit address |

All V6C relocations are **absolute** (no PC-relative; the 8080 has no
PC-relative addressing). `lld` computes `S + A` and writes the byte(s); v6asm
must store the addend `A` in the RELA entry and write a 0 placeholder in the
section bytes at the fixup offset.

---

## The core architectural problem

v6asm today is an **absolute** assembler:

- `OutputBuffer` is a single sparse 64 KiB array indexed by absolute address;
  `.org` chooses where bytes land.
- `eval_expr()` collapses every expression — including symbol references — to a
  concrete `i64` via a `resolve_symbol(name) -> Option<i64>` callback. The
  resolved value is baked directly into the bytes.
- There is **no notion of a section, a section-relative symbol, an undefined
  (external) symbol, or a relocation**.

A relocatable object requires all four. The central new capability is
**relocatable expression evaluation**: when an expression reduces to
`Symbol ± constant` where the symbol is *section-relative or undefined*, v6asm
must emit a relocation (type + symbol + addend) instead of a baked value.
Expressions that reduce to a pure constant (or to a difference of two symbols
in the same section) are baked as today.

---

## Design overview

Introduce an **object-emission mode** layered on top of the existing two-pass
core. The instruction encoder and expression parser are reused unchanged; the
differences are confined to (a) how output bytes are bucketed (sections vs.
one 64 KiB image), (b) a relocatable evaluation result type, and (c) a new ELF
serializer.

```
                         ┌──────────────────────────────┐
   .asm  ──preproc──▶    │  existing two-pass assembler │
 (macros/loops/if/      │  (parser, encoder, symbols)  │
  optional all expand)  └───────────────┬──────────────┘
                                         │
                    ┌────────────────────┴─────────────────────┐
                    ▼                                           ▼
           RomConfig (default)                        ObjConfig (--emit obj)
           OutputBuffer (abs 64K)                     SectionedOutput + Relocs
                    │                                           │
                    ▼                                           ▼
              generate_rom()                            elf::write_object()
                 *.rom                                       *.o (ET_REL)
```

---

## Phase 1: CLI & configuration plumbing

1. Learn the asm library: `temp\object_output\asm`, the best start will be `temp\object_output\asm\v6_interruption.asm`.
2. **New CLI flags** in `crates/v6asm/src/main.rs`:
   - `-f, --emit <fmt>` with values `rom` (default) | `obj`/`elf`.
   - When `obj`, default output extension becomes `.o` (instead of `.rom`).
   - `--rom-align` / ROM-only flags are ignored (with a warning) in obj mode.
3. **`OutputFormat` enum** threaded into the assemble path. Keep `RomConfig`;
   add `ObjConfig { /* reserved: e.g. emit_symtab_locals: bool */ }`.
4. Wire `cmd_assemble` to branch: build the `Assembler`, run the two passes,
   then call either `generate_rom`/`write_rom` or `elf::write_object`.

No behavior change to existing invocations.

---

## Phase 2: Section model

Goal: let assembled bytes live in named sections with **section-relative**
location counters, while remaining 100% backward compatible in ROM mode.

4. **`Section` type** (new `crates/v6_core/src/object/section.rs`):
   ```
   struct Section {
       name: String,           // ".text.interruption", ".data", ".bss"
       flags: u64,             // SHF_ALLOC|SHF_EXECINSTR|SHF_WRITE
       sh_type: u32,           // SHT_PROGBITS | SHT_NOBITS
       bytes: Vec<u8>,         // empty for NOBITS (.bss)
       pc: u32,                // section-relative location counter
       relocs: Vec<Reloc>,     // RELA entries for this section
   }
   ```
5. **Directives** (parser + assembler), active in **both** modes but only
   *meaningful* in obj mode:
   - `.section <name>[,"flags"[,@type]]` — open/append a section.
   - `.text` / `.data` / `.bss` / `.rodata` shorthands.
   - `.globl` / `.global`, `.local`, `.weak` — symbol binding.
   - `.type sym,@function|@object`, `.size sym,expr` — optional, low priority.
   In ROM mode these map to the existing single-image model (`.section`/`.text`
   are no-ops or just adjust PC); in obj mode they switch the active section.
6. **`.org` semantics in obj mode**: forbid absolute `.org` (error with a clear
   message — the linker chooses addresses). Allow a section-relative
   `.align`/`.storage` which already exist. Hardware-fixed addresses continue
   to be expressed as **absolute constants** (`name = $7331`), which become
   `SHN_ABS` symbols, not `.org`.

**Backward-compat rule**: if the source never opens a section and mode is ROM,
everything funnels into the legacy `OutputBuffer` exactly as today.

---

## Phase 3: Symbol classification

7. **Extend `SymbolInfo`** (`symbols.rs`) with an emission-facing kind, derived
   after pass 2:
   - `Defined { section_id, offset }` — a label inside a section
     (section-relative). STB_LOCAL unless `.globl`-ed.
   - `Absolute { value }` — a constant defined via `=`/`EQU`/`.var` that does
     **not** depend on any section-relative symbol (e.g. `$7331`, port numbers).
     Emitted as `SHN_ABS` if referenced by a reloc, otherwise baked.
   - `Undefined` — referenced but never defined in this TU → `SHN_UNDEF`
     global (an import the linker must satisfy).
8. **Binding & visibility**: `.globl` → `STB_GLOBAL`; default defined → `STB_LOCAL`;
   `.weak` → `STB_WEAK`. `@local` labels and macro-internal labels stay LOCAL
   and, by default, are **omitted** from `.symtab` unless referenced by a
   relocation (keeps the symbol table small; matches LLVM `.L` behavior).
9. **Undefined-symbol policy**: in ROM mode an unresolved symbol is an error
   (unchanged). In obj mode, an unresolved symbol that is *referenced in a
   relocatable context* becomes an external `SHN_UNDEF` import instead of an
   error. (Still error if used in a context that must be constant, e.g. `.if`,
   `.loop` count, `.org`, `.align`.)

---

## Phase 4: Relocatable expression evaluation (the heart)

10. **New result type** in `expr.rs`:
    ```
    struct RelocValue {
        addend: i64,                 // constant part
        symbol: Option<SymbolRef>,   // at most ONE unresolved/relocatable term
        coeff: i8,                   // +1 or -1 for the symbol term
    }
    ```
    Add `eval_expr_reloc(expr, resolver, pc) -> AsmResult<RelocValue>` that runs
    the same AST walk as `eval_expr` but, instead of failing on a
    section-relative/undefined symbol, carries it symbolically:
    - `Number/BoolLiteral/CurrentPC` → pure addend (`* `/PC inside a section is
      itself a section-relative reference → symbol = current section + offset).
    - `Symbol`/`LocalSymbol`:
      - resolves to `Absolute` → fold into addend;
      - resolves to `Defined`(section-relative) or `Undefined` → set `symbol`.
    - `Add/Sub`: combine; allow at most one live symbol term. `sym - sym` in the
      **same** section folds to a constant; cross-section/undefined `sym - sym`
      → error ("relocatable difference across sections not supported").
    - `Mul/Div/Shl/...` on a live symbol term → error ("non-linear relocatable
      expression"). Constant sub-expressions evaluate normally.
    - `LowByte`/`HighByte` on a live symbol term → record a **byte-op tag**
      (Lo/Hi) on the `RelocValue`; on a constant, behaves as today.
11. **Existing `eval_expr` stays** as the constant-only fast path (used by
    `.if`, `.loop`, `.align`, `.org`, `.storage` filler — all of which must be
    constant). It can be implemented as `eval_expr_reloc(...).require_constant()`.

---

## Phase 5: Fixup generation at emit sites

12. **Instruction immediates** (`instructions/` encoder call sites in
    `assembler.rs`): today an `imm16`/`imm8`/direct-address operand is evaluated
    to `i64` and the bytes are written. Change the obj-mode path to:
    - call `eval_expr_reloc`;
    - if `symbol.is_none()` → write bytes as today;
    - else → write a **0 placeholder** of the right width and push a `Reloc`:
      | operand width / byte-op | reloc type |
      |--------------------------|------------|
      | 16-bit immediate / address (`LXI`, `LHLD`, `SHLD`, `JMP`, `CALL`, `LDA`, `STA`, `DW`) | `R_V6C_16` |
      | 8-bit immediate (`MVI`, `ADI`, …) plain symbol | `R_V6C_8` |
      | 8-bit immediate with `<(expr)` low-byte tag | `R_V6C_LO8` |
      | 8-bit immediate with `>(expr)` high-byte tag | `R_V6C_HI8` |
      Reloc `r_offset` = section-relative offset of the operand field; `addend`
      = `RelocValue.addend` (sign-adjusted by `coeff`); `r_sym` = symbol index.
13. **Data directives** `.byte`/`.word`/`.dword` (`DB`/`DW`/`DD`): same logic
    per element. `.word sym+1` → `R_V6C_16` (covers self-modifying
    `dw label+1` and `shld label+1` patterns). `.byte <(sym)` → `R_V6C_LO8`.
    `.dword` against a relocatable symbol → error (no 32-bit V6C reloc) unless
    we later add one; keep `.dword` constant-only for now.
14. **`.optional` interaction**: v6asm normally prunes unused `.optional` blocks
    pre-emit. In obj mode the cleanest mapping is to **place each `.optional`
    block (or each exported routine) into its own `.section .text.<label>`** and
    let `ld.lld --gc-sections` do the pruning, matching how the LLVM backend
    already emits one `.text.<func>` per function. Provide a `.setting optional`
    toggle: `prune` (assemble-time, current behavior) vs `sections`
    (per-section, link-time). Default in obj mode = `sections`.

---

## Phase 6: ELF serializer

15. **New module `crates/v6_core/src/object/elf.rs`** — a small, dependency-free
    ELF32-LE writer (no external crate needed; the format subset is tiny):
    - **ELF header** (`Elf32_Ehdr`): `ET_REL`, `EM_V6C`, `EV_CURRENT`, class 32,
      LSB, OSABI 0, `e_flags=0`, no program headers.
    - **Sections** emitted in order: `null`, the user sections (PROGBITS/NOBITS),
      per-PROGBITS `.rela<name>` (SHT_RELA), `.symtab` (SHT_SYMTAB),
      `.strtab`, `.shstrtab`.
    - **`.symtab`**: index 0 = null; then **local** symbols (incl. one
      `STT_SECTION` symbol per section, used as the reloc base for
      section-relative references), then **global**/weak symbols.
      `sh_info` of `.symtab` = index of first global. `sh_link` = `.strtab`.
    - **RELA entries** (`Elf32_Rela`): `r_offset`, `r_info = (sym<<8)|type`,
      `r_addend`. `sh_link` = `.symtab`, `sh_info` = target section index.
    - **Section-relative references** use the target section's `STT_SECTION`
      symbol with the offset folded into the addend (LLVM-style), so the symbol
      table need not list every internal label.
16. **`write_object(asm, &ObjConfig, path)`** entry point in `output.rs`
    (sibling to `generate_rom`/`write_rom`): assembles the section/symbol/reloc
    model from the `Assembler` state and calls `elf::serialize`.

---

## Phase 7: Validation & tests

17. **Round-trip vs. LLVM tooling** (authoritative): assemble a small `.asm`
    in obj mode, then inspect with the project's LLVM binaries:
    - `llvm-readelf -h` → confirm `ET_REL`, `EM_V6C (0x8080)`, LSB, class 32.
    - `llvm-readelf -S -s -r` → sections, symbol bindings, RELA types/addends.
    - **Link test**: `ld.lld` the v6asm `.o` together with a clang-produced
      `.o` and the `v6c.ld` script; confirm symbols resolve and the final image
      bytes match a reference. This is the real acceptance gate.
18. **Golden tests** in v6asm's own suite: hex-compare emitted `.o` against
    checked-in expected bytes for:
    - a section-relative `JMP`/`CALL`/`LXI` (→ `R_V6C_16`);
    - `mvi a, <(sym)` / `mvi h, >(sym)` (→ `R_V6C_LO8`/`HI8`);
    - `dw label+1` and `shld label+1` self-modifying patterns (addend +1);
    - an undefined external (`call controls_check`) → `SHN_UNDEF` + reloc;
    - an absolute constant reference (`= $7331`) → baked, no reloc.
19. **End-to-end**: convert one real library file (e.g. `v6_interruption.asm`)
    to obj mode and link it with the C program that currently inlines it,
    confirming identical runtime behavior. This directly retires the
    hand-ported `v6_interruption.s` shim.

---

## Phase 8: Documentation

20. Add `docs/object-output.md` (on the v6llvmc side) and a section
    in v6asm's `docs/` covering: the `-f obj` flag, the section/symbol model,
    which expressions are relocatable, the `.optional` → per-section mapping,
    and the supported reloc types. Cross-reference the V6C ELF contract table.

---

## Scope boundaries / explicit non-goals (v1)

- **No PC-relative / GOT / PLT / dynamic linking** — V6C has none.
- **No 32-bit relocations** — `.dword` stays constant-only.
- **No `sym - sym` across sections** — only same-section differences fold.
- **DWARF is implemented separately** — see
  `design/plan_v6asm_dwarf_debug_metadata.md` and `docs/debug-metadata.md`.
  The object-output implementation remains responsible for section, symbol,
  and relocation mechanics used by that metadata.
- **No change to ROM mode** — default output and all existing flags behave
  identically.

---

## Risk / sequencing notes

- **Highest-risk piece is Phase 4** (relocatable evaluation): get the
  "at most one linear symbol term + addend + byte-op" algebra and its error
  cases right before wiring emit sites. Unit-test it in isolation first.
- **Section model (Phase 2) is invasive** to the assembler's PC handling;
  guard it behind obj mode and keep the ROM path on the existing `OutputBuffer`
  to avoid regressions.
- Sequence: 1 → 2 → 3 → 6 (serializer skeleton w/ no relocs) → 4 → 5 → 7. Land
  a no-reloc `.o` that `llvm-readelf` accepts early; add relocations on top.
- Keep the ELF writer self-contained (no `object`/`gimli` crates) to preserve
  v6asm's light dependency footprint, matching the existing hand-rolled style.
