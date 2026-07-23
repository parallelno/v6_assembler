use std::path::{Path, PathBuf};

use crate::diagnostics::{AsmError, AsmResult, SourceLocation};
use crate::encoding::{Encoding, EncodingCase, EncodingType};
use crate::expr::{eval_expr, eval_expr_reloc, ByteOp, Expr, RelocValue, SymValue};
use crate::instructions::{encode_instruction, ParsedOperand};
use crate::lexer::tokenize_line;
use crate::object::section::{Reloc, RelocKind, Section};
use crate::parser::{self, Directive, ParsedLine, PackKind, PrintArg, TextItem};
use crate::preprocessor::{SourceLine, OriginalSource, expand_macro, parse_macro_invocation};
use crate::project::CpuMode;
use crate::symbols::SymbolTable;

const MAX_LOOP_ITERATIONS: usize = 100_000;

/// Output backend selected for an assembly run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Fully-located flat ROM image (default).
    Rom,
    /// Relocatable ELF32 object (`ET_REL`, `EM_V6C`).
    Obj,
}

/// Object-mode state: sections, the active section, and symbol bindings.
pub struct ObjectState {
    /// Sections in creation order; index 0 is the default `.text`.
    pub sections: Vec<Section>,
    /// Index of the currently active section.
    pub active: usize,
    /// Names (original case) marked `.globl`/`.global`.
    pub globls: Vec<String>,
    /// Names (original case) marked `.weak`.
    pub weaks: Vec<String>,
    /// Names (original case) marked `.local`.
    pub locals: Vec<String>,
    /// Set when `.globl *` is used — export all module-level symbols.
    pub glob_all: bool,
}

impl ObjectState {
    fn new() -> Self {
        Self {
            sections: vec![Section::new(
                ".text",
                Section::default_flags(".text"),
                Section::default_type(".text"),
            )],
            active: 0,
            globls: Vec::new(),
            weaks: Vec::new(),
            locals: Vec::new(),
            glob_all: false,
        }
    }

    /// Find an existing section by name or create a new one, returning its index.
    fn section_index(&mut self, name: &str) -> usize {
        if let Some(idx) = self.sections.iter().position(|s| s.name == name) {
            idx
        } else {
            let flags = Section::default_flags(name);
            let sh_type = Section::default_type(name);
            self.sections.push(Section::new(name, flags, sh_type));
            self.sections.len() - 1
        }
    }
}

/// Output buffer for assembled code (sparse 64KB address space)
pub struct OutputBuffer {
    data: Vec<Option<u8>>,
    min_addr: Option<u16>,
    max_addr: Option<u16>,
    write_count: usize,
}

impl OutputBuffer {
    pub fn new() -> Self {
        Self {
            data: vec![None; 65536],
            min_addr: None,
            max_addr: None,
            write_count: 0,
        }
    }

    pub fn write_byte(&mut self, addr: u16, byte: u8) {
        self.data[addr as usize] = Some(byte);
        self.min_addr = Some(self.min_addr.map_or(addr, |m: u16| m.min(addr)));
        self.max_addr = Some(self.max_addr.map_or(addr, |m: u16| m.max(addr)));
        self.write_count += 1;
    }

    pub fn write_bytes(&mut self, start_addr: u16, bytes: &[u8]) {
        for (i, &b) in bytes.iter().enumerate() {
            self.write_byte(start_addr.wrapping_add(i as u16), b);
        }
    }

    /// Extract the contiguous ROM bytes
    pub fn extract_rom(&self) -> Vec<u8> {
        let min = match self.min_addr {
            Some(a) => a as usize,
            None => return Vec::new(),
        };
        let max = match self.max_addr {
            Some(a) => a as usize,
            None => return Vec::new(),
        };
        let mut rom = Vec::with_capacity(max - min + 1);
        for i in min..=max {
            rom.push(self.data[i].unwrap_or(0));
        }
        rom
    }

    pub fn min_addr(&self) -> Option<u16> {
        self.min_addr
    }

    pub fn max_addr(&self) -> Option<u16> {
        self.max_addr
    }

    pub fn read_byte(&self, addr: u16) -> Option<u8> {
        self.data[addr as usize]
    }

    pub fn write_count(&self) -> usize {
        self.write_count
    }
}

/// A single entry for the listing file
#[derive(Debug, Clone)]
pub struct ListingLine {
    pub file: String,
    pub line_num: usize,
    pub text: String,
    pub addr: u16,
    pub byte_count: usize,
    /// If this line is from a macro expansion
    pub macro_expansion: bool,
}

/// Assembler settings that can be modified by .setting
#[derive(Debug, Clone)]
pub struct AssemblerSettings {
    pub optional_enabled: bool,
    /// In object mode, controls how `.optional` blocks are emitted:
    /// `Some(true)` = each block in its own section (link-time pruning),
    /// `Some(false)` = assemble-time pruning, `None` = format default
    /// (sections in object mode, prune otherwise).
    pub optional_sections: Option<bool>,
    /// Mirrors the preprocessor-level `force_once` setting for test assertions.
    pub force_once: bool,
}

impl Default for AssemblerSettings {
    fn default() -> Self {
        Self {
            optional_enabled: true,
            optional_sections: None,
            force_once: false,
        }
    }
}

/// Effective strategy for handling an `.optional` block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OptionalStrategy {
    /// Assemble-time pruning of unreferenced blocks.
    Prune,
    /// Emit each block into its own ELF section (object mode only).
    Sections,
    /// Always include every block in the current section.
    IncludeAll,
}

/// What to do with a resolved `.optional` block.
enum OptionalAction {
    /// Drop the block (assemble-time pruning).
    Skip,
    /// Process the block in the current section.
    IncludeHere,
    /// Process the block in a dedicated section with the given name.
    IncludeInSection(String),
}

/// The kind of dedicated section an `.optional` block maps to in `sections`
/// mode, based on its contents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OptionalBlockClass {
    /// Contains instructions/macros — `.text.<label>`.
    Code,
    /// Emits initialized bytes — `.data.<label>`.
    Data,
    /// Only reserves space via `.storage` (no bytes) — `.bss.<label>`.
    Bss,
}

/// The main assembler context
pub struct Assembler {
    pub symbols: SymbolTable,
    pub output: OutputBuffer,
    pub listing_data: Vec<ListingLine>,
    pub original_sources: Vec<OriginalSource>,
    pub pc: u16,
    pub cpu_mode: CpuMode,
    pub encoding: Encoding,
    pub settings: AssemblerSettings,
    pub errors: Vec<AsmError>,
    pub quiet: bool,
    /// Selected output backend.
    pub output_format: OutputFormat,
    /// Object-mode state (only meaningful when `output_format == Obj`).
    pub obj: ObjectState,
    project_dir: PathBuf,

    // Tracking for .optional blocks
    _optional_stack: Vec<OptionalBlock>,
    _optional_blocks: Vec<OptionalBlockInfo>,

    /// Pending section alignment from a `.align` directive, applied to the next
    /// section entered via `.section`/`.optional` (object mode). 1 = none.
    pending_align: u32,

    /// True once the `.pack` arena has been laid out in the current pass.
    pack_laid_out: bool,
    /// Absolute base address of the pack arena (ROM mode).
    pack_arena_base: u16,
    /// Total size of the pack arena in bytes.
    pack_arena_size: u32,

    // Loop/if expansion depth tracking
    macro_depth: usize,
}

struct OptionalBlock {
    _start_idx: usize,
    _symbols_defined: Vec<String>,
}

struct OptionalBlockInfo {
    _start_line_idx: usize,
    _end_line_idx: usize,
    _symbols_defined: Vec<String>,
}

impl Assembler {
    pub fn new(cpu_mode: CpuMode, project_dir: PathBuf) -> Self {
        Self {
            symbols: SymbolTable::new(),
            output: OutputBuffer::new(),
            listing_data: Vec::new(),
            original_sources: Vec::new(),
            pc: 0,
            cpu_mode,
            encoding: Encoding::default(),
            settings: AssemblerSettings::default(),
            errors: Vec::new(),
            quiet: false,
            output_format: OutputFormat::Rom,
            obj: ObjectState::new(),
            project_dir,
            _optional_stack: Vec::new(),
            _optional_blocks: Vec::new(),
            pending_align: 1,
            pack_laid_out: false,
            pack_arena_base: 0,
            pack_arena_size: 0,
            macro_depth: 0,
        }
    }

    pub fn project_dir(&self) -> &Path {
        &self.project_dir
    }

    /// Assemble preprocessed source lines (two-pass)
    pub fn assemble(&mut self, lines: &[SourceLine]) -> AsmResult<()> {
        // Pass 1: Collect symbols and sizes
        self.reset_object_state();
        self.pass1(lines)?;

        // Resolve deferred constants
        self.resolve_deferred_constants()?;

        // Pass 2: Generate code
        self.symbols.reset_for_pass2();
        self.symbols.reset_macro_call_count();
        self.pc = 0;
        self.encoding = Encoding::default();
        self.reset_object_state();
        self.pass2(lines)?;

        Ok(())
    }

    /// Reset object-mode sections/bindings to their initial state. Section
    /// creation order is deterministic, so section indices recorded for symbols
    /// in pass 1 remain valid in pass 2.
    fn reset_object_state(&mut self) {
        if self.output_format == OutputFormat::Obj {
            self.obj = ObjectState::new();
        }
        self.pending_align = 1;
        self.pack_laid_out = false;
        self.pack_arena_base = 0;
        self.pack_arena_size = 0;
    }

    fn pass1(&mut self, lines: &[SourceLine]) -> AsmResult<()> {
        self.process_lines_pass1(lines)
    }

