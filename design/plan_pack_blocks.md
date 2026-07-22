# Plan: `.pack` / `.endpack` blocks — assembler-side hole packing

## Goal

Let the assembler pack movable **runtime data** blocks into the holes that
0x100 alignment leaves, squeezing more RAM variables into the tight page-bounded
layout without changing `ld.lld`. This targets exactly the pattern in
`temp/pack/runtime_data.asm`: a wall of RAM variable blocks, some of which must
be 0x100-aligned or must fit inside a single 0x100 page.

```
prev block ends at 0x730D+len
a 0x100-aligned block must start at the next boundary -> wastes the gap
.pack blocks let the assembler relocate reserved blocks into those gaps
```

## Model (locked)

### Blocks are runtime-only (uninitialized)

A `.pack` block may **only reserve space and define labels** — it never emits
bytes. This makes the arena a `.bss`-style region and removes byte emission,
relocations, and out-of-order code placement entirely.

- **Allowed inside a `.pack` block:** labels (`foo:`), constant/alias defs
  (`foo = expr`), and uninitialized reservation `.storage <len>` (no filler).
- **Forbidden inside a `.pack` block:** instructions; initialized data
  (`.byte` / `.word` / `.dword` / `.text "..."`, or `.storage <len>, <filler>`);
  `.align`; `.org`; section switches (`.section` / `.text` / `.data` /
  `.rodata` / `.bss`); `.optional`; and nested `.pack`.

### Three placement kinds, one fixed domain (0x100)

The packing domain is always **0x100** (the page). Placement is chosen by a
keyword, not a numeric argument:

| Directive | Kind | Rule |
|-----------|------|------|
| `.pack`          | filler   | no alignment; may sit anywhere, may cross page boundaries |
| `.pack align`    | anchor   | must **start** on a 0x100 boundary |
| `.pack window`   | windowed | must **not straddle** a 0x100 boundary (fits inside one page), but need not start on one |

`window` is the type that makes real data (e.g. `runtime_data.asm`) pack
tightly — see the efficiency study below.

### One collected arena

- **Object mode:** all `.pack` blocks collect into a single implicitly-created
  section named **`.bss.pack`**, `SHT_NOBITS`, `SHF_ALLOC | SHF_WRITE`,
  `sh_addralign = 0x100`. The `.bss.*` name makes the section inherit NOBITS +
  alloc/write from `default_type`/`default_flags` and be routed by the standard
  linker-script rule `*(.bss .bss.* ...)` into the RAM/BSS region with no custom
  script. It occupies **no file bytes**; `ld.lld` places it on a 0x100 boundary
  as one unit, preserving every internal offset — so each `align` block lands on
  a page boundary and each `window` block stays within one page.
- **ROM mode:** no sections — the arena is one contiguous **reserved** region
  placed at the location of the **first `.pack` block in source order**, rounded
  up to 0x100. It reserves address space (advances the location counter) without
  emitting initialized bytes. Subsequent inline content resumes after the arena.

### Multiple objects (obj mode)

The `.bss.pack` section is shared **in name only**. The linker never re-packs
across objects — it **concatenates input sections**: each object's `.bss.pack`
contribution is placed as one atomic 0x100-aligned unit, in link order, with up
to 255 bytes of inter-object padding to re-align each to a page boundary. So
packing is **per-object**; each object's internal layout (anchors on boundaries,
windows within a page) is preserved because every object's arena base is
0x100-aligned. Cross-object holes are not filled — that stays a non-goal. For
this workload all packed RAM lives in one object, so the padding cost is zero.

### Reordering is expected

Blocks are laid out by the packer, not in source order; all references go through
labels (which resolve to packed addresses).

## Directive

```
.pack            ; filler   (no alignment)
.pack align      ; anchor   (starts on a 0x100 boundary)
.pack window     ; windowed (must not cross a 0x100 boundary)
    ... labels + `.storage <len>` reservations only ...
.endpack
```

### Errors

- `.pack` block defining **no label** → error (unreferenceable after moving).
- **Empty** `.pack` block (reserves zero bytes) → error.
- A `window` block whose reserved length `> 0x100` → error (can never fit a
  page). (`align` and `filler` blocks may exceed 0x100 and span pages.)
