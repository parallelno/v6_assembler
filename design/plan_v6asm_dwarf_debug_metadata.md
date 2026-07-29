# Plan: v6asm DWARF Debug Metadata

## Scope and Repository Boundary

Implement this plan in the upstream Rust project
`https://github.com/parallelno/v6asm`. This repository contains only a packaged
`tools/v6asm/v6asm.exe` and mirrored documentation, so source changes must land
and pass upstream first. After release, update the packaged executable and docs
here and run the mixed Clang/v6asm integration tests from
`plan_source_debug_metadata.md`.

This plan implements the assembler side of the shared final-ELF contract. It
does not add DWARF parsing to v6asm or v6emul.

## 1. Problem

### Current behavior

Verified against upstream `parallelno/v6asm` on 2026-07-28:

- `-f obj` already emits ELF32 little-endian `ET_REL`, `EM_V6C` objects with
  sections, section-relative symbols, and RELA relocations.
- Pass 2 already records `ListingLine { file, line_num, text, addr,
  byte_count, macro_expansion }` for emitted instructions and data.
- `SourceLine` carries `file`, `line_num`, `text`, and an optional
  `macro_context`; included files retain their own file and line.
- Loop expansion repeats the body records at distinct addresses.
- Macro bodies retain definition file/line metadata. The listing prints
  definition lines and a source-only invocation line, but the current metadata
  does not retain a structured invocation stack suitable for debugger policy.
- Object serialization is specialized around user sections, `.symtab`,
  `.strtab`, and `.rela.*`; it has no general debug-section model.
- Only exported/weak named symbols are added to ELF `.symtab`; ordinary labels
  are generally represented through section symbols and relocation addends.
- `docs/object-output.md` explicitly lists DWARF as a non-goal.
- Documentation mentions an existing `.symbols.json` debug path, but current
  source and CLI contain no generator or option. Treat this statement as stale
  documentation, not as an implementation dependency.

### Desired behavior

1. `v6asm -g -f obj source.asm -o source.o` emits a relocatable object with
   minimal standards-compatible DWARF v4 (`.debug_info`, `.debug_abbrev`,
   `.debug_line`, and `.debug_str`) plus relocations.
2. LLD links those objects with Clang V6C objects, resolves debug addresses,
   and preserves usable line tables in the final ELF.
3. Every executable line row identifies its section-relative instruction
   start, byte range, source file, line, optional column, statement status, and
   expansion provenance.
4. Includes have distinct file-table entries. Loop and macro expansion can
   produce several address ranges for one source location.
5. Non-emitting lines and data directives are not statement breakpoint rows.
6. Useful module labels appear as local or global ELF symbols with final linked
   addresses; local-scoped and macro-generated names remain deterministic and
   collision-free.
7. In a second milestone, direct ROM mode can emit an `ET_EXEC` debug companion
   from the same assembly result, avoiding a custom JSON map.

### Root cause

The listing recorder contains much of the raw information but is designed for
human presentation. It uses a 16-bit absolute `addr`, has no object-section
identity, only a boolean macro marker, and mixes code/data/non-emitting rows.
The ELF serializer cannot currently accept arbitrary metadata sections or all
relocation widths needed by DWARF.

## 2. Strategy

### Approach: a dedicated debug-row model feeding a minimal DWARF v4 writer

Do not parse `.lst` output and do not serialize `ListingLine` directly. Add an
internal model such as:

```rust
struct DebugLineRow {
    section: Option<usize>,       // Some in object mode; absolute in ROM mode
    offset_or_address: u32,
    byte_len: u32,
    source: SourceLocation,
    column: u32,
    is_stmt: bool,
    kind: EmissionKind,           // Instruction, Data, Storage, Padding
    expansion: Vec<ExpansionSite>,
}

struct ExpansionSite {
    kind: ExpansionKind,          // Macro or Loop
    name: Option<String>,
    definition: SourceLocation,
    invocation: SourceLocation,
}
```

Record rows at the point pass 2 knows both the active output section and the
emitted byte count. Keep listing generation as a separate consumer so debug
policy changes cannot alter `.lst` compatibility.

### Source attribution policy

- Ordinary instruction: its own source location, `is_stmt = true`.
- Include: the included file's location, with normalized path in the line file
  table.
- Loop body: the body source location for each emitted iteration; repeated
  addresses are expected.
- Macro body: emit the invocation location as the primary `is_stmt` row so a
  breakpoint on `DRAW_SPRITE(...)` works. Retain the definition location in
  `expansion`; optionally emit it as a non-statement row only if LLVM tools and
  the adapter fixture handle same-address rows deterministically.
