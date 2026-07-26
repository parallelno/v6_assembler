# Plan: Always-on symbol dependency diagnostics

## Goal

When assembly fails because an expression cannot be evaluated, always explain
which symbol was missing and how the failed expression depends on it. Preserve
the immediate error and source location, then add deterministic `note:` entries
for relevant symbol definitions and dependency links.

The motivating failure is a derived constant used by `.storage`:

```asm
V6_GC_TASK_SPS_LEN = GC_TASKS * WORD_LEN
v6_gc_task_sps:
    .storage V6_GC_TASK_SPS_LEN
```

Instead of reporting only the consequence:

```text
error: Undefined symbol: V6_GC_TASK_SPS_LEN
  --> sound/v6_gc_runtime_data.asm:31
```

report the dependency that prevented the constant from resolving:

```text
error: Undefined symbol: V6_GC_TASK_SPS_LEN
  --> sound/v6_gc_runtime_data.asm:31

note: `V6_GC_TASK_SPS_LEN` is defined here
  --> sound/v6_gc_runtime_data.asm:29
note: its value depends on undefined symbol `GC_TASKS`
```

For a longer chain:

```asm
BUFFER_SIZE = ITEM_COUNT * ITEM_SIZE
ITEM_COUNT = TABLE_END - TABLE_START
.storage BUFFER_SIZE
```

report:

```text
error: Undefined symbol: BUFFER_SIZE
  --> main.asm:3

note: `BUFFER_SIZE` is defined here and depends on `ITEM_COUNT`
  --> main.asm:1
note: `ITEM_COUNT` is defined here and depends on undefined symbol `TABLE_END`
  --> main.asm:2
```

This behavior is always enabled. There is no command-line option, setting, or
verbosity requirement.

## Decisions

- Keep the current primary error message and location for compatibility.
- Add structured diagnostic notes; do not build explanation text ad hoc at each
  expression call site.
- Explain only failures. A valid undefined external in ELF object mode remains a
  relocation and produces no warning or note.
- Trace dependencies through deferred global constants and aliases.
- For local symbols, include the global-label scope in which lookup was
  attempted and distinguish "not defined in this scope" from a missing global.
- Detect and report dependency cycles directly instead of exhausting the current
  100-pass deferred-resolution loop.
- Report one deterministic path to each terminal missing symbol. Do not print
  every possible graph path by default.
- Bound diagnostics to prevent pathological source from producing unbounded
  output. The proposed initial limit is 16 links per chain and 8 terminal
  missing symbols per failed expression; report omitted counts.
- Deduplicate identical notes when both assembler passes encounter the same
  failure.
- Preserve source names and line numbers through includes and macro expansion.
  When available, also identify the macro invocation that produced the line.

## Scope

Always-on dependency explanations apply wherever an expression must resolve
while assembling:

- constant and alias definitions;
- `.storage` length and filler;
- `.align` and `.org`;
- `.if` conditions and `.loop` counts;
- `.byte`, `.word`, `.dword`, `.text`, `.print`, and `.error` expressions;
- `.incbin` offset and length;
- ROM-mode instruction operands;
- object-mode operands that require absolute constants rather than relocations;
- `.pack` block sizes and constants used to measure reservations;
- macro-expanded expressions and expressions originating in included files.

Valid object-mode symbol references that become relocations are outside the
failure path and must remain silent.

## Current behavior

### Error representation

`AsmError` in `crates/v6_core/src/diagnostics.rs` currently stores one message,
one optional location, and one optional source line. It cannot represent
secondary locations or notes.

### Expression evaluation

`eval_expr` in `crates/v6_core/src/expr.rs` returns a string-only `AsmError` as
soon as it reaches an unresolved `Expr::Symbol` or `Expr::LocalSymbol`. Recursive
unary and binary evaluation preserves neither the expression role nor a symbol
dependency path.

`eval_expr_reloc` intentionally carries undefined globals as relocation targets.
That behavior is correct and must not be diagnosed unless the caller invokes
`RelocValue::require_constant()` in a context that cannot accept relocation.

### Deferred constants

`SymbolInfo` in `crates/v6_core/src/symbols.rs` already retains the essential
provenance for unresolved global constants:

- original symbol name;
- expression AST;
- definition file and line;
- current value/resolution state;
- scope ID and section affiliation.