    fn process_lines_pass1(&mut self, lines: &[SourceLine]) -> AsmResult<()> {
        let mut i = 0;
        while i < lines.len() {
            let line = &lines[i];

            if let Some((macro_name, args)) = parse_macro_invocation(&line.text, &self.symbols) {
                self.expand_macro_pass1(line, &macro_name, &args)
                    .map_err(|e| e.ensure_location(&line.file, line.line_num))?;
                i += 1;
                continue;
            }

            let tokens = tokenize_line(&line.text, &line.file, line.line_num)
                .map_err(|e| e.ensure_location(&line.file, line.line_num))?;
            if tokens.is_empty() {
                i += 1;
                continue;
            }

            let parsed = parser::parse_line(&tokens, self.cpu_mode)
                .map_err(|e| e.ensure_location(&line.file, line.line_num))?;
            if parsed.len() == 1 {
                if let Some(control) = Self::control_directive(&parsed[0]) {
                    match control {
                        ControlDirective::If(expr) => {
                            let end = self.find_matching_block_end(lines, i, BlockKind::If)?;
                            if self.eval_expr(expr)
                                .map_err(|e| e.ensure_location(&line.file, line.line_num))? != 0 {
                                self.process_lines_pass1(&lines[i + 1..end])?;
                            }
                            i = end + 1;
                            continue;
                        }
                        ControlDirective::Loop(expr) => {
                            let end = self.find_matching_block_end(lines, i, BlockKind::Loop)?;
                            let count = self.eval_expr(expr)
                                .map_err(|e| e.ensure_location(&line.file, line.line_num))?;
                            if count < 0 {
                                return Err(AsmError::new("Loop count must be non-negative")
                                    .ensure_location(&line.file, line.line_num));
                            }
                            if count as usize > MAX_LOOP_ITERATIONS {
                                return Err(AsmError::new(format!(
                                    "Loop iteration count exceeded {}",
                                    MAX_LOOP_ITERATIONS
                                )).ensure_location(&line.file, line.line_num));
                            }
                            for _ in 0..count as usize {
                                self.process_lines_pass1(&lines[i + 1..end])?;
                            }
                            i = end + 1;
                            continue;
                        }
                        ControlDirective::Optional => {
                            let end = self.find_matching_block_end(lines, i, BlockKind::Optional)?;
                            match self.resolve_optional_block(lines, i + 1, end)? {
                                OptionalAction::Skip => {}
                                OptionalAction::IncludeHere => {
                                    self.process_lines_pass1(&lines[i + 1..end])?;
                                }
                                OptionalAction::IncludeInSection(name) => {
                                    let saved = self.enter_optional_section(&name);
                                    self.process_lines_pass1(&lines[i + 1..end])?;
                                    self.leave_optional_section(saved);
                                }
                            }
                            i = end + 1;
                            continue;
                        }
                        ControlDirective::Pack => {
                            let end = self.find_matching_block_end(lines, i, BlockKind::Pack)?;
                            self.ensure_pack_laid_out(lines)
                                .map_err(|e| e.ensure_location(&line.file, line.line_num))?;
                            i = end + 1;
                            continue;
                        }
                        ControlDirective::EndIf
                        | ControlDirective::EndLoop
                        | ControlDirective::EndOptional
                        | ControlDirective::EndPack => {
                            i += 1;
                            continue;
                        }
                    }
                }
            }

            self.process_parsed_line_pass1(line, &parsed)
                .map_err(|e| e.ensure_location(&line.file, line.line_num))?;
            i += 1;
        }
        Ok(())
    }

    fn process_parsed_line_pass1(&mut self, line: &SourceLine, parsed: &[ParsedLine]) -> AsmResult<()> {
        // Capture the scope in effect before any Label on this line changes it.
        // This lets a ConstDef on the same line as a label resolve @locals that
        // were defined in the scope that just ended.
        // e.g.  label: = @local_in_previous_scope
        let pre_label_scope = self.symbols.current_scope();

        for item in parsed {
            match item {
                ParsedLine::Empty => {}
                ParsedLine::Label(name) => {
                    self.define_label_here(name, &line.file, line.line_num)?;
                }
                ParsedLine::LocalLabel(name) => {
                    self.define_local_label_here(name, &line.file, line.line_num)?;
                }
                ParsedLine::ConstDef { name, is_local, expr } => {
                    if self.output_format == OutputFormat::Obj && !*is_local {
                        // Object mode: evaluate the alias relocatably already in
                        // pass 1 so a section-relative RHS (`foo = label + 1`)
                        // records its section affiliation immediately. Otherwise
                        // a reference appearing *earlier* in the file than the
                        // alias definition would be encoded in pass 2 before the
                        // section is known and get baked as an absolute address
                        // instead of a relocation.
                        let active = self.obj.active;
                        let rv = {
                            let symbols = &self.symbols;
                            let pc = self.pc;
                            eval_expr_reloc(expr, &|sym, is_local_sym| {
                                let info = if is_local_sym {
                                    symbols.get_local_info(sym)
                                        .or_else(|| symbols.get_local_info_in_scope(sym, pre_label_scope))
                                } else {
                                    symbols.get_global_info(sym)
                                };
                                match info {
                                    Some(i) => {
                                        if let Some(sec) = i.section {
                                            SymValue::Section { index: sec, offset: i.value.unwrap_or(0) }
                                        } else if let Some(v) = i.value {
                                            SymValue::Absolute(v)
                                        } else {
                                            SymValue::Undefined
                                        }
                                    }
                                    None => SymValue::Undefined,
                                }
                            }, pc, active)
                        };
                        match rv {
                            Ok(rv) if self.symbols.is_mutable(name) => {
                                self.symbols.update_variable(name, rv.addend)?;
                            }
                            Ok(rv) => match &rv.target {
                                Some(crate::object::section::RelocTarget::Section(sec)) => {
                                    self.symbols.define_constant_in_section(name, rv.addend, *sec, &line.file, line.line_num)?;
                                }
                                None => {
                                    self.symbols.define_constant(name, rv.addend, &line.file, line.line_num)?;
                                }
                                Some(crate::object::section::RelocTarget::Symbol(_)) => {
                                    // RHS references a symbol not yet defined
                                    // (forward reference / external). Defer and
                                    // retry after pass 1.
                                    self.symbols.define_constant_deferred(name, expr.clone(), &line.file, line.line_num)?;
                                }
                            },
                            Err(_) => {
                                self.symbols.define_constant_deferred(name, expr.clone(), &line.file, line.line_num)?;
                            }
                        }
                    } else {
                        // ROM mode or local constant: plain evaluation, defer on
                        // forward reference. The resolver checks globals, the
                        // current scope, and also the pre-label scope so that
                        // `lbl: = @local` works when @local was defined in the
                        // scope that ended at `lbl:`.
                        let resolver = |sym: &str| -> Option<i64> {
                            self.symbols.resolve(sym)
                                .or_else(|| self.symbols.resolve_local(sym))
                                .or_else(|| self.symbols.resolve_local_in_scope(sym, pre_label_scope))
                        };
                        match eval_expr(expr, &resolver, self.pc) {
                            Ok(val) => {
                                if *is_local {
                                    self.symbols.define_local_constant(name, val, &line.file, line.line_num)?;
                                } else if self.symbols.is_mutable(name) {
                                    self.symbols.update_variable(name, val)?;
                                } else {
                                    self.symbols.define_constant(name, val, &line.file, line.line_num)?;
                                }
                            }
                            Err(_) => {
                                // Defer evaluation
                                if !*is_local {
                                    self.symbols.define_constant_deferred(name, expr.clone(), &line.file, line.line_num)?;
                                }
                            }
                        }
                    }
                    }
                ParsedLine::VarDef { name, expr } => {
                    let resolver = |sym: &str| -> Option<i64> {
                        self.symbols.resolve(sym)
                    };
                    if let Ok(val) = eval_expr(expr, &resolver, self.pc) {
                        self.symbols.define_variable(name, val, &line.file, line.line_num)?;
                    }
                }
                ParsedLine::Instruction { mnemonic, operands, .. } => {
                    let size = self.instruction_size(mnemonic, operands)?;
                    self.advance_pc(size as u16);
                }
                ParsedLine::Directive(dir) => {
                    self.process_directive_pass1(dir, &line.file, line.line_num)?;
                }
            }
        }
        Ok(())
    }