- Labels/comments/control directives: no executable row.
- Data, `.storage`, alignment, and `.org` gaps: retain ranges internally when
  useful for diagnostics, but do not set `is_stmt` and do not expose them as
  source-breakpoint targets.
- A source line containing several `\`-separated instructions produces one row
  per instruction start, sharing line and using lexer columns when available.

### DWARF/ELF contract

Emit the same DWARF v4 subset defined by
`plan_source_debug_metadata.md`. Object-mode addresses are relocatable against
their code sections. Use V6C 16-bit address relocations for code addresses and
the shared `R_V6C_32` relocation for 32-bit DWARF section offsets if required
by the final LLVM-side ABI.

The first release supports debug metadata only with `-f obj`; users link the
object with `ld.lld` and retain the final ELF. This gets the mixed-language
workflow working before adding an executable-ELF writer for direct ROM mode.

### Direct ROM companion design

After object mode is stable, define:

```text
v6asm -g game.asm -o game.rom
    -> game.rom
    -> game.elf
```

Allow an explicit companion path (for example `--debug-elf <path>`) and make
the default sibling behavior conditional on `-g`. Both artifacts must be
generated from one immutable assembly result. The companion is ELF32
little-endian `ET_EXEC`, `EM_V6C`, with allocatable image sections at their
actual `.org` addresses, `.symtab`, and non-allocating DWARF sections. ROM bytes
must remain byte-identical to non-debug assembly.

Do not create a new JSON source-map schema. If executable-ELF output proves too
large for the first release, document direct ROM debugging as deferred and use
the object -> LLD -> ELF -> objcopy flow.

### Why this works

- It reuses v6asm's proven source, expansion, section, symbol, and relocation
  machinery without coupling machine-readable metadata to listing formatting.
- Section-relative debug rows survive LLD placement and `--gc-sections`.
- The same adapter reader handles C, v6asm, and mixed binaries.
- An opt-in `-g` preserves current ROM/object bytes and command behavior.

### Summary of changes

| Upstream area | Change |
|---------------|--------|
| `preprocessor.rs` | Structured expansion provenance and normalized source identities |
| `assembler.rs` | Dedicated debug rows with active section, offset, byte length, and emission kind |
| `object/` | Generic ELF sections/relocations and DWARF v4 serializer |
| `output.rs` | Include debug sections and debugger-visible local symbols in object output |
| CLI crate | Add `-g/--debug` and optional companion path; validate combinations |
| Tests/docs | Unit fixtures, LLVM interoperability, macro/include/loop coverage, corrected `.symbols.json` claim |

## 3. Implementation Steps

### Step 3.1 - Read references and freeze compatibility [ ]

Read upstream `docs/cli.md`, `docs/object-output.md`, `docs/listing.md`,
`docs/macros.md`, `crates/v6_core/src/preprocessor.rs`, `assembler.rs`,
`symbols.rs`, `output.rs`, `object/section.rs`, and `object/elf.rs`, plus the
shared V6C DWARF/relocation contract in `plan_source_debug_metadata.md`.

Capture golden hashes for representative ROM and object outputs without `-g`.
These hashes are non-regression gates.

> **Implementation Notes**:

### Step 3.2 - Add failing metadata-model tests [ ]

Add unit fixtures for one instruction, two statements on one physical line,
include, nested include, loop expansion, macro expansion, nested macro, data,
storage, alignment, optional sections, and a discarded optional block.

Assert the intended `DebugLineRow` values before implementing serialization.
These tests must demonstrate the current inability to retain section identity
and macro invocation provenance.

> **Implementation Notes**:

### Step 3.3 - Enrich source and expansion provenance [ ]

Replace the lossy `Option<String>` macro context used for debugging with a
structured expansion chain while preserving any existing field/API needed by
diagnostics. Record definition and invocation source locations for each macro
level. Record loop origin/iteration where useful without creating synthetic
source lines.

Normalize paths relative to a declared compilation directory, retain the
compilation directory separately, and keep path comparison independent of
host separator/case rules. Add tests for Windows paths and two includes with
the same basename.

> **Implementation Notes**:

### Step 3.4 - Record section-relative debug rows in pass 2 [ ]

Add the dedicated row collection to `Assembler`. Capture active section and
offset before emission, byte count after emission, exact emission kind, source
location, lexer-derived column where possible, and expansion chain.

For ROM mode capture absolute addresses. Keep `ListingLine` behavior and
listing snapshots unchanged. Ensure pass reset and optional-section layout do
not leak duplicate rows between passes.

> **Implementation Notes**:

### Step 3.5 - Add a minimal DWARF v4 encoder [ ]

Implement small, independently tested encoders for ULEB128/SLEB128, initial
lengths, abbreviations, compile-unit DIEs, file/directory tables, and line
program opcodes. Prefer a mature Rust DWARF-writing crate if it supports
16-bit addresses, relocatable section references, custom ELF integration, and
the project's license; otherwise keep the local encoder limited to the frozen
subset.

Use deterministic ordering for directories, files, strings, rows, and
compilation units. End every discontiguous code section/sequence correctly.

> **Implementation Notes**:

### Step 3.6 - Generalize ELF serialization for debug sections [ ]

Extend `object/elf.rs` so callers can add non-allocating PROGBITS sections,
explicit alignment, links, entry sizes, and RELA sections without hard-coding
only user sections and symbols. Preserve existing section indices or introduce
a clear mapping from assembler section IDs to final ELF section indices.

Emit `.debug_info`, `.debug_abbrev`, `.debug_line`, `.debug_str`, and required
`.rela.debug_*` sections only under `-g`. Verify section flags are zero and no
debug bytes are allocatable.

> **Implementation Notes**:

### Step 3.7 - Complete debugger-visible ELF symbols [ ]

Add ordinary module-level labels to `.symtab` with `STB_LOCAL` unless declared
global/weak. Preserve current relocation optimization through section symbols;
debugger symbols do not need to become relocation targets.

Use `STT_FUNC` for code labels with a known function/optional-block extent,
`STT_OBJECT` for named data with a known extent, and `STT_NOTYPE` otherwise.
Give functions non-zero sizes when the `.function`/`.optional` boundaries make
the size unambiguous. Define deterministic names for scoped local and macro
labels or deliberately omit only those that cannot be represented without
collision; test and document the rule.

> **Implementation Notes**:

### Step 3.8 - Add CLI and object-mode output [ ]

Add `-g` / `--debug`. In the first milestone require `--format obj` (or accept
ROM mode only when `--debug-elf` is implemented). Keep debug output opt-in and
do not change default names or bytes.

Update help snapshots and errors for unsupported option combinations. Document
the canonical flow:

```powershell
v6asm -g -f obj game.asm -o game.o
ld.lld -m elf32v6c -T v6c.ld game.o -o game.elf
llvm-objcopy -O binary game.elf game.rom
```

> **Implementation Notes**:

### Step 3.9 - LLVM interoperability tests [ ]

In CI or an integration script, inspect v6asm output with `llvm-readelf`,
`llvm-dwarfdump`, and `llvm-objdump`. Link it alone and with a Clang `-g
-gdwarf-4` object. Assert final line rows, source files, symbol addresses,
relocation resolution, and `--gc-sections` removal.

Include multiple source ranges for one loop/macro line and verify the shared
adapter fixture resolves every address.

> **Implementation Notes**:

### Step 3.10 - Implement direct ROM companion ELF [ ]

After Step 3.9 passes, add the `ET_EXEC` writer and `--debug-elf`/sibling output
behavior. Represent sparse `.org` regions without turning non-loaded gaps into
source rows. Emit actual runtime addresses, not zero-based addresses plus a
consumer-side implicit bias.

Compare ROM bytes from a debug and non-debug invocation and reconstruct the
same bytes from the companion ELF using the adapter identity checker.

> **Implementation Notes**:

### Step 3.11 - Build [ ]

Run `cargo build --release` (or upstream `scripts/build.bat`) and ensure the
packaged CLI help reflects debug options. Build LLVM tools required by the
interoperability fixtures.

> **Implementation Notes**:

### Step 3.12 - Unit and integration tests [ ]

Run `cargo test -p v6_core` and `cargo test --workspace`. Add malformed-input
tests for line-program overflow, more than 64 KiB of code addresses, invalid
section mappings, missing source files, deep expansion provenance, and
unsupported relocation width.

Set a performance gate on a representative macro/loop-heavy project. Debug
recording and DWARF serialization should remain linear in emitted rows; record
assembly time, peak memory, object size, and debug-section size.

> **Implementation Notes**:

### Step 3.13 - V6C end-to-end regression [ ]

Update `tools/v6asm/v6asm.exe` only from a passing tagged/pinned upstream
build. Assemble/link a mixed C and v6asm feature test, run it in
`tools/v6emul`, and execute the source-breakpoint integration from the main
plan.

Run `python tests/run_all.py` in this repository and confirm all non-debug ROM
goldens remain unchanged.

> **Implementation Notes**:

### Step 3.14 - Verification assembly steps from `tests/features/README.md` [ ]

Follow the documented feature-test compile, v6asm object assembly, LLD link,
objcopy, and v6emul steps. Record expected source/address mappings for main
file, include, macro invocation, and loop body.

> **Implementation Notes**:

### Step 3.15 - Make sure `result.txt` is created [ ]

Create/update the selected feature's `result.txt` according to
`tests/features/README.md`. Include v6asm version, ELF debug sections, symbol
and line-table excerpts, ROM digest, mapped breakpoint addresses, stop PCs,
emulator output, and timing/size measurements.

> **Implementation Notes**:

### Step 3.16 - Documentation and stale-claim cleanup [ ]

Update upstream CLI, object-output, listing, macro, and README documentation.
Remove or correct the `.symbols.json` statements unless that artifact is
separately restored and tested. Explain DWARF version, path normalization,
macro attribution, direct-ROM limitations, symbol inclusion, and stripping.

Mirror the released docs under `tools/v6asm/docs/`. Mark completed steps `[x]`
and fill every Implementation Notes block.

> **Implementation Notes**:

### Step 3.17 - Sync mirror [ ]

No Rust source mirror exists in this repository. Sync means: pin the upstream
release/version, replace the packaged executable, mirror changed public docs,
run the binary `--version`/`--help` smoke tests, and then run
`pwsh scripts/sync_llvm_mirror.ps1` for any shared V6C relocation/test changes
made by the main plan.

> **Implementation Notes**:

## 4. Expected Results

### Example 1 - Included instruction

`sprites.asm:18` emits an instruction in section `.text.draw_sprite` at offset
`0x0007`. After LLD places the section at `0x1830`, the final line table maps
the source row to CPU address `0x1837`.

### Example 2 - Macro invocation

`main.asm:42` invokes `COPY_ROW(...)`, whose body emits four instructions. The
primary statement rows all identify the invocation line and resolve to four
distinct final addresses; definition locations remain available as expansion
provenance.

### Example 3 - Link-time garbage collection

An unreferenced `.optional` function and its source rows are discarded by
`ld.lld --gc-sections`. The adapter cannot place a breakpoint in dead code.

### Example 4 - Direct ROM workflow

`v6asm -g game.asm -o game.rom` emits `game.rom` and `game.elf` from one
assembly result. The companion's runtime addresses match listing/emulator PCs,
and enabling `-g` does not change a single ROM byte.

## 5. Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Listing records are reused as machine metadata | Introduce `DebugLineRow`; keep listing snapshots independent |
| Object section IDs differ from serialized ELF indices | Maintain and unit-test an explicit section-index mapping |
| Macro provenance is lossy | Replace the boolean/string marker with a structured definition/invocation chain |
| Same-address definition/invocation rows confuse tools | Make invocation the sole Level 1 statement row; test optional non-statement rows before emitting them |
| Local labels bloat `.symtab` | Include useful deterministic labels under `-g`; measure size and document exclusions |
| Hand-written DWARF is malformed | Freeze a minimal v4 subset, unit-test encoders, and validate every fixture with LLVM tools |
| Debug relocations disagree with LLVM | Share the numeric ABI and link v6asm + Clang objects in CI |
| Direct companion ELF mishandles `.org` gaps | Implement after relocatable mode; compare reconstructed bytes and source ranges against ROM/listing |
| Debug mode changes normal output | Golden hash default ROM/object/listing artifacts before and after implementation |
| Packaged binary cannot be traced to source | Pin a tagged upstream version and record `v6asm --version` in integration results |

## 6. Relationship to Other Improvements

- Extends, rather than replaces, v6asm's existing ELF object output.
- Uses the V6C relocation support completed by the main debug metadata plan.
- Supplies assembly line tables to the same adapter index used for Clang.
- Makes `.function` boundaries more useful through symbol sizes without
  changing optional-section semantics.

## 7. Future Enhancements

- DWARF macro sections with full nested expansion views.
- Rich `.debug_info` subprogram DIEs and assembly variable/constants.
- Source-aware stepping policies for one invocation expanding to many
  instructions.
- Reproducible path remapping flags analogous to Clang's debug prefix maps.
- DWARF v5 and split/compressed debug data after ecosystem validation.
- A library API returning the debug companion in memory for editor/build-tool
  integrations.

## 8. References

* [parallelno/v6asm](https://github.com/parallelno/v6asm)
* [DWARF Debugging Information Format](https://dwarfstd.org/)
* `tools/v6asm/docs/cli.md`
* `tools/v6asm/docs/object-output.md`
* `tools/v6asm/docs/listing.md`
* `tools/v6asm/docs/macros.md`
* `design/future_plans/plan_source_debug_metadata.md`
* `design/future_plans/README.md`