`resolve_deferred_constants` in `assembler.rs` repeatedly retries unresolved
expressions. If no progress is possible, it reports a flat list of unresolved
symbols. It does not distinguish a missing leaf from a cycle or reconstruct the
definition chain.

### Evaluation contexts

Assembler call sites know whether an expression is a storage length, alignment,
condition, instruction operand, or another role, but that context is currently
lost when `eval_expr` returns an error.

## Diagnostic model

### Notes on `AsmError`

Extend `AsmError` with ordered secondary diagnostics:

```rust
pub struct DiagnosticNote {
    pub message: String,
    pub location: Option<SourceLocation>,
    pub source_line: Option<String>,
}

pub struct AsmError {
    pub location: Option<SourceLocation>,
    pub message: String,
    pub source_line: Option<String>,
    pub notes: Vec<DiagnosticNote>,
}
```

Add builders such as `with_note` and `with_note_at`. `Display` renders notes
after the primary diagnostic in insertion order. Existing errors with no notes
retain their current output exactly.

Do not store dependency graphs inside `AsmError`; construct a concise set of
notes at the point where a failure becomes final.

### Structured evaluation failure

Introduce an internal expression failure type rather than parsing symbol names
back out of `AsmError.message`:

```rust
enum EvalFailureKind {
    UndefinedGlobal { name: String },
    UndefinedLocal { name: String, scope_id: usize },
    DivisionByZero,
    ModuloByZero,
    RelocationNotConstant { target: RelocTarget },
    UnsupportedRelocExpression,
}

struct EvalFailure {
    kind: EvalFailureKind,
}
```

Expression evaluation should return structured failures internally. Public
wrappers may continue returning `AsmResult<T>` after converting a failure into
an `AsmError`. This avoids making every caller understand dependency graphs and
keeps arithmetic diagnostics unchanged.

### Evaluation context

Use a compact context enum when converting a terminal evaluation failure:

```rust
enum EvalContext {
    Constant { name: String },
    Directive { name: &'static str, operand: &'static str },
    Conditional { name: &'static str },
    Instruction { mnemonic: String, operand_index: usize },
    PackStorage { block_label: Option<String> },
    Other,
}
```

The context improves the primary or first note, for example:

```text
note: required while evaluating `.storage` length
```

It must not replace the dependency chain or require every call site to create
custom prose.

## Dependency graph

### Nodes

A graph node represents a symbol identity, not just a spelling:

- global: normalized name;
- local: normalized name plus scope ID;
- macro-local: normalized expanded identity;
- terminal missing symbol: requested identity plus attempted scope.

Each defined node carries its `SymbolInfo` definition location and expression.

### Edges

Collect direct symbol references from each deferred expression AST. Add a small
AST walker in `expr.rs`:

```rust
fn collect_symbol_refs(expr: &Expr, output: &mut Vec<SymbolRef>);
```

`SymbolRef` retains global versus local spelling. Preserve first appearance
order and deduplicate repeated references within one expression.

Edges are derived from retained expressions when diagnostics are needed; they
do not need to be maintained incrementally during successful assembly.

### Traversal

When an expression fails on symbol `X`:

1. Look up `X` in the symbol table using the same global/local/macro scope rules
   as normal evaluation.
2. If no definition exists, emit a terminal missing-symbol note.
3. If `X` has a deferred expression, emit its definition note and recursively
   inspect unresolved dependencies in source-expression order.
4. Stop at the first terminal missing dependency for the concise default path.
5. Track visited node identities. Revisiting a node emits a cycle diagnostic.
6. Enforce depth and terminal-count limits and emit an omission note when a
   limit is reached.

Traversal must be deterministic across runs and independent of `HashMap`
iteration order.

### Cycles

Replace the generic unresolved list for cycles with a direct explanation:

```text
error: Circular symbol dependency involving `A`
  --> main.asm:1

note: `A` depends on `B`
  --> main.asm:1
note: `B` depends on `A`, completing the cycle
  --> main.asm:2
```

A source file may contain both cycles and genuinely missing leaves. Diagnose
both categories, ordered by definition location and symbol name.

## Local-symbol scopes

Local dependency lookup must use the scope captured when the defining
expression was parsed, not the assembler's later current scope. `SymbolInfo`
already stores `scope_id`; deferred pack constants also retain a preceding global
label for scope restoration.

A missing local should produce a scope-aware note:

```text
note: local symbol `@data_end` is not defined in the scope of `global_states`
  --> runtime_data.asm:60
```

If the same local spelling exists in another scope, optionally add:

```text
note: a symbol named `@data_end` exists under `lv_data_init_tbl`, but local
      labels do not cross global-label scopes
```

This suggestion is useful only when an alternate definition is found; do not
scan or speculate otherwise.

## Deferred-resolution integration

Refactor `resolve_deferred_constants` into two stages:

1. Keep the current fixed-point resolution behavior for successful forward
   references, but terminate when a full iteration makes no progress.
2. On no progress, build dependency explanations for the remaining symbols,
   detect cycles, and return one `AsmError` with ordered notes.

Remove the arbitrary 100-iteration limit once progress detection and cycle
analysis guarantee termination. A long acyclic chain may still require many
fixed-point passes; a future optimization may topologically resolve it, but that
is not required for this feature.

When an immediate caller such as `.storage` encounters an unresolved deferred
constant before end-of-pass resolution, enrich that immediate error by walking
the retained definition. This is the behavior needed for the motivating pack
failure.

## Object-mode behavior

Object mode distinguishes two cases:

1. A symbol reference is legal as a relocation. Assembly succeeds and no
   diagnostic is emitted.
2. A context requires an assembly-time constant, such as `.storage`, `.align`,
   `.if`, or a non-relocatable operator. If the value depends on an external or
   unresolved symbol, assembly fails and the normal always-on dependency notes
   are attached.

Do not label all undefined object symbols as errors or warnings. The deciding
factor is whether the current context can represent the dependency in ELF.

For `require_constant()` failures, retain the referenced symbol identity in the
structured failure so the dependency walker can continue through a deferred
alias where possible.

## Source and macro provenance

`SourceLine` already carries file, line, and optional macro context. Diagnostic
construction should attach the original source line when available and add a
macro-expansion note such as:

```text
note: while expanding macro `ALLOC_BUFFER`
  --> main.asm:42
```

Do not duplicate the same included-file or macro note for every dependency link.
Prefer the closest useful invocation context.

## Implementation phases

### Phase 1 - Structured notes

1. Add `DiagnosticNote` and `AsmError.notes`.
2. Add note builder methods and deterministic rendering.
3. Preserve byte-for-byte display output for errors without notes.
4. Add unit tests for primary errors, multiple notes, note locations, source
   lines, and caret rendering.

Exit gate: existing diagnostics are unchanged unless notes are explicitly
attached.

### Phase 2 - Structured expression failures

1. Add internal `EvalFailure` and `EvalFailureKind`.
2. Split plain expression evaluation into an internal structured evaluator and
   the existing public `AsmResult` wrapper.
3. Preserve arithmetic and relocation error wording.
4. Retain global/local identity and attempted local scope for undefined symbols.
5. Add expression-level tests for undefined globals, undefined locals, and
   non-symbol arithmetic failures.

Exit gate: callers can reliably identify the missing symbol without parsing an
error string.

### Phase 3 - Dependency extraction and traversal

1. Add the expression symbol-reference walker.
2. Add symbol-table lookup helpers that return unresolved definitions even when
   `value` is `None`.
3. Implement deterministic dependency traversal with visited-node cycle
   detection and output limits.
4. Add local-scope names to symbol metadata if scope IDs cannot currently be
   mapped back to their owning global labels.
5. Unit-test missing leaves, multi-step chains, branches, repeated references,
   local scopes, and cycles.

Exit gate: a failed symbol can be converted into an ordered explanation chain.

### Phase 4 - Deferred constants

1. Integrate dependency traversal into `resolve_deferred_constants`.
2. Replace the flat `Unresolved symbols` list with missing-leaf and cycle notes.
3. Remove the 100-pass cap in favor of explicit no-progress termination.
4. Preserve successful forward-reference and section-relative alias behavior in
   both ROM and object modes.

Exit gate: unresolved aliases always identify their terminal missing dependency
or cycle.

### Phase 5 - Immediate expression contexts

1. Add `EvalContext` and one assembler helper that evaluates a required constant
   and enriches symbol failures.
2. Route `.storage`, `.align`, `.org`, `.if`, `.loop`, and pack measurement
   through the helper first.
3. Extend it to data directives, instruction operands, print/error expressions,
   and `.incbin` arguments.