    fn process_directive_pass1(&mut self, dir: &Directive, file: &str, line_num: usize) -> AsmResult<()> {
        match dir {
            Directive::Org(expr) => {
                if self.output_format == OutputFormat::Obj {
                    return Err(AsmError::new(
                        "absolute .org is not allowed in object mode; the linker chooses addresses",
                    ));
                }
                let val = self.eval_expr(expr)?;
                self.pc = val as u16;
            }
            Directive::Align(expr) => {
                let alignment = self.eval_expr(expr)? as u16;
                if alignment > 0 {
                    let mask = alignment - 1;
                    if self.pc & mask != 0 {
                        let target = (self.pc | mask).wrapping_add(1);
                        self.advance_pc(target.wrapping_sub(self.pc));
                    }
                    if self.output_format == OutputFormat::Obj {
                        self.obj.sections[self.obj.active].set_align(alignment as u32);
                        self.pending_align = alignment as u32;
                    }
                }
            }
            Directive::Storage { length, filler: _ } => {
                let len = self.eval_expr(length)? as u16;
                self.advance_pc(len);
            }
            Directive::Byte(exprs) => {
                self.advance_pc(exprs.len() as u16);
            }
            Directive::Word(exprs) => {
                self.advance_pc((exprs.len() * 2) as u16);
            }
            Directive::Dword(exprs) => {
                self.advance_pc((exprs.len() * 4) as u16);
            }
            Directive::Text(items) => {
                let byte_count = self.text_byte_count(items);
                self.advance_pc(byte_count as u16);
            }
            Directive::Section(name) => {
                self.switch_section(name);
            }
            Directive::Globl(names) => self.apply_binding(names, BindingKind::Global),
            Directive::GloblAll => { if self.output_format == OutputFormat::Obj { self.obj.glob_all = true; } }
            Directive::Weak(names) => self.apply_binding(names, BindingKind::Weak),
            Directive::Local(names) => self.apply_binding(names, BindingKind::Local),
            Directive::Encoding { enc_type, case } => {
                if let Some(et) = EncodingType::from_str(enc_type) {
                    self.encoding.encoding_type = et;
                }
                if let Some(c) = case {
                    if let Some(ec) = EncodingCase::from_str(c) {
                        self.encoding.case = ec;
                    }
                }
            }
            Directive::Setting(pairs) => {
                for (key, val) in pairs {
                    if key.eq_ignore_ascii_case("optional") {
                        self.apply_optional_setting(val);
                    } else if key.eq_ignore_ascii_case("force_once") {
                        self.settings.force_once |= !val.eq_ignore_ascii_case("false");
                    }
                }
            }
            Directive::If(_) | Directive::EndIf | Directive::Loop(_) | Directive::EndLoop => {
            }
            Directive::Optional | Directive::EndOptional => {
            }
            Directive::Pack(_) | Directive::EndPack => {
            }
            Directive::IncBin { path, offset, length } => {
                // For pass 1 we need to know the size
                let resolved = self.resolve_file_path(path)?;
                let file_len = std::fs::metadata(&resolved)
                    .map_err(|e| AsmError::new(format!("Cannot read {}: {}", path, e)))?
                    .len() as usize;
                let off = offset.as_ref().map(|e| self.eval_expr(e).unwrap_or(0) as usize).unwrap_or(0);
                let len = length.as_ref().map(|e| self.eval_expr(e).unwrap_or(0) as usize).unwrap_or(file_len - off);
                self.advance_pc(len as u16);
            }
            Directive::FileSize { name, path } => {
                let resolved = self.resolve_file_path(path)?;
                let size = std::fs::metadata(&resolved)
                    .map_err(|e| AsmError::new(format!("Cannot stat {}: {}", path, e)))?
                    .len() as i64;
                if !name.is_empty() {
                    self.symbols.define_constant(name, size, file, line_num)?;
                }
            }
            Directive::Include(_) => {
                // Should have been expanded already
            }
            Directive::MacroDef { .. } | Directive::EndMacro => {
                // Should have been collected already
            }
            Directive::Print(_) | Directive::Error(_) => {
                // Only processed in pass 2
            }
        }
        Ok(())
    }

    fn pass2(&mut self, lines: &[SourceLine]) -> AsmResult<()> {
        self.process_lines_pass2(lines)
    }

    fn process_lines_pass2(&mut self, lines: &[SourceLine]) -> AsmResult<()> {
        let mut i = 0;
        while i < lines.len() {
            let line = &lines[i];

            let pc_before = self.pc;
            let wc_before = self.output.write_count();

            if let Some((macro_name, args)) = parse_macro_invocation(&line.text, &self.symbols) {
                let macro_start_pc = self.pc;
                self.expand_macro_pass2(line, &macro_name, &args)
                    .map_err(|e| e.ensure_location(&line.file, line.line_num))?;
                self.listing_data.push(ListingLine {
                    file: line.file.clone(),
                    line_num: line.line_num,
                    text: line.text.clone(),
                    addr: macro_start_pc,
                    byte_count: 0,
                    macro_expansion: line.macro_context.is_some(),
                });
                i += 1;
                continue;
            }

            let tokens = tokenize_line(&line.text, &line.file, line.line_num)
                .map_err(|e| e.ensure_location(&line.file, line.line_num))?;
            if tokens.is_empty() {
                self.listing_data.push(ListingLine {
                    file: line.file.clone(),
                    line_num: line.line_num,
                    text: line.text.clone(),
                    addr: pc_before,
                    byte_count: 0,
                    macro_expansion: line.macro_context.is_some(),
                });
                i += 1;
                continue;
            }

            let parsed = parser::parse_line(&tokens, self.cpu_mode)
                .map_err(|e| e.ensure_location(&line.file, line.line_num))?;
            if parsed.len() == 1 {
                if let Some(control) = Self::control_directive(&parsed[0]) {
                    match control {
                        ControlDirective::If(expr) => {
                            let end = self.find_matching_block_end(lines, i, BlockKind::If)?;
                            self.listing_data.push(ListingLine {
                                file: line.file.clone(),
                                line_num: line.line_num,
                                text: line.text.clone(),
                                addr: self.pc,
                                byte_count: 0,
                                macro_expansion: line.macro_context.is_some(),
                            });
                            if self.eval_expr(expr)
                                .map_err(|e| e.ensure_location(&line.file, line.line_num))? != 0 {
                                self.process_lines_pass2(&lines[i + 1..end])?;
                            }
                            // Record the closing .endif
                            let end_line = &lines[end];
                            self.listing_data.push(ListingLine {
                                file: end_line.file.clone(),
                                line_num: end_line.line_num,
                                text: end_line.text.clone(),
                                addr: self.pc,
                                byte_count: 0,
                                macro_expansion: end_line.macro_context.is_some(),
                            });
                            i = end + 1;
                            continue;
                        }
                        ControlDirective::Loop(expr) => {
                            let end = self.find_matching_block_end(lines, i, BlockKind::Loop)?;
                            let count = self.eval_expr(expr)
                                .map_err(|e| e.ensure_location(&line.file, line.line_num))?;
                            if count < 0 {
                                return Err(AsmError::new("Loop count must be non-negative")
                                    .ensure_location(&line.file, line.line_num));
                            }
                            if count as usize > MAX_LOOP_ITERATIONS {
                                return Err(AsmError::new(format!(
                                    "Loop iteration count exceeded {}",
                                    MAX_LOOP_ITERATIONS
                                )).ensure_location(&line.file, line.line_num));
                            }
                            self.listing_data.push(ListingLine {
                                file: line.file.clone(),
                                line_num: line.line_num,
                                text: line.text.clone(),
                                addr: self.pc,
                                byte_count: 0,
                                macro_expansion: line.macro_context.is_some(),
                            });
                            for _ in 0..count as usize {
                                self.process_lines_pass2(&lines[i + 1..end])?;
                            }
                            // Record the closing .endl/.endloop
                            let end_line = &lines[end];
                            self.listing_data.push(ListingLine {
                                file: end_line.file.clone(),
                                line_num: end_line.line_num,
                                text: end_line.text.clone(),
                                addr: self.pc,
                                byte_count: 0,
                                macro_expansion: end_line.macro_context.is_some(),
                            });
                            i = end + 1;
                            continue;
                        }
                        ControlDirective::Optional => {
                            let end = self.find_matching_block_end(lines, i, BlockKind::Optional)?;
                            self.listing_data.push(ListingLine {
                                file: line.file.clone(),
                                line_num: line.line_num,
                                text: line.text.clone(),
                                addr: self.pc,
                                byte_count: 0,
                                macro_expansion: line.macro_context.is_some(),
                            });
                            match self.resolve_optional_block(lines, i + 1, end)? {
                                OptionalAction::Skip => {}
                                OptionalAction::IncludeHere => {
                                    self.process_lines_pass2(&lines[i + 1..end])?;
                                }
                                OptionalAction::IncludeInSection(name) => {
                                    let saved = self.enter_optional_section(&name);
                                    self.process_lines_pass2(&lines[i + 1..end])?;
                                    self.leave_optional_section(saved);
                                }
                            }
                            // Record the closing .endoptional
                            let end_line = &lines[end];
                            self.listing_data.push(ListingLine {
                                file: end_line.file.clone(),
                                line_num: end_line.line_num,
                                text: end_line.text.clone(),
                                addr: self.pc,
                                byte_count: 0,
                                macro_expansion: end_line.macro_context.is_some(),
                            });
                            i = end + 1;
                            continue;
                        }
                        ControlDirective::Pack => {
                            let end = self.find_matching_block_end(lines, i, BlockKind::Pack)?;
                            self.ensure_pack_laid_out(lines)
                                .map_err(|e| e.ensure_location(&line.file, line.line_num))?;
                            // Record the .pack and .endpack lines in the listing.
                            self.listing_data.push(ListingLine {
                                file: line.file.clone(),
                                line_num: line.line_num,
                                text: line.text.clone(),
                                addr: self.pc,
                                byte_count: 0,
                                macro_expansion: line.macro_context.is_some(),
                            });
                            let end_line = &lines[end];
                            self.listing_data.push(ListingLine {
                                file: end_line.file.clone(),
                                line_num: end_line.line_num,
                                text: end_line.text.clone(),
                                addr: self.pc,
                                byte_count: 0,
                                macro_expansion: end_line.macro_context.is_some(),
                            });
                            i = end + 1;
                            continue;
                        }
                        ControlDirective::EndIf
                        | ControlDirective::EndLoop
                        | ControlDirective::EndOptional
                        | ControlDirective::EndPack => {
                            self.listing_data.push(ListingLine {
                                file: line.file.clone(),
                                line_num: line.line_num,
                                text: line.text.clone(),
                                addr: self.pc,
                                byte_count: 0,
                                macro_expansion: line.macro_context.is_some(),
                            });
                            i += 1;
                            continue;
                        }
                    }
                }
            }

            self.process_parsed_line_pass2(line, &parsed)
                .map_err(|e| e.ensure_location(&line.file, line.line_num))?;
            let byte_count = self.output.write_count() - wc_before;
            self.listing_data.push(ListingLine {
                file: line.file.clone(),
                line_num: line.line_num,
                text: line.text.clone(),
                addr: pc_before,
                byte_count,
                macro_expansion: line.macro_context.is_some(),
            });
            i += 1;
        }
        Ok(())
    }