- Any forbidden directive/content (code, initialized data, `.align`, `.org`,
  section switch, `.optional`, nested `.pack`) inside a block → error.
- Unknown keyword after `.pack` → error.
- `.endpack` without a matching `.pack`, or an unclosed `.pack` → error.

## Packing algorithm (best-fit-decreasing)

Per compilation unit (no cross-object packing). Domain = 0x100; the arena base is
0x100-aligned, so arena-relative offsets translate to correctly aligned/paged
absolute addresses.

1. **Skeleton (anchors).** Place `align` blocks in **descending size order**.
   Keep a `cursor` (arena-relative, from 0). For each anchor, round `cursor` up
   to 0x100; the skipped range becomes an open **hole**; place the block;
   advance `cursor`. Ordering large anchors first keeps the smallest anchor last
   (unrounded), so its trailing padding is never forced into a hole.
2. **Fill — windows first (most-constrained-first).** Fill in two size-descending
   passes (tie-break: source order): first the `window` blocks, then the
   `filler` blocks. Windows have the tighter constraint and must get first pick
   of the scarce non-straddling positions; fillers then mop up everything —
   including the page-crossing regions windows can never use. For each block pick
   the **best-fit hole** (smallest hole it fits):
   - `filler`: fits if `size ≤ hole`; placed at the hole front.
   - `window`: must land without crossing a 0x100 boundary — try the hole front;
     if that would straddle, try the next 0x100 boundary inside the hole; else
     the hole doesn't fit.

   If no hole fits, append after the skeleton. A `window` block bumps to the
   next 0x100 boundary there if needed to avoid straddling; the bytes it skips
   are **registered as a new hole** so later (smaller) blocks can still fill them
   instead of leaking waste to the end of the arena.
3. `arena_size` = final append cursor. Remaining hole bytes are unused space (in
   a NOBITS section they cost nothing in the file, only address range).

Every block gets an arena-relative offset; each interior label gets
`block_offset + label_offset_within_block`.

## Emission (now trivial — no bytes)

Because blocks only reserve space:

1. **Collection.** At the first `.pack` encountered in each assembler pass,
  scan all source lines for every `.pack` block. Record each block's source
  range, kind, reserved **size**, and each label's offset within the block.
  Run all validations here. Do not assign inline addresses to pack labels.
2. **Layout.** Run the packer immediately after collection → per-block arena
  offset and `arena_size`. This lets pass 1 reserve the complete arena before
  processing inline content that follows the first `.pack` block.
3. **Address assignment.** Define every pack-block label at its packed address:
   - ROM: `arena_addr + offset`, where `arena_addr = align_up(pc_at_first_pack,
     0x100)`.
   - Obj: section `.bss.pack`, section-relative `offset`.
4. **Reservation.** ROM: reserve `arena_size` at `arena_addr` so later inline
   content doesn't overlap. Obj: grow the `.bss.pack` NOBITS section by
   `arena_size` (no bytes written). No pass-2 body replay, no relocation
   handling.

## Relevant existing code