4. Ensure pass 1 and pass 2 do not duplicate notes.
5. Preserve valid object relocations without diagnostics.

Exit gate: every constant-required expression site emits an always-on dependency
explanation when resolution fails.

### Phase 6 - Includes and macros

1. Carry source-line and macro invocation provenance into dependency notes.
2. Add the alternate-local-scope hint when a same-spelled local exists elsewhere.
3. Verify include paths and original line numbers remain correct after
   preprocessing and `force_once` deduplication.
4. Deduplicate repeated macro/include provenance notes.

Exit gate: explanations remain actionable across real multi-file and macro-heavy
projects.

### Phase 7 - Documentation and compatibility

1. Document always-on dependency notes in `docs/cli.md` under diagnostics; make
   clear that no flag is required.
2. Add examples to the syntax or directives documentation for deferred
   constants and local-scope mistakes.
3. Update CLI snapshot/help tests only if diagnostic formatting is represented
   there; do not add a diagnostic toggle.
4. Run the engine build that motivated the feature as a manual integration
   check.

Exit gate: public docs match the unconditional behavior and examples.

## Implementation checklist

### Phase 1 - Structured notes

- [ ] Add `DiagnosticNote` with message, optional location, and optional source
  line to `diagnostics.rs`.
- [ ] Add `notes: Vec<DiagnosticNote>` to `AsmError` and initialize it in every
  existing constructor.
- [ ] Add `with_note` and `with_note_at` builders.
- [ ] Render notes after the primary error, preserving the current exact output
  when no notes exist.
- [ ] Add diagnostics unit tests for ordered notes, note locations, source
  excerpts, and carets.

### Phase 2 - Structured expression failures

- [ ] Add internal `EvalFailure` and `EvalFailureKind` in `expr.rs`.
- [ ] Represent undefined global and local symbols without parsing display text.
- [ ] Preserve the current messages for division by zero, modulo by zero, and
  unsupported relocatable expressions.
- [ ] Keep public `eval_expr` and `eval_expr_reloc` APIs returning `AsmError`.
- [ ] Add expression tests for undefined global, undefined local, and arithmetic
  failures.

### Phase 3 - Dependency traversal

- [ ] Add an expression walker that collects global and local symbol references
  in first-use order.
- [ ] Add symbol-table lookup APIs for unresolved global and local definitions.
- [ ] Preserve definition-time local scope identity and map it to its owning
  global label for diagnostics.
- [ ] Implement deterministic traversal from a failed symbol to a missing leaf.
- [ ] Detect cycles with visited node identities, not symbol spelling alone.
- [ ] Enforce the initial 16-link and 8-terminal diagnostic limits.
- [ ] Add tests for chains, branches, repeated dependencies, local scopes,
  cycles, and truncation.

### Phase 4 - Deferred constants

- [ ] Refactor `resolve_deferred_constants` to stop after an iteration with no
  resolution progress.
- [ ] Convert unresolved deferred aliases into dependency notes.
- [ ] Emit direct cycle diagnostics for cyclic aliases.
- [ ] Remove the arbitrary 100-pass retry limit.
- [ ] Verify forward references and section-relative aliases still resolve in
  ROM and object modes.

### Phase 5 - Constant-required expressions

- [ ] Add `EvalContext` and one assembler helper that turns an expression
  failure into an error with dependency notes.
- [ ] Route `.storage`, `.align`, `.org`, `.if`, `.loop`, and `.pack` sizing
  through the helper.
- [ ] Route data directives, ROM instruction operands, `.print`, `.error`, and
  `.incbin` arguments through the helper.
- [ ] Attach source and macro context without duplicating notes in pass 1 and
  pass 2.
- [ ] Verify valid object-mode external references remain relocations with no
  diagnostic output.

### Phase 6 - Provenance and documentation

- [ ] Add include-file source excerpts and macro-invocation notes where useful.
- [ ] Add the alternate-local-scope hint only when a same-named local exists in
  another scope.
- [ ] Add regression fixtures for multi-file constants, macros, `.pack`, and
  the sound-style `GC_TASKS` dependency failure.
- [ ] Document always-on dependency notes in `docs/cli.md` and directive/syntax
  documentation.
- [ ] Run `cargo test -p v6_core`, `cargo test --workspace`, and representative
  v6gel builds with music enabled and disabled.

## Tests

### Diagnostic rendering