    fn process_parsed_line_pass2(&mut self, line: &SourceLine, parsed: &[ParsedLine]) -> AsmResult<()> {
        // Capture scope before any Label on this line changes it (see pass1 comment).
        let pre_label_scope = self.symbols.current_scope();

        for item in parsed {
            match item {
                ParsedLine::Empty => {}
                ParsedLine::Label(name) => {
                    self.define_label_here(name, &line.file, line.line_num)?;
                }
                ParsedLine::LocalLabel(name) => {
                    self.define_local_label_here(name, &line.file, line.line_num)?;
                }
                ParsedLine::ConstDef { name, is_local, expr } => {
                    if self.output_format == OutputFormat::Obj && !*is_local {
                        // In object mode use the relocatable evaluator so that
                        // section-relative results (e.g. `foo = bar + 1`) are
                        // recorded with their section affiliation.  Plain
                        // constants (ABS) fall through to define_constant.
                        let active = self.obj.active;
                        let rv = {
                            let symbols = &self.symbols;
                            let pc = self.pc;
                            eval_expr_reloc(expr, &|sym, is_local_sym| {
                                let info = if is_local_sym {
                                    symbols.get_local_info(sym)
                                        .or_else(|| symbols.get_local_info_in_scope(sym, pre_label_scope))
                                } else {
                                    symbols.get_global_info(sym)
                                };
                                match info {
                                    Some(i) => {
                                        if let Some(sec) = i.section {
                                            SymValue::Section { index: sec, offset: i.value.unwrap_or(0) }
                                        } else if let Some(v) = i.value {
                                            SymValue::Absolute(v)
                                        } else {
                                            SymValue::Undefined
                                        }
                                    }
                                    None => SymValue::Undefined,
                                }
                            }, pc, active)?
                        };
                        let val = rv.addend;
                        match &rv.target {
                            Some(crate::object::section::RelocTarget::Section(sec)) => {
                                self.symbols.define_constant_in_section(name, val, *sec, &line.file, line.line_num)?;
                            }
                            _ => {
                                // Absolute or undefined — fall back to regular constant
                                if self.symbols.is_mutable(name) {
                                    self.symbols.update_variable(name, val)?;
                                } else {
                                    self.symbols.define_constant(name, val, &line.file, line.line_num)?;
                                }
                            }
                        }
                    } else {
                        // ROM mode or local constant: plain evaluation.
                        let val = {
                            let symbols = &self.symbols;
                            let pc = self.pc;
                            eval_expr(expr, &|sym| {
                                symbols.resolve(sym)
                                    .or_else(|| symbols.resolve_local(sym))
                                    .or_else(|| symbols.resolve_local_in_scope(sym, pre_label_scope))
                            }, pc)?
                        };
                        if *is_local {
                            self.symbols.define_local_constant(name, val, &line.file, line.line_num)?;
                        } else {
                            if self.symbols.is_mutable(name) {
                                self.symbols.update_variable(name, val)?;
                            } else if self.symbols.exists(name) {
                                self.symbols.define_constant(name, val, &line.file, line.line_num)?;
                            } else {
                                self.symbols.define_constant(name, val, &line.file, line.line_num)?;
                            }
                        }
                    }
                }
                ParsedLine::VarDef { name, expr } => {
                    let val = self.eval_expr(expr)?;
                    if self.symbols.exists(name) {
                        let _ = self.symbols.update_variable(name, val);
                    } else {
                        self.symbols.define_variable(name, val, &line.file, line.line_num)?;
                    }
                }
                ParsedLine::Instruction { mnemonic, operands, expressions } => {
                    self.emit_instruction(mnemonic, operands, expressions)?;
                }
                ParsedLine::Directive(dir) => {
                    self.process_directive_pass2(dir, &line.file, line.line_num)?;
                }
            }
        }
        Ok(())
    }

    fn expand_macro_pass1(&mut self, line: &SourceLine, macro_name: &str, args: &[String]) -> AsmResult<()> {
        if self.macro_depth >= 32 {
            return Err(AsmError::new("Macro expansion depth exceeded 32 levels"));
        }
        let macro_def = self.symbols.get_macro(macro_name).unwrap().clone();
        let call_idx = self.symbols.macro_call_count() + 1;
        let expanded = expand_macro(&macro_def, args, call_idx, &line.file, line.line_num)?;
        self.symbols.begin_macro_expansion(macro_name);
        self.macro_depth += 1;
        let result = self.process_lines_pass1(&expanded);
        self.macro_depth -= 1;
        self.symbols.end_macro_expansion();
        result
    }

    fn expand_macro_pass2(&mut self, line: &SourceLine, macro_name: &str, args: &[String]) -> AsmResult<()> {
        if self.macro_depth >= 32 {
            return Err(AsmError::new("Macro expansion depth exceeded 32 levels"));
        }
        let macro_def = self.symbols.get_macro(macro_name).unwrap().clone();
        let call_idx = self.symbols.macro_call_count() + 1;
        let expanded = expand_macro(&macro_def, args, call_idx, &line.file, line.line_num)?;
        self.symbols.begin_macro_expansion(macro_name);
        self.macro_depth += 1;
        let result = self.process_lines_pass2(&expanded);
        self.macro_depth -= 1;
        self.symbols.end_macro_expansion();
        result
    }