- Parser: `Directive` enum ([crates/v6_core/src/parser.rs](crates/v6_core/src/parser.rs#L42-L55));
  keyword dispatch (`"ALIGN"` L405, `"ORG"` L293, `"OPTIONAL"` L370) — add
  `"PACK"` (optional `align`/`window` keyword operand) and `"ENDPACK"`.
- Block matching: `ControlDirective`/`BlockKind` and `find_matching_block_end`
  ([assembler.rs L929-L967](crates/v6_core/src/assembler.rs#L929-L967)) — add
  `BlockKind::Pack` and `ControlDirective::Pack`/`EndPack`; `.pack` cannot nest.
- NOBITS/alloc/align plumbing already present: `Section::addralign` / `set_align`
  and `default_type` (NOBITS for `.bss*`)
  ([object/section.rs L90-L177](crates/v6_core/src/object/section.rs#L90-L177)),
  `switch_section` ([L1621-L1633](crates/v6_core/src/assembler.rs#L1621-L1633)),
  `section_index` ([L58](crates/v6_core/src/assembler.rs#L58)),
  `out_reserve` / `advance_pc` ([L1636](crates/v6_core/src/assembler.rs#L1636)),
  `Section::reserve` ([object/section.rs L94-L99](crates/v6_core/src/object/section.rs#L94-L99)).
- `.storage` handling (reservation source): pass 2
  [L1258-L1262](crates/v6_core/src/assembler.rs#L1258-L1262).
- Label definition: `define_label_in`
  ([symbols.rs L104-L125](crates/v6_core/src/symbols.rs#L104-L125)).

## Implementation outline

1. **Parser**: `Directive::Pack(PackKind)` where `PackKind = Filler | Align |
   Window`, plus `Directive::EndPack`.
2. **Block model**: `BlockKind::Pack`, `ControlDirective::Pack`/`EndPack`; reject
   nesting; extend `find_matching_block_end`.
3. **Collection/validation**: gather `PackBlock { kind, size, labels:
   Vec<(name, offset)> }`; emit the no-label, empty, window>0x100, and
   forbidden-content errors.
4. **Packer**: descending-size skeleton of anchors, then two best-fit passes
   (windows first, then fillers) with the `align`/`window`/`filler` rules above
   → arena offsets and `arena_size`.
5. **Address assignment + reservation**: define pack labels at packed addresses;
   reserve `arena_size` (ROM at the first-`.pack` anchor; obj in the `.bss.pack`
   NOBITS section).
6. **Listing**: record `.pack` / `.endpack` lines like the `.optional` arms.

## Efficiency study (temp/pack/pack.py)

Using the 23 real blocks from `temp/pack/runtime_data.asm` (3232 data bytes;
4 `align`, 4 `window`, 15 `filler`; theoretical minimum = 3232):

| model | arena | waste | efficiency |
|-------|-------|-------|------------|
| no `window` type (`window` treated as `align`)          | 3570 | 338 | 90.5% |
| `window` type, source anchors, combined fill (early)    | 3339 | 107 | 96.8% |
| **`window` type, descending anchors, windows-first**    | 3232 |   0 | **100%** |

Three cumulative wins:
- **`window` type** recovers the bulk (231 bytes vs. the no-window model).
- **windows-first fill** (most-constrained-first) gives windows first pick of the
  scarce non-straddling positions.
- **append-bump holes**: when an appended `window` is bumped to the next page
  boundary, the skipped bytes are registered as a hole and filled by later small
  blocks. On this dataset these three together reach the theoretical minimum
  (zero waste). See `temp/pack/pack_layout.txt` for the full packed address
  table.

Descending-anchor ordering is used for robustness in the worst case (few
fillers): keeping the smallest anchor last avoids turning its trailing padding
into a forced hole.

## Tests

`assembler_tests.rs` / `cases/` (ROM):
- `align` block + filler → filler lands in the hole before the anchor; anchor on
  its boundary.
- `window` block placed inside a page without straddling; a `window` block whose
  natural spot would straddle gets bumped to the next boundary within the hole.
- Fillers sorted best-fit-decreasing into holes; leftover appended.
- Arena placed at the first `.pack` position, rounded up to 0x100.
- Errors: `.pack` with no label; empty `.pack`; `window` > 0x100; code or
  initialized data inside a block; `.align`/`.org`/section-switch/`.optional`/
  nested `.pack`.

`object_tests.rs` (obj):
- All `.pack` blocks land in one `.bss.pack` section: `SHT_NOBITS`,
  `SHF_ALLOC | SHF_WRITE`, `sh_addralign = 0x100`, zero file bytes.
- `align` label at a 0x100-multiple offset; `window` label within one page;
  filler label anywhere.
- `llvm-readelf -S out.o` shows `.bss.pack` as NOBITS with `Align = 256`.

## Verification

- `cargo test -p v6_core`.
- ROM: assemble a runtime-data file using `.pack` / `.pack align` / `.pack
  window`; inspect the `.lst` to confirm holes are filled, anchors on boundaries,
  and no `window` block straddles a page.
- Obj: `llvm-readelf -S` shows the NOBITS aligned `.bss.pack` section.

## Non-goals

- Initialized data or code inside `.pack` blocks (reservation only).
- Cross-object-file packing (would need a post-link tool).
- Alignment domains other than 0x100.
- Nested `.pack` blocks; `.align` inside a `.pack` block.
- Honoring source order for placed blocks (packer reorders freely).
- Optimal bin-packing (best-fit-decreasing is a heuristic).
