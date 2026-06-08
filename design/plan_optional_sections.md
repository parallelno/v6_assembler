# Plan: Per-section emission for `.optional` blocks in ELF object mode

## Goal

In object mode (`v6asm -f obj`), place each `.optional` / `.function` block into
its own ELF section so the linker (`ld.lld --gc-sections`) prunes unused blocks
at **link time**, instead of the current **assemble-time** pruning. This matches
how the LLVM V6C backend emits one `.text.<func>` section per function.

- Section name = `.text.<first label>` when the block contains code.
- Section name = `.data.<first label>` when the block contains **only data**
  (no instructions). `.data.*` is `SHF_ALLOC | SHF_WRITE` — relocatable and
  overridable, suitable for data that another object may replace.
- `<first label>` = the first label defined in the block that is **referenced
  from outside** the block (the label that keeps the block alive); falls back to
  the first defined label if none are referenced externally.
- Activation: `.setting optional, prune|sections` (also `true`/`false`).
  Default in obj mode = `sections`; default in ROM mode = `prune`.
- A `.optional` block that defines **no** label or constant is now an **error**
  (previously it was silently pruned, which is meaningless: a block with no
  externally-visible symbols can never be referenced).

## Decisions

- "First label" = the first label defined in the block that is referenced from
  outside the block (`ParsedLine::Label`), falling back to the first defined
  label when none are referenced externally.
- Data-only block → `.data.<label>` (writable / overridable).
- Default ON in obj mode, toggle via `.setting optional, prune|sections`.
- Label-less `.optional` block → assembler error in both ROM and obj mode.

## Relevant existing code

- `.optional` handled in `assembler.rs`:
  - pass1 `process_lines_pass1` (`ControlDirective::Optional` arm).
  - pass2 `process_lines_pass2` (`ControlDirective::Optional` arm, also records
    listing lines).
  - Both previously used `should_include_optional_block` /
    `find_matching_block_end` / `collect_optional_block_symbols`.
- `AssemblerSettings.optional_enabled: bool` parsed by `.setting optional`.
- `ObjectState` (`obj.sections`, `obj.active`, `section_index(name)`) and
  `switch_section(name)` (obj-mode only; sets `active` and `pc = section.size`).
- `Section::default_flags` / `default_type` already infer correct flags/type for
  `.text.*` (ALLOC|EXEC, PROGBITS) and `.data.*` (ALLOC|WRITE, PROGBITS) — no
  change needed there.

## Implementation

### Settings

- Keep `optional_enabled: bool` (backward compatible; `false` = include all,
  no pruning, no sections).
- Add `optional_sections: Option<bool>`:
  - `None` → format default (sections in obj mode, prune otherwise).
  - `Some(true)` → sections, `Some(false)` → prune.
- `.setting optional, <val>`:
  - `false` / `disabled` → `optional_enabled = false`.
  - `true` → `optional_enabled = true`.
  - `prune` → `optional_enabled = true`, `optional_sections = Some(false)`.
  - `sections` → `optional_enabled = true`, `optional_sections = Some(true)`.

### Strategy resolution

`OptionalStrategy { Prune, Sections, IncludeAll }`:

- `!optional_enabled` → `IncludeAll`.
- else `want_sections = optional_sections.unwrap_or(obj_mode)`; if `want_sections`
  and obj mode → `Sections`, otherwise `Prune` (sections impossible in ROM mode).

### Per-block resolution

`resolve_optional_block(lines, inner_start, inner_end) -> OptionalAction`:

1. Collect defined symbols; if empty → **error** (label-less block).
2. `IncludeAll` → `IncludeHere`.
3. `Prune` → reference scan over lines outside the block; referenced →
   `IncludeHere`, else `Skip`.
4. `Sections` → first referenced label → `IncludeInSection(".text."|".data." + label)`
   (`.text.` if block contains an instruction or macro invocation, else
   `.data.`); constants-only block with no label → `IncludeHere`.

`OptionalAction { Skip, IncludeHere, IncludeInSection(String) }`.

### Pass integration

Both passes:

```text
match resolve_optional_block(...) {
    Skip              => {}
    IncludeHere       => process block,
    IncludeInSection(name) => { saved = enter_optional_section(name);
                                process block;
                                leave_optional_section(saved); }
}
```

`enter_optional_section` saves `(obj.active, pc)` and calls `switch_section`;
`leave_optional_section` restores them. Nested blocks each get their own section
and restore correctly.

## Tests

- `object_tests.rs`:
  - code block → `.text.<lbl>` (ALLOC|EXEC).
  - data-only block → `.data.<lbl>` (ALLOC|WRITE).
  - nested optional blocks → two sections.
  - `.setting optional, prune` in obj mode → assemble-time prune (no extra
    section, unreferenced block dropped).
  - label-less `.optional` block → error.
- Existing ROM-mode `.optional` cases unchanged.

## Verification

- `cargo test -p v6_core`.
- Manual: assemble a file mixing a code `.optional` and a data-only `.optional`
  in obj mode, `llvm-readelf -S out.o` shows `.text.<lbl>` (ALLOC+EXEC) and
  `.data.<lbl>` (ALLOC+WRITE).

## Non-goals

- COMDAT / `SHT_GROUP` for true cross-object override semantics; weak binding is
  unchanged. `.data.<label>` is writable + relocatable, but duplicate strong
  defs across objects still conflict.
- ROM-mode behavior is unchanged (still assemble-time prune by default).