- Existing one-location errors render exactly as before.
- Notes render after the primary source excerpt in stable order.
- Notes may independently carry locations and source excerpts.
- Diagnostic limits report omitted links without truncating the primary error.

### Dependency chains

- `A = MISSING` reports `A -> MISSING`.
- `A = B`, `B = C`, `C = MISSING` reports the complete chain.
- A binary expression with two missing leaves reports both, up to the terminal
  limit, in expression order.
- Repeated use of one dependency emits one note.
- A chain longer than 16 links ends with an explicit truncation note.

### Cycles

- Self-cycle: `A = A + 1`.
- Two-node cycle: `A = B`, `B = A`.
- Longer cycle reached through another alias.
- Mixed cycle and missing-leaf graph.
- Cycle output is deterministic across repeated runs.

### Contexts

- Undefined dependency in `.storage` length.
- Undefined dependency in `.align` and `.org`.
- Undefined dependency in `.if` and `.loop`.
- Undefined dependency in data emission and ROM instruction operands.
- Undefined dependency while measuring a `.pack` block.
- Object-mode absolute-only expression rejects an unresolved dependency with
  notes.
- Object-mode `call external_symbol` succeeds silently with a relocation.

### Scopes and preprocessing

- Missing `@local` reports its owning global scope.
- Same local spelling in another scope produces the targeted hint.
- Included constant definition reports the included file and line.
- Macro-expanded failure reports both generated source and invocation context.
- `.setting force_once, true` does not lose definition provenance.

### Regression workload

Add a fixture modeled on the engine sound case:

```asm
GC_TASKS = 14
WORD_LEN = 2
V6_GC_TASK_SPS_LEN = GC_TASKS * WORD_LEN
.storage V6_GC_TASK_SPS_LEN
```

Also retain a deliberately broken version without `GC_TASKS` and assert that the
error names both `V6_GC_TASK_SPS_LEN` and its missing dependency `GC_TASKS`.

## Verification

- `cargo test -p v6_core`.
- `cargo test --workspace`.
- Assemble a multi-file ROM fixture with a three-level missing dependency.
- Assemble an ELF object with a valid external relocation and confirm there is
  no diagnostic output.
- Assemble an ELF object whose `.storage` depends on an external and confirm the
  always-on explanation is emitted.
- Build `C:/Work/Programming/v6gel/samples/00_empty` with music both enabled and
  disabled to verify successful builds remain quiet.

## Risks and mitigations

### Diagnostic noise

Always-on explanations can become large. Keep the primary error concise,
deduplicate links, report one deterministic path per terminal leaf, and enforce
explicit depth/terminal limits. Do not hide the feature behind a flag.

### Valid ELF externals

Treating every unresolved object symbol as a problem would produce false
positives. Attach explanations only when a caller requires an assembly-time
constant and assembly fails; representable relocations remain silent.

### Scope drift

Deferred local expressions may be diagnosed after the assembler has entered a
different scope. Store and use the definition-time scope identity rather than
current mutable assembler state.

### Error-string coupling

Parsing `AsmError.message` to discover a symbol would be fragile. Use structured
expression failures and convert them to display text only at the diagnostic
boundary.

### Nondeterministic graph output

The symbol table uses hash maps. Traverse dependencies in expression order and
sort independent unresolved roots by source location then normalized name.

### Performance

Successful assemblies should pay only for retaining metadata already stored and
small structured failures. Build dependency graphs lazily after a real failure;
do not continuously trace every successful lookup.

## Non-goals

- Emitting warnings for valid undefined ELF externals.
- Suggesting similarly spelled global symbol names in the first implementation.
- Changing symbol visibility, forward-reference, local-scope, or relocation
  semantics.
- Recovering from an undefined-symbol error and continuing code generation.
- Adding a CLI flag, `.setting`, environment variable, or verbosity gate for
  dependency explanations.
- Printing the entire dependency graph when one bounded actionable chain is
  sufficient.

## Success criteria

1. Every undefined-symbol assembly failure identifies the immediate failed use.
2. If that symbol has a retained definition, notes trace to the terminal missing
   dependency or a dependency cycle.
3. Local-symbol errors identify the attempted scope.
4. Valid ELF external references remain silent and continue producing
   relocations.
5. Diagnostics are deterministic, bounded, and always enabled.
6. Existing errors without dependency notes retain their current formatting.
7. The full workspace test suite and representative v6gel builds pass.