    fn control_directive<'a>(parsed: &'a ParsedLine) -> Option<ControlDirective<'a>> {
        match parsed {
            ParsedLine::Directive(Directive::If(expr)) => Some(ControlDirective::If(expr)),
            ParsedLine::Directive(Directive::EndIf) => Some(ControlDirective::EndIf),
            ParsedLine::Directive(Directive::Loop(expr)) => Some(ControlDirective::Loop(expr)),
            ParsedLine::Directive(Directive::EndLoop) => Some(ControlDirective::EndLoop),
            ParsedLine::Directive(Directive::Optional) => Some(ControlDirective::Optional),
            ParsedLine::Directive(Directive::EndOptional) => Some(ControlDirective::EndOptional),
            ParsedLine::Directive(Directive::Pack(_)) => Some(ControlDirective::Pack),
            ParsedLine::Directive(Directive::EndPack) => Some(ControlDirective::EndPack),
            _ => None,
        }
    }

    fn find_matching_block_end(&self, lines: &[SourceLine], start: usize, kind: BlockKind) -> AsmResult<usize> {
        let mut depth = 0usize;
        for (idx, line) in lines.iter().enumerate().skip(start) {
            // A macro invocation can never be a block directive. Skip it so we
            // don't try to parse `FOO()` as an instruction/expression while
            // scanning for the matching block terminator.
            if parse_macro_invocation(&line.text, &self.symbols).is_some() {
                continue;
            }
            let tokens = tokenize_line(&line.text, &line.file, line.line_num)
                .map_err(|e| e.ensure_location(&line.file, line.line_num))?;
            if tokens.is_empty() {
                continue;
            }
            let parsed = parser::parse_line(&tokens, self.cpu_mode)
                .map_err(|e| e.ensure_location(&line.file, line.line_num))?;
            if parsed.len() != 1 {
                continue;
            }
            if let Some(control) = Self::control_directive(&parsed[0]) {
                match (kind, control) {
                    (BlockKind::If, ControlDirective::If(_))
                    | (BlockKind::Loop, ControlDirective::Loop(_))
                    | (BlockKind::Optional, ControlDirective::Optional)
                    | (BlockKind::Pack, ControlDirective::Pack) => {
                        depth += 1;
                    }
                    (BlockKind::If, ControlDirective::EndIf)
                    | (BlockKind::Loop, ControlDirective::EndLoop)
                    | (BlockKind::Optional, ControlDirective::EndOptional)
                    | (BlockKind::Pack, ControlDirective::EndPack) => {
                        depth -= 1;
                        if depth == 0 {
                            return Ok(idx);
                        }
                    }
                    _ => {}
                }
            }
        }
        Err(AsmError::new(format!("Missing {}", kind.end_directive_name()))
            .ensure_location(&lines[start].file, lines[start].line_num))
    }

    /// The effective strategy for handling `.optional` blocks given the current
    /// settings and output format.
    fn optional_strategy(&self) -> OptionalStrategy {
        if !self.settings.optional_enabled {
            return OptionalStrategy::IncludeAll;
        }
        let want_sections = self
            .settings
            .optional_sections
            .unwrap_or(self.output_format == OutputFormat::Obj);
        if want_sections && self.output_format == OutputFormat::Obj {
            OptionalStrategy::Sections
        } else {
            OptionalStrategy::Prune
        }
    }

    /// Apply a `.setting optional, <val>` value. Accepts `true`/`false`,
    /// `prune`/`sections`, and `disabled`.
    fn apply_optional_setting(&mut self, val: &str) {
        if val.eq_ignore_ascii_case("false") || val.eq_ignore_ascii_case("disabled") {
            self.settings.optional_enabled = false;
        } else if val.eq_ignore_ascii_case("prune") {
            self.settings.optional_enabled = true;
            self.settings.optional_sections = Some(false);
        } else if val.eq_ignore_ascii_case("sections") {
            self.settings.optional_enabled = true;
            self.settings.optional_sections = Some(true);
        } else {
            // `true` and any other value enable pruning with the format default.
            self.settings.optional_enabled = true;
        }
    }

    /// Decide how to handle the `.optional` block covering
    /// `lines[block_start..block_end]`. `block_start` is the index just after
    /// the `.optional` directive (so `block_start - 1` is the directive line).
    fn resolve_optional_block(
        &self,
        lines: &[SourceLine],
        block_start: usize,
        block_end: usize,
    ) -> AsmResult<OptionalAction> {
        let block = &lines[block_start..block_end];
        let defined = self.collect_optional_block_symbols(block)?;
        if defined.is_empty() {
            let loc = &lines[block_start.saturating_sub(1)];
            return Err(AsmError::new(
                "an .optional/.function block must define at least one label or constant"
                    .to_string(),
            )
            .ensure_location(&loc.file, loc.line_num));
        }

        match self.optional_strategy() {
            OptionalStrategy::IncludeAll => Ok(OptionalAction::IncludeHere),
            OptionalStrategy::Prune => {
                if self.optional_block_referenced(lines, block_start, block_end, &defined)? {
                    Ok(OptionalAction::IncludeHere)
                } else {
                    Ok(OptionalAction::Skip)
                }
            }
            OptionalStrategy::Sections => {
                match self.optional_block_section_label(lines, block_start, block_end)? {
                    Some(label) => {
                        let prefix = match self.optional_block_class(block)? {
                            OptionalBlockClass::Code => ".text.",
                            OptionalBlockClass::Data => ".data.",
                            OptionalBlockClass::Bss => ".bss.",
                        };
                        Ok(OptionalAction::IncludeInSection(format!("{prefix}{label}")))
                    }
                    None => Ok(OptionalAction::IncludeHere),
                }
            }
        }
    }

    /// Returns true if any symbol defined in the block is referenced by a line
    /// outside the block (the assemble-time pruning test).
    fn optional_block_referenced(
        &self,
        lines: &[SourceLine],
        block_start: usize,
        block_end: usize,
        defined: &[String],
    ) -> AsmResult<bool> {
        for (idx, line) in lines.iter().enumerate() {
            if idx >= block_start && idx < block_end {
                continue;
            }
            let tokens = tokenize_line(&line.text, &line.file, line.line_num)?;
            if tokens.is_empty() {
                continue;
            }
            for token in &tokens {
                if let crate::lexer::Token::Identifier(name) = &token.value {
                    if defined.iter().any(|defined_name| defined_name == name) {
                        return Ok(true);
                    }
                }
            }
        }

        Ok(false)
    }

    /// The label used to name the block's dedicated section in `sections` mode.
    /// This is the first label *defined in the block* that is *referenced from
    /// outside* the block (the label that keeps the block alive). If no defined
    /// label is referenced externally, falls back to the first defined label.
    fn optional_block_section_label(
        &self,
        lines: &[SourceLine],
        block_start: usize,
        block_end: usize,
    ) -> AsmResult<Option<String>> {
        // Labels defined in the block, in definition order.
        let mut defined_labels = Vec::new();
        for line in &lines[block_start..block_end] {
            if parse_macro_invocation(&line.text, &self.symbols).is_some() {
                continue;
            }
            let tokens = tokenize_line(&line.text, &line.file, line.line_num)?;
            if tokens.is_empty() {
                continue;
            }
            let parsed = parser::parse_line(&tokens, self.cpu_mode)?;
            for item in parsed {
                if let ParsedLine::Label(name) = item {
                    defined_labels.push(name);
                }
            }
        }
        if defined_labels.is_empty() {
            return Ok(None);
        }

        // Identifiers referenced by lines outside the block.
        let mut referenced: std::collections::HashSet<String> = std::collections::HashSet::new();
        for (idx, line) in lines.iter().enumerate() {
            if idx >= block_start && idx < block_end {
                continue;
            }
            let tokens = tokenize_line(&line.text, &line.file, line.line_num)?;
            for token in &tokens {
                if let crate::lexer::Token::Identifier(name) = &token.value {
                    referenced.insert(name.clone());
                }
            }
        }

        // First defined label referenced externally; fall back to the first
        // defined label if none are referenced.
        let chosen = defined_labels
            .iter()
            .find(|name| referenced.contains(name.as_str()))
            .cloned()
            .unwrap_or_else(|| defined_labels[0].clone());
        Ok(Some(chosen))
    }

    /// Returns true if the block contains any instruction (or macro invocation,
    /// which expands to instructions), meaning it belongs in a `.text.*`
    /// section rather than a data `.data.*` section.
    fn optional_block_class(&self, lines: &[SourceLine]) -> AsmResult<OptionalBlockClass> {
        let mut has_initialized_data = false;
        let mut has_storage = false;
        for line in lines {
            // A macro invocation expands to instructions: treat as code.
            if parse_macro_invocation(&line.text, &self.symbols).is_some() {
                return Ok(OptionalBlockClass::Code);
            }
            let tokens = tokenize_line(&line.text, &line.file, line.line_num)?;
            if tokens.is_empty() {
                continue;
            }
            let parsed = parser::parse_line(&tokens, self.cpu_mode)?;
            for item in parsed {
                match item {
                    ParsedLine::Instruction { .. } => return Ok(OptionalBlockClass::Code),
                    ParsedLine::Directive(dir) => match dir {
                        // `.storage` without a filler reserves space but emits
                        // no bytes — eligible for `.bss`.
                        Directive::Storage { filler: None, .. } => has_storage = true,
                        // `.storage` with a filler and all other emitting
                        // directives produce initialized bytes — `.data`.
                        Directive::Storage { filler: Some(_), .. }
                        | Directive::Byte(_)
                        | Directive::Word(_)
                        | Directive::Dword(_)
                        | Directive::Text(_)
                        | Directive::IncBin { .. }
                        | Directive::Align(_) => has_initialized_data = true,
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
        if has_initialized_data {
            Ok(OptionalBlockClass::Data)
        } else if has_storage {
            Ok(OptionalBlockClass::Bss)
        } else {
            // Only labels/constants: keep with initialized data semantics.
            Ok(OptionalBlockClass::Data)
        }
    }

    /// Switch to a dedicated section for an `.optional` block, returning the
    /// saved `(active section, pc)` to restore afterwards. Object mode only.
    fn enter_optional_section(&mut self, name: &str) -> (usize, u16) {
        let saved = (self.obj.active, self.pc);
        self.switch_section(name);
        saved
    }

    /// Restore the active section and pc saved by `enter_optional_section`.
    fn leave_optional_section(&mut self, saved: (usize, u16)) {
        let (active, pc) = saved;
        self.obj.active = active;
        self.pc = pc;
    }

    fn collect_optional_block_symbols(&self, lines: &[SourceLine]) -> AsmResult<Vec<String>> {
        let mut names = Vec::new();
        for line in lines {
            // A macro invocation defines no symbols and would fail to parse as
            // an instruction, so skip it.
            if parse_macro_invocation(&line.text, &self.symbols).is_some() {
                continue;
            }
            let tokens = tokenize_line(&line.text, &line.file, line.line_num)?;
            if tokens.is_empty() {
                continue;
            }
            let parsed = parser::parse_line(&tokens, self.cpu_mode)?;
            for item in parsed {
                match item {
                    ParsedLine::Label(name) => names.push(name),
                    ParsedLine::ConstDef { name, is_local: false, .. } => names.push(name),
                    ParsedLine::VarDef { name, .. } => names.push(name),
                    _ => {}
                }
            }
        }
        Ok(names)
    }

    fn process_directive_pass2(&mut self, dir: &Directive, file: &str, line_num: usize) -> AsmResult<()> {
        match dir {
            Directive::Org(expr) => {
                if self.output_format == OutputFormat::Obj {
                    return Err(AsmError::new(
                        "absolute .org is not allowed in object mode; the linker chooses addresses",
                    ));
                }
                let val = self.eval_expr(expr)?;
                self.pc = val as u16;
            }
            Directive::Align(expr) => {
                let alignment = self.eval_expr(expr)? as u16;
                if alignment > 0 {
                    let mask = alignment - 1;
                    if self.pc & mask != 0 {
                        let target = (self.pc | mask).wrapping_add(1);
                        let pad = target.wrapping_sub(self.pc);
                        self.out_reserve(pad, Some(0));
                    }
                    if self.output_format == OutputFormat::Obj {
                        self.obj.sections[self.obj.active].set_align(alignment as u32);
                        self.pending_align = alignment as u32;
                    }
                }
            }
            Directive::Storage { length, filler } => {
                let len = self.eval_expr(length)? as u16;
                let fill = filler.as_ref().map(|e| self.eval_expr(e)).transpose()?.map(|v| v as u8);
                self.out_reserve(len, fill);
            }
            Directive::Byte(exprs) => {
                for expr in exprs {
                    self.emit_byte_expr(expr)?;
                }
            }
            Directive::Word(exprs) => {
                for expr in exprs {
                    self.emit_word_expr(expr)?;
                }
            }
            Directive::Dword(exprs) => {
                for expr in exprs {
                    if self.output_format == OutputFormat::Obj {
                        let rv = self.eval_reloc(expr)?;
                        let val = rv.require_constant().map_err(|_| AsmError::new(
                            "relocatable .dword is not supported (no 32-bit V6C relocation)",
                        ))? as u32;
                        for i in 0..4 {
                            self.out_emit_byte(((val >> (i * 8)) & 0xFF) as u8);
                        }
                    } else {
                        let val = self.eval_expr(expr)? as u32;
                        for i in 0..4 {
                            self.out_emit_byte(((val >> (i * 8)) & 0xFF) as u8);
                        }
                    }
                }
            }
            Directive::Text(items) => {
                let bytes = self.encode_text_items(items);
                self.out_emit_bytes(&bytes);
            }
            Directive::Section(name) => {
                self.switch_section(name);
            }
            Directive::Globl(names) => self.apply_binding(names, BindingKind::Global),
            Directive::GloblAll => { if self.output_format == OutputFormat::Obj { self.obj.glob_all = true; } }
            Directive::Weak(names) => self.apply_binding(names, BindingKind::Weak),
            Directive::Local(names) => self.apply_binding(names, BindingKind::Local),
            Directive::Encoding { enc_type, case } => {
                if let Some(et) = EncodingType::from_str(enc_type) {
                    self.encoding.encoding_type = et;
                }
                if let Some(c) = case {
                    if let Some(ec) = EncodingCase::from_str(c) {
                        self.encoding.case = ec;
                    }
                }
            }
            Directive::Setting(pairs) => {
                for (key, val) in pairs {
                    if key.eq_ignore_ascii_case("optional") {
                        self.apply_optional_setting(val);
                    } else if key.eq_ignore_ascii_case("force_once") {
                        self.settings.force_once |= !val.eq_ignore_ascii_case("false");
                    }
                }
            }
            Directive::Print(args) if !self.quiet => {
                let mut output = String::new();
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 { output.push(' '); }
                    match arg {
                        PrintArg::Str(s) => output.push_str(s),
                        PrintArg::Expr(expr) => {
                            let val = self.eval_expr(expr)?;
                            output.push_str(&val.to_string());
                        }
                    }
                }
                eprintln!("{}", output);
            }
            Directive::Print(_) => {}
            Directive::Error(args) => {
                let mut output = String::new();
                for (i, arg) in args.iter().enumerate() {
                    if i > 0 { output.push(' '); }
                    match arg {
                        PrintArg::Str(s) => output.push_str(s),
                        PrintArg::Expr(expr) => {
                            let val = self.eval_expr(expr)?;
                            output.push_str(&val.to_string());
                        }
                    }
                }
                return Err(AsmError::new(output)
                    .with_location(SourceLocation {
                        file: file.to_string(),
                        line: line_num,
                        col: 1,
                    }));
            }
            Directive::IncBin { path, offset, length } => {
                let resolved = self.resolve_file_path(path)?;
                let data = std::fs::read(&resolved)
                    .map_err(|e| AsmError::new(format!("Cannot read {}: {}", path, e)))?;
                let off = offset.as_ref().map(|e| self.eval_expr(e).unwrap_or(0) as usize).unwrap_or(0);
                let len = length.as_ref().map(|e| self.eval_expr(e).unwrap_or(0) as usize).unwrap_or(data.len() - off);
                let slice = &data[off..off + len];
                let bytes: Vec<u8> = slice.to_vec();
                self.out_emit_bytes(&bytes);
            }
            Directive::FileSize { name, path } => {
                let resolved = self.resolve_file_path(path)?;
                let size = std::fs::metadata(&resolved)
                    .map_err(|e| AsmError::new(format!("Cannot stat {}: {}", path, e)))?
                    .len() as i64;
                if !name.is_empty() {
                    if self.symbols.exists(name) {
                        let _ = self.symbols.update_variable(name, size);
                    } else {
                        self.symbols.define_constant(name, size, file, line_num)?;
                    }
                }
            }
            Directive::If(_) | Directive::EndIf | Directive::Loop(_) | Directive::EndLoop => {}
            Directive::Optional | Directive::EndOptional => {}
            Directive::Pack(_) | Directive::EndPack => {}
            Directive::Include(_) | Directive::MacroDef { .. } | Directive::EndMacro => {}
        }
        Ok(())
    }

    fn emit_instruction(&mut self, mnemonic: &str, operands: &[ParsedOperand], expressions: &[Expr]) -> AsmResult<()> {
        let mut encoded = encode_instruction(mnemonic, operands, self.cpu_mode)?;

        if self.output_format == OutputFormat::Obj {
            let active = self.obj.active;
            let base = self.obj.sections[active].size; // offset of the opcode
            let mut expr_idx = 0;
            if encoded.has_imm8 && expr_idx < expressions.len() {
                let rv = self.eval_reloc(&expressions[expr_idx])?;
                if let Some(target) = rv.target.clone() {
                    let kind = match rv.byte_op {
                        ByteOp::Lo => RelocKind::Lo8,
                        ByteOp::Hi => RelocKind::Hi8,
                        ByteOp::None => RelocKind::Abs8,
                    };
                    self.obj.sections[active].add_reloc(Reloc {
                        offset: base + 1,
                        kind,
                        target,
                        addend: rv.addend,
                    });
                } else {
                    encoded.bytes[1] = rv.require_constant()? as u8;
                }
                expr_idx += 1;
            }
            if encoded.has_imm16 && expr_idx < expressions.len() {
                let rv = self.eval_reloc(&expressions[expr_idx])?;
                if let Some(target) = rv.target.clone() {
                    let kind = match rv.byte_op {
                        ByteOp::Lo => RelocKind::Lo8,
                        ByteOp::Hi => RelocKind::Hi8,
                        ByteOp::None => RelocKind::Abs16,
                    };
                    self.obj.sections[active].add_reloc(Reloc {
                        offset: base + 1,
                        kind,
                        target,
                        addend: rv.addend,
                    });
                } else {
                    let val = rv.require_constant()? as u16;
                    encoded.bytes[1] = (val & 0xFF) as u8;
                    encoded.bytes[2] = ((val >> 8) & 0xFF) as u8;
                }
            }
            self.out_emit_bytes(&encoded.bytes);
            return Ok(());
        }

        // Fill in immediate values from expressions
        let mut expr_idx = 0;
        if encoded.has_imm8 && expr_idx < expressions.len() {
            let val = self.eval_expr(&expressions[expr_idx])? as u8;
            encoded.bytes[1] = val;
            expr_idx += 1;
        }
        if encoded.has_imm16 && expr_idx < expressions.len() {
            let val = self.eval_expr(&expressions[expr_idx])? as u16;
            encoded.bytes[1] = (val & 0xFF) as u8;
            encoded.bytes[2] = ((val >> 8) & 0xFF) as u8;
        }

        self.output.write_bytes(self.pc, &encoded.bytes);
        self.pc = self.pc.wrapping_add(encoded.size as u16);
        Ok(())
    }

    fn instruction_size(&self, mnemonic: &str, operands: &[ParsedOperand]) -> AsmResult<usize> {
        let encoded = encode_instruction(mnemonic, operands, self.cpu_mode)?;
        Ok(encoded.size)
    }

    /// Emit one byte of data from an expression (`.byte`/`DB` element),
    /// generating a relocation in object mode if needed.
    fn emit_byte_expr(&mut self, expr: &Expr) -> AsmResult<()> {
        if self.output_format == OutputFormat::Obj {
            let active = self.obj.active;
            let off = self.obj.sections[active].size;
            let rv = self.eval_reloc(expr)?;
            if let Some(target) = rv.target.clone() {
                let kind = match rv.byte_op {
                    ByteOp::Lo => RelocKind::Lo8,
                    ByteOp::Hi => RelocKind::Hi8,
                    ByteOp::None => RelocKind::Abs8,
                };
                self.obj.sections[active].add_reloc(Reloc { offset: off, kind, target, addend: rv.addend });
                self.out_emit_byte(0);
            } else {
                self.out_emit_byte(rv.require_constant()? as u8);
            }
        } else {
            let val = self.eval_expr(expr)? as u8;
            self.out_emit_byte(val);
        }
        Ok(())
    }

    /// Emit one 16-bit word of data from an expression (`.word`/`DW` element),
    /// generating an `R_V6C_16` relocation in object mode if needed.
    fn emit_word_expr(&mut self, expr: &Expr) -> AsmResult<()> {
        if self.output_format == OutputFormat::Obj {
            let active = self.obj.active;
            let off = self.obj.sections[active].size;
            let rv = self.eval_reloc(expr)?;
            if let Some(target) = rv.target.clone() {
                if rv.byte_op != ByteOp::None {
                    return Err(AsmError::new(
                        "byte operation on a 16-bit data word is not supported",
                    ));
                }
                self.obj.sections[active].add_reloc(Reloc {
                    offset: off,
                    kind: RelocKind::Abs16,
                    target,
                    addend: rv.addend,
                });
                self.out_emit_byte(0);
                self.out_emit_byte(0);
            } else {
                let val = rv.require_constant()? as u16;
                self.out_emit_byte((val & 0xFF) as u8);
                self.out_emit_byte(((val >> 8) & 0xFF) as u8);
            }
        } else {
            let val = self.eval_expr(expr)? as u16;
            self.out_emit_byte((val & 0xFF) as u8);
            self.out_emit_byte(((val >> 8) & 0xFF) as u8);
        }
        Ok(())
    }

    fn eval_expr(&self, expr: &Expr) -> AsmResult<i64> {
        let symbols = &self.symbols;
        eval_expr(expr, &|name| {
            symbols.resolve(name).or_else(|| symbols.resolve_local(name))
        }, self.pc)
    }

    /// Evaluate an expression as a relocatable value (object mode).
    fn eval_reloc(&self, expr: &Expr) -> AsmResult<RelocValue> {
        let symbols = &self.symbols;
        let active = self.obj.active;
        eval_expr_reloc(expr, &|name, is_local| {
            let info = if is_local {
                symbols.get_local_info(name)
            } else {
                symbols.get_global_info(name)
            };
            match info {
                Some(i) => {
                    if let Some(sec) = i.section {
                        SymValue::Section { index: sec, offset: i.value.unwrap_or(0) }
                    } else if let Some(v) = i.value {
                        SymValue::Absolute(v)
                    } else {
                        SymValue::Undefined
                    }
                }
                None => SymValue::Undefined,
            }
        }, self.pc, active)
    }

    /// Emit a single byte to the active output (ROM image or active section).
    fn out_emit_byte(&mut self, b: u8) {
        self.pending_align = 1;
        match self.output_format {
            OutputFormat::Rom => self.output.write_byte(self.pc, b),
            OutputFormat::Obj => self.obj.sections[self.obj.active].push_byte(b),
        }
        self.pc = self.pc.wrapping_add(1);
    }

    /// Emit a slice of bytes to the active output.
    fn out_emit_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.out_emit_byte(b);
        }
    }

    /// Reserve `len` bytes, optionally filling with a byte. With no filler in
    /// ROM mode the location counter just advances (leaving a gap); in object
    /// mode bytes are always materialized (zeros) for PROGBITS sections.
    fn out_reserve(&mut self, len: u16, fill: Option<u8>) {
        self.pending_align = 1;
        match self.output_format {
            OutputFormat::Rom => {
                if let Some(f) = fill {
                    for _ in 0..len {
                        self.output.write_byte(self.pc, f);
                        self.pc = self.pc.wrapping_add(1);
                    }
                } else {
                    self.pc = self.pc.wrapping_add(len);
                }
            }
            OutputFormat::Obj => {
                let active = self.obj.active;
                if self.obj.sections[active].is_nobits() {
                    self.obj.sections[active].size += len as u32;
                } else {
                    let f = fill.unwrap_or(0);
                    for _ in 0..len {
                        self.obj.sections[active].push_byte(f);
                    }
                }
                self.pc = self.pc.wrapping_add(len);
            }
        }
    }

    /// Define a global label at the current location, recording the active
    /// section in object mode.
    fn define_label_here(&mut self, name: &str, file: &str, line: usize) -> AsmResult<()> {
        match self.output_format {
            OutputFormat::Rom => self.symbols.define_label(name, self.pc, file, line),
            OutputFormat::Obj => {
                self.symbols.define_label_in(name, self.pc, Some(self.obj.active), file, line)
            }
        }
    }

    /// Define a local label at the current location.
    fn define_local_label_here(&mut self, name: &str, file: &str, line: usize) -> AsmResult<()> {
        match self.output_format {
            OutputFormat::Rom => self.symbols.define_local_label(name, self.pc, file, line),
            OutputFormat::Obj => {
                self.symbols.define_local_label_in(name, self.pc, Some(self.obj.active), file, line)
            }
        }
    }

    /// Switch to (or create) a section, syncing the location counter. Applies
    /// and clears any pending `.align` so the entered section receives the
    /// alignment requested just before the section/optional directive.
    fn switch_section(&mut self, name: &str) {
        if self.output_format != OutputFormat::Obj {
            return;
        }
        let idx = self.obj.section_index(name);
        self.obj.active = idx;
        if self.pending_align > 1 {
            self.obj.sections[idx].set_align(self.pending_align);
        }
        self.pending_align = 1;
        self.pc = self.obj.sections[idx].size as u16;
    }

    /// Advance the location counter (pass 1 size tracking), keeping the active
    /// section size in sync in object mode.
    fn advance_pc(&mut self, n: u16) {
        self.pending_align = 1;
        self.pc = self.pc.wrapping_add(n);
        if self.output_format == OutputFormat::Obj {
            self.obj.sections[self.obj.active].size += n as u32;
        }
    }

    /// Collect, pack, reserve and assign addresses for every `.pack` block in
    /// `lines`. Runs once per pass (guarded by `self.pack_laid_out`) the first
    /// time a `.pack` directive is reached, so trailing content receives the
    /// correct location counter. Pack blocks are runtime-only reservations: no
    /// bytes are emitted; labels are defined at their packed addresses.
    fn ensure_pack_laid_out(&mut self, lines: &[SourceLine]) -> AsmResult<()> {
        if self.pack_laid_out {
            return Ok(());
        }
        self.pack_laid_out = true;

        let blocks = self.collect_pack_blocks(lines)?;
        if blocks.is_empty() {
            return Ok(());
        }

        let (offsets, arena_size) = compute_pack_offsets(&blocks);

        match self.output_format {
            OutputFormat::Rom => {
                let base = round_up_u32(self.pc as u32, PACK_DOMAIN) as u16;
                for (bi, block) in blocks.iter().enumerate() {
                    let block_base = base as u32 + offsets[bi];
                    for lbl in &block.labels {
                        let addr = (block_base + lbl.offset) as u16;
                        if lbl.is_local {
                            self.symbols.define_local_label(&lbl.name, addr, &block.file, block.line_num)?;
                        } else {
                            self.symbols.define_label(&lbl.name, addr, &block.file, block.line_num)?;
                        }
                    }
                }
                self.pack_arena_base = base;
                self.pack_arena_size = arena_size;
                // Reserve the whole arena so inline content after the first
                // `.pack` resumes past it. No fill: the region is uninitialized.
                self.pc = base.wrapping_add(arena_size as u16);
            }
            OutputFormat::Obj => {
                let sec = self.obj.section_index(".bss.pack");
                self.obj.sections[sec].set_align(PACK_DOMAIN);
                let sec_base = self.obj.sections[sec].size;
                for (bi, block) in blocks.iter().enumerate() {
                    let block_base = sec_base + offsets[bi];
                    for lbl in &block.labels {
                        let off = (block_base + lbl.offset) as u16;
                        if lbl.is_local {
                            self.symbols.define_local_label_in(&lbl.name, off, Some(sec), &block.file, block.line_num)?;
                        } else {
                            self.symbols.define_label_in(&lbl.name, off, Some(sec), &block.file, block.line_num)?;
                        }
                    }
                }
                self.obj.sections[sec].reserve(arena_size);
                self.pack_arena_size = arena_size;
            }
        }

        // Constant definitions inside pack blocks are resolved after every pack
        // label is known, so cross-references within the arena work.
        for block in &blocks {
            for c in &block.consts {
                let resolver = |sym: &str| -> Option<i64> {
                    self.symbols.resolve(sym).or_else(|| self.symbols.resolve_local(sym))
                };
                match eval_expr(&c.expr, &resolver, 0) {
                    Ok(val) => {
                        if c.is_local {
                            self.symbols.define_local_constant(&c.name, val, &block.file, block.line_num)?;
                        } else {
                            self.symbols.define_constant(&c.name, val, &block.file, block.line_num)?;
                        }
                    }
                    Err(_) => {
                        if !c.is_local {
                            self.symbols.define_constant_deferred(&c.name, c.expr.clone(), &block.file, block.line_num)?;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Scan `lines` for every `.pack`/`.endpack` block, validating and
    /// measuring each. Returns the collected blocks in source order.
    fn collect_pack_blocks(&self, lines: &[SourceLine]) -> AsmResult<Vec<PackBlockData>> {
        let mut blocks = Vec::new();
        let mut i = 0;
        while i < lines.len() {
            let line = &lines[i];
            let tokens = match tokenize_line(&line.text, &line.file, line.line_num) {
                Ok(t) => t,
                Err(_) => { i += 1; continue; }
            };
            if tokens.is_empty() { i += 1; continue; }
            let parsed = match parser::parse_line(&tokens, self.cpu_mode) {
                Ok(p) => p,
                Err(_) => { i += 1; continue; }
            };
            let kind = if parsed.len() == 1 {
                if let ParsedLine::Directive(Directive::Pack(k)) = &parsed[0] { Some(*k) } else { None }
            } else { None };

            if let Some(kind) = kind {
                let end = self.find_matching_block_end(lines, i, BlockKind::Pack)?;
                let block = self.collect_one_pack_block(lines, i, end, kind)?;
                blocks.push(block);
                i = end + 1;
            } else {
                i += 1;
            }
        }
        Ok(blocks)
    }

    /// Collect and validate the body of a single pack block spanning lines
    /// `start` (the `.pack`) .. `end` (the `.endpack`).
    fn collect_one_pack_block(&self, lines: &[SourceLine], start: usize, end: usize, kind: PackKind) -> AsmResult<PackBlockData> {
        let start_line = &lines[start];
        let mut size: u32 = 0;
        let mut labels: Vec<PackLabel> = Vec::new();
        let mut consts: Vec<PackConst> = Vec::new();

        for line in &lines[start + 1..end] {
            let tokens = tokenize_line(&line.text, &line.file, line.line_num)
                .map_err(|e| e.ensure_location(&line.file, line.line_num))?;
            if tokens.is_empty() { continue; }
            let parsed = parser::parse_line(&tokens, self.cpu_mode)
                .map_err(|e| e.ensure_location(&line.file, line.line_num))?;
            for item in &parsed {
                match item {
                    ParsedLine::Empty => {}
                    ParsedLine::Label(name) => {
                        labels.push(PackLabel { name: name.clone(), is_local: false, offset: size });
                    }
                    ParsedLine::LocalLabel(name) => {
                        labels.push(PackLabel { name: name.clone(), is_local: true, offset: size });
                    }
                    ParsedLine::ConstDef { name, is_local, expr } => {
                        consts.push(PackConst { name: name.clone(), is_local: *is_local, expr: expr.clone() });
                    }
                    ParsedLine::Directive(Directive::Storage { length, filler }) => {
                        if filler.is_some() {
                            return Err(AsmError::new(".storage inside .pack must not specify a filler")
                                .ensure_location(&line.file, line.line_num));
                        }
                        let len = self.eval_expr(length)
                            .map_err(|e| e.ensure_location(&line.file, line.line_num))?;
                        if len < 0 {
                            return Err(AsmError::new(".storage length must be non-negative")
                                .ensure_location(&line.file, line.line_num));
                        }
                        size += len as u32;
                    }
                    _ => {
                        return Err(AsmError::new(
                            "Only labels, constant assignments and .storage are allowed inside a .pack block")
                            .ensure_location(&line.file, line.line_num));
                    }
                }
            }
        }

        if labels.is_empty() {
            return Err(AsmError::new("A .pack block must define at least one label")
                .ensure_location(&start_line.file, start_line.line_num));
        }
        if size == 0 {
            return Err(AsmError::new("A .pack block must reserve at least one byte")
                .ensure_location(&start_line.file, start_line.line_num));
        }
        if kind == PackKind::Window && size > PACK_DOMAIN {
            return Err(AsmError::new(format!(
                ".pack window block is {} bytes but must not exceed {} bytes",
                size, PACK_DOMAIN))
                .ensure_location(&start_line.file, start_line.line_num));
        }

        Ok(PackBlockData {
            kind,
            size,
            labels,
            consts,
            file: start_line.file.clone(),
            line_num: start_line.line_num,
        })
    }

    /// Apply `.globl`/`.weak`/`.local` bindings, recording original-case names.
    fn apply_binding(&mut self, names: &[String], kind: BindingKind) {
        if self.output_format != OutputFormat::Obj {
            return;
        }
        let list = match kind {
            BindingKind::Global => &mut self.obj.globls,
            BindingKind::Weak => &mut self.obj.weaks,
            BindingKind::Local => &mut self.obj.locals,
        };
        for n in names {
            if !list.iter().any(|e| e.eq_ignore_ascii_case(n)) {
                list.push(n.clone());
            }
        }
    }

    fn resolve_deferred_constants(&mut self) -> AsmResult<()> {
        // Multiple passes until all constants are resolved
        for _ in 0..100 {
            let mut any_resolved = false;
            let unresolved: Vec<_> = self.symbols.all_globals()
                .iter()
                .filter(|(_, info)| info.value.is_none() && info.expr.is_some())
                .map(|(name, info)| (name.clone(), info.expr.clone().unwrap(), info.file.clone(), info.line))
                .collect();

            if unresolved.is_empty() {
                return Ok(());
            }

            for (name, expr, file, line) in unresolved {
                if self.output_format == OutputFormat::Obj {
                    // Object mode: evaluate relocatably so that aliases whose RHS
                    // is a (now-defined) section-relative label record their
                    // section affiliation, producing relocations when referenced.
                    let rv = {
                        let symbols = &self.symbols;
                        eval_expr_reloc(&expr, &|sym, is_local_sym| {
                            let info = if is_local_sym {
                                symbols.get_local_info(sym)
                            } else {
                                symbols.get_global_info(sym)
                            };
                            match info {
                                Some(i) => {
                                    if let Some(sec) = i.section {
                                        SymValue::Section { index: sec, offset: i.value.unwrap_or(0) }
                                    } else if let Some(v) = i.value {
                                        SymValue::Absolute(v)
                                    } else {
                                        SymValue::Undefined
                                    }
                                }
                                None => SymValue::Undefined,
                            }
                        }, 0, 0)
                    };
                    if let Ok(rv) = rv {
                        match &rv.target {
                            None => {
                                self.symbols.define_constant(&name, rv.addend, &file, line)?;
                                any_resolved = true;
                            }
                            Some(crate::object::section::RelocTarget::Section(sec)) => {
                                self.symbols.define_constant_in_section(&name, rv.addend, *sec, &file, line)?;
                                any_resolved = true;
                            }
                            Some(crate::object::section::RelocTarget::Symbol(_)) => {
                                // Still references an undefined symbol; retry.
                            }
                        }
                    }
                } else {
                    let resolver = |sym: &str| -> Option<i64> {
                        self.symbols.resolve(sym)
                    };
                    if let Ok(val) = eval_expr(&expr, &resolver, 0) {
                        self.symbols.define_constant(&name, val, &file, line)?;
                        any_resolved = true;
                    }
                }
            }

            if !any_resolved {
                // Check if there are still unresolved symbols
                let still_unresolved: Vec<_> = self.symbols.all_globals()
                    .iter()
                    .filter(|(_, info)| info.value.is_none())
                    .map(|(_, info)| {
                        if info.file.is_empty() {
                            info.original_name.clone()
                        } else {
                            format!("{} ({}:{})", info.original_name, info.file, info.line)
                        }
                    })
                    .collect();
                if !still_unresolved.is_empty() {
                    return Err(AsmError::new(format!(
                        "Unresolved symbols:\n{}",
                        still_unresolved.iter().map(|s| format!("  {}", s)).collect::<Vec<_>>().join("\n")
                    )));
                }
                break;
            }
        }
        Ok(())
    }

    fn text_byte_count(&self, items: &[TextItem]) -> usize {
        items.iter().map(|item| match item {
            TextItem::Str(s) => s.len(),
            TextItem::Char(_) => 1,
        }).sum()
    }

    fn encode_text_items(&self, items: &[TextItem]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for item in items {
            match item {
                TextItem::Str(s) => {
                    bytes.extend(self.encoding.encode_string(s));
                }
                TextItem::Char(c) => {
                    bytes.push(self.encoding.encode_char(*c));
                }
            }
        }
        bytes
    }

    fn resolve_file_path(&self, path: &str) -> AsmResult<PathBuf> {
        let p = self.project_dir.join(path);
        if p.exists() {
            return Ok(p);
        }
        let p = PathBuf::from(path);
        if p.exists() {
            return Ok(p);
        }
        Err(AsmError::new(format!("Cannot find file: {}", path)))
    }
}

#[derive(Clone, Copy)]
enum BlockKind {
    If,
    Loop,
    Optional,
    Pack,
}

#[derive(Clone, Copy)]
enum BindingKind {
    Global,
    Weak,
    Local,
}

impl BlockKind {
    fn end_directive_name(self) -> &'static str {
        match self {
            BlockKind::If => ".endif",
            BlockKind::Loop => ".endloop",
            BlockKind::Optional => ".endoptional",
            BlockKind::Pack => ".endpack",
        }
    }
}

enum ControlDirective<'a> {
    If(&'a Expr),
    EndIf,
    Loop(&'a Expr),
    EndLoop,
    Optional,
    EndOptional,
    Pack,
    EndPack,
}

/// The 0x100-byte alignment/window domain used by `.pack` blocks.
const PACK_DOMAIN: u32 = 0x100;

/// A label defined inside a pack block, with its offset from the block start.
struct PackLabel {
    name: String,
    is_local: bool,
    offset: u32,
}

/// A constant assignment appearing inside a pack block.
struct PackConst {
    name: String,
    is_local: bool,
    expr: Expr,
}

/// A collected, measured pack block.
struct PackBlockData {
    kind: PackKind,
    size: u32,
    labels: Vec<PackLabel>,
    consts: Vec<PackConst>,
    file: String,
    line_num: usize,
}

fn round_up_u32(x: u32, m: u32) -> u32 {
    ((x + m - 1) / m) * m
}

/// True when a block of `size` bytes placed at `pos` would cross a
/// `PACK_DOMAIN` boundary.
fn pack_straddles(pos: u32, size: u32) -> bool {
    size != 0 && (pos / PACK_DOMAIN) != ((pos + size - 1) / PACK_DOMAIN)
}

/// Try to place a block of `size` bytes in the hole `[hs, he)`. For windows the
/// placement must not straddle a `PACK_DOMAIN` boundary. Returns the chosen
/// position on success.
fn fit_in_hole(hs: u32, he: u32, size: u32, is_window: bool) -> Option<u32> {
    if he - hs < size {
        return None;
    }
    if !is_window || !pack_straddles(hs, size) {
        return Some(hs);
    }
    let p = round_up_u32(hs, PACK_DOMAIN);
    if p + size <= he { Some(p) } else { None }
}

/// Compute packed offsets for every block. Mirrors `temp/pack/pack.py`:
/// align anchors are laid out first in descending-size order (rounding the
/// cursor to `PACK_DOMAIN`, turning skipped ranges into holes); windows are
/// then placed (descending size, best-fit, non-straddling) followed by fillers.
/// When an appended window bumps past a boundary the skipped bytes are
/// registered as a hole so later blocks can reuse them. Returns per-block
/// offsets (in source order) and the total arena size.
fn compute_pack_offsets(blocks: &[PackBlockData]) -> (Vec<u32>, u32) {
    let n = blocks.len();
    let mut offsets = vec![0u32; n];
    let mut holes: Vec<(u32, u32)> = Vec::new();

    let size_desc = |a: &usize, b: &usize| {
        blocks[*b].size.cmp(&blocks[*a].size).then(a.cmp(b))
    };

    // 1) Anchors (align) skeleton, descending size.
    let mut anchors: Vec<usize> = (0..n).filter(|&i| blocks[i].kind == PackKind::Align).collect();
    anchors.sort_by(size_desc);
    let mut cursor = 0u32;
    for &i in &anchors {
        let start = round_up_u32(cursor, PACK_DOMAIN);
        if start > cursor {
            holes.push((cursor, start));
        }
        offsets[i] = start;
        cursor = start + blocks[i].size;
    }
    let mut append_cursor = cursor;

    // 2) Fill order: windows (desc) then fillers (desc).
    let mut windows: Vec<usize> = (0..n).filter(|&i| blocks[i].kind == PackKind::Window).collect();
    windows.sort_by(size_desc);
    let mut fillers: Vec<usize> = (0..n).filter(|&i| blocks[i].kind == PackKind::Filler).collect();
    fillers.sort_by(size_desc);
    let fill: Vec<usize> = windows.into_iter().chain(fillers).collect();

    for &i in &fill {
        let size = blocks[i].size;
        let is_window = blocks[i].kind == PackKind::Window;

        // Best-fit: smallest hole that can accommodate the block.
        let mut best: Option<(usize, u32, u32)> = None; // (hole idx, pos, room)
        for (hi, &(hs, he)) in holes.iter().enumerate() {
            if let Some(pos) = fit_in_hole(hs, he, size, is_window) {
                let room = he - hs;
                if best.map_or(true, |(_, _, br)| room < br) {
                    best = Some((hi, pos, room));
                }
            }
        }

        if let Some((hi, pos, _)) = best {
            let (hs, he) = holes.remove(hi);
            if pos > hs {
                holes.push((hs, pos));
            }
            if pos + size < he {
                holes.push((pos + size, he));
            }
            holes.sort();
            offsets[i] = pos;
        } else {
            let mut pos = append_cursor;
            if is_window && pack_straddles(pos, size) {
                let newpos = round_up_u32(pos, PACK_DOMAIN);
                if newpos > append_cursor {
                    holes.push((append_cursor, newpos));
                    holes.sort();
                }
                pos = newpos;
            }
            offsets[i] = pos;
            append_cursor = pos + size;
        }
    }

    (offsets, append_cursor.max(cursor))
}
