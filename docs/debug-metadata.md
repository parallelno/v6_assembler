# Debug Metadata

`v6asm` can emit DWARF v4 source metadata for relocatable ELF objects. This is
intended for the object -> linker -> ELF workflow; direct ROM output does not
yet create a debug companion ELF.

## Generate A Debug Object

Pass `-g` (or `--debug`) together with `-f obj`:

```powershell
v6asm -g -f obj game.asm -o game.o
ld.lld -m elf32v6c -T v6c.ld game.o -o game.elf
llvm-objcopy -O binary game.elf game.rom
```

`--debug` currently requires object output. It is rejected for the default ROM
format, so enabling debug metadata never changes ROM bytes.

Keep `game.elf` for the debugger and use `game.rom` where a raw ROM image is
required. Do not run a debug-section stripping step on the ELF that the
debugger will consume.

## Emitted ELF Data

With `-g`, the object contains these non-allocating DWARF v4 sections:

| Section | Contents |
|---------|----------|
| `.debug_info` | One minimal compilation-unit DIE |
| `.debug_abbrev` | Abbreviations used by that compilation unit |
| `.debug_line` | Instruction source locations |
| `.debug_str` | Compilation-unit strings |

`.debug_line` uses `R_V6C_16` relocations against each code section. Its
corresponding `.rela.debug_line` section is present whenever the input has
executable statement rows. LLD resolves those section-relative instruction
addresses when it places the input sections in the final ELF.

Inspect an object before linking with:

```powershell
llvm-readelf -S -r game.o
llvm-dwarfdump --debug-info --debug-line game.o
```

Inspect the linked result with the same commands against `game.elf`.

## Source Attribution

The line table contains statement rows only for instructions. Labels, comments,
control directives, data directives, `.storage`, alignment padding, and `.org`
gaps are not breakpoint targets.

- An ordinary instruction maps to its own source file and line.
- Included files have their own line-table file entries. Paths are normalized
  with `/` separators relative to the source project where possible, so files
  with the same basename remain distinct.
- A loop body creates one instruction row per emitted iteration. These rows
  can share a source line while having different linked addresses.
- A macro-expanded instruction maps to the macro invocation line. This makes a
  breakpoint on the invocation stop in its generated code rather than on the
  macro definition.
- Multiple instructions separated by `\` on one physical source line produce
  separate rows at their respective instruction starts.

The human-readable listing has its own presentation rules. In particular, it
may show macro definition lines while the DWARF line table maps generated
instructions to the invocation site. See [Listing File Format](listing.md) for
listing behavior.

## Symbols

For a debug object, module-level code labels are included in `.symtab` as local
symbols unless `.globl`, `.global`, or `.weak` gives them an external binding.
Code labels in executable sections use `STT_FUNC`; existing section symbols
continue to be used for ordinary intra-object relocations.

Scoped `@` labels and macro-generated names are intentionally omitted from the
debugger-facing named-symbol set because their source spelling is not unique.
Use the DWARF line table to locate generated instructions instead.

## Current Limits

- Metadata is DWARF v4 and uses the V6C 16-bit address model.
- The assembler emits relocatable `ET_REL` objects only; it does not parse
  DWARF and does not write a direct-ROM `ET_EXEC` debug companion.
- Link the object with LLD and retain the linked ELF for debugging. The raw ROM
  alone has no embedded source metadata.
- Debug sections are opt-in. Invocations without `-g` retain the existing
  object and ROM output behavior.