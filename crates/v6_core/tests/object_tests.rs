//! Tests for relocatable ELF object output (`-f obj`).
//!
//! These drive the assembler in object mode and inspect the resulting section
//! / relocation model, plus the serialized ELF32 bytes.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use v6_core::assembler::{Assembler, OutputFormat};
use v6_core::diagnostics::AsmError;
use v6_core::object::section::{RelocKind, RelocTarget};
use v6_core::output::{generate_object, ObjConfig};
use v6_core::preprocessor::preprocess;
use v6_core::project::CpuMode;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A throwaway temp directory that is removed when dropped.
struct TempDir {
    root: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let unique = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("v6asm-obj-tests-{}-{}", nanos, unique));
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Assemble `source` in object mode and return the assembler for inspection.
fn assemble_obj(source: &str) -> Result<Assembler, AsmError> {
    let dir = TempDir::new();
    let main_path = dir.root.join("main.asm");
    fs::write(&main_path, source).unwrap();

    let mut asm = Assembler::new(CpuMode::I8080, dir.root.clone());
    asm.quiet = true;
    asm.output_format = OutputFormat::Obj;

    let lines = preprocess(&main_path, &dir.root, &[], &mut asm.symbols, &|path| {
        fs::read_to_string(path).map_err(|err| AsmError::new(err.to_string()))
    })?;
    asm.assemble(&lines)?;
    Ok(asm)
}

/// Find the relocation in section 0 whose fixup offset equals `offset`.
fn reloc_at(asm: &Assembler, offset: u32) -> &v6_core::object::section::Reloc {
    asm.obj.sections[0]
        .relocs
        .iter()
        .find(|r| r.offset == offset)
        .unwrap_or_else(|| panic!("no relocation at offset {:#x}", offset))
}

// ── relocation kinds ─────────────────────────────────────────────────────────

#[test]
fn jmp_to_label_emits_abs16() {
    // jmp target -> R_V6C_16 against .text + offset(target)
    let asm = assemble_obj(
        "start:\n\
         jmp target\n\
         target:\n\
         nop\n",
    )
    .unwrap();

    let sec = &asm.obj.sections[0];
    assert_eq!(sec.name, ".text");
    // jmp = opcode at 0, operand at offset 1.
    let r = reloc_at(&asm, 1);
    assert_eq!(r.kind, RelocKind::Abs16);
    assert_eq!(r.target, RelocTarget::Section(0));
    assert_eq!(r.addend, 3); // target follows the 3-byte jmp
    // placeholder operand bytes are zeroed.
    assert_eq!(sec.bytes[1], 0);
    assert_eq!(sec.bytes[2], 0);
}

#[test]
fn call_and_lxi_emit_abs16() {
    let asm = assemble_obj(
        "entry:\n\
         call entry\n\
         lxi h, entry\n",
    )
    .unwrap();

    // call operand at offset 1.
    let c = reloc_at(&asm, 1);
    assert_eq!(c.kind, RelocKind::Abs16);
    assert_eq!(c.target, RelocTarget::Section(0));
    assert_eq!(c.addend, 0);

    // lxi opcode at offset 3, operand at offset 4.
    let l = reloc_at(&asm, 4);
    assert_eq!(l.kind, RelocKind::Abs16);
    assert_eq!(l.target, RelocTarget::Section(0));
    assert_eq!(l.addend, 0);
}

#[test]
fn lo_hi_byte_ops_emit_lo8_hi8() {
    let asm = assemble_obj(
        "sym:\n\
         mvi a, <(sym)\n\
         mvi h, >(sym)\n",
    )
    .unwrap();

    // first mvi operand at offset 1.
    let lo = reloc_at(&asm, 1);
    assert_eq!(lo.kind, RelocKind::Lo8);
    assert_eq!(lo.target, RelocTarget::Section(0));
    assert_eq!(lo.addend, 0);

    // second mvi operand at offset 3.
    let hi = reloc_at(&asm, 3);
    assert_eq!(hi.kind, RelocKind::Hi8);
    assert_eq!(hi.target, RelocTarget::Section(0));
    assert_eq!(hi.addend, 0);
}

#[test]
fn dw_and_shld_self_modifying_addend() {
    // dw label+1 and shld label+1 are the classic self-modifying patterns.
    let asm = assemble_obj(
        "label:\n\
         shld label + 1\n\
         dw label + 1\n",
    )
    .unwrap();

    // shld operand at offset 1.
    let s = reloc_at(&asm, 1);
    assert_eq!(s.kind, RelocKind::Abs16);
    assert_eq!(s.target, RelocTarget::Section(0));
    assert_eq!(s.addend, 1);

    // dw at offset 3 (after the 3-byte shld).
    let d = reloc_at(&asm, 3);
    assert_eq!(d.kind, RelocKind::Abs16);
    assert_eq!(d.target, RelocTarget::Section(0));
    assert_eq!(d.addend, 1);
}

#[test]
fn undefined_external_becomes_symbol_reloc() {
    // A symbol that is never defined is an external reference.
    let asm = assemble_obj("call controls_check\n").unwrap();

    let r = reloc_at(&asm, 1);
    assert_eq!(r.kind, RelocKind::Abs16);
    assert_eq!(r.target, RelocTarget::Symbol("controls_check".to_string()));
    assert_eq!(r.addend, 0);
}

#[test]
fn absolute_constant_is_baked_without_reloc() {
    // `= $7331` is a pure constant: emitted directly, no relocation.
    let asm = assemble_obj(
        "value = $7331\n\
         lxi h, value\n",
    )
    .unwrap();

    let sec = &asm.obj.sections[0];
    assert!(sec.relocs.is_empty(), "constant must not produce a reloc");
    // lxi h, $7331 => 0x21 0x31 0x73
    assert_eq!(sec.bytes[0], 0x21);
    assert_eq!(sec.bytes[1], 0x31);
    assert_eq!(sec.bytes[2], 0x73);
}

#[test]
fn immediate_constant_byte_is_baked() {
    let asm = assemble_obj(
        "port = $0c\n\
         mvi a, port\n",
    )
    .unwrap();

    let sec = &asm.obj.sections[0];
    assert!(sec.relocs.is_empty());
    // mvi a, $0c => 0x3E 0x0C
    assert_eq!(sec.bytes[0], 0x3E);
    assert_eq!(sec.bytes[1], 0x0C);
}

// ── sections ─────────────────────────────────────────────────────────────────

#[test]
fn section_directives_create_distinct_sections() {
    let asm = assemble_obj(
        ".section .data\n\
         dw 0\n\
         .bss\n\
         .storage 4\n\
         .text\n\
         nop\n",
    )
    .unwrap();

    let names: Vec<&str> = asm.obj.sections.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&".text"));
    assert!(names.contains(&".data"));
    assert!(names.contains(&".bss"));

    let bss = asm
        .obj
        .sections
        .iter()
        .find(|s| s.name == ".bss")
        .unwrap();
    assert!(bss.is_nobits());
    assert_eq!(bss.size, 4);
    assert!(bss.bytes.is_empty());
}

#[test]
fn cross_section_reference_uses_target_section() {
    // A label defined in .data, referenced from .text.
    let asm = assemble_obj(
        ".section .data\n\
         buffer:\n\
         .storage 2\n\
         .text\n\
         lxi h, buffer\n",
    )
    .unwrap();

    let text_idx = asm
        .obj
        .sections
        .iter()
        .position(|s| s.name == ".text")
        .unwrap();
    let data_idx = asm
        .obj
        .sections
        .iter()
        .position(|s| s.name == ".data")
        .unwrap();

    let text = &asm.obj.sections[text_idx];
    // lxi operand at offset 1 within .text.
    let r = text.relocs.iter().find(|r| r.offset == 1).unwrap();
    assert_eq!(r.kind, RelocKind::Abs16);
    assert_eq!(r.target, RelocTarget::Section(data_idx));
    assert_eq!(r.addend, 0);
}

// ── .optional blocks become sections ─────────────────────────────────────────

#[test]
fn optional_code_block_becomes_text_section() {
    use v6_core::object::section::{SHF_ALLOC, SHF_EXECINSTR, SHT_PROGBITS};

    let asm = assemble_obj(
        "call helper\n\
         .opt\n\
         helper:\n\
         mvi a, 1\n\
         ret\n\
         .endopt\n",
    )
    .unwrap();

    let sec = asm
        .obj
        .sections
        .iter()
        .find(|s| s.name == ".text.helper")
        .expect("expected a .text.helper section");
    assert_eq!(sec.flags, SHF_ALLOC | SHF_EXECINSTR);
    assert_eq!(sec.sh_type, SHT_PROGBITS);
    // mvi a,1 (2 bytes) + ret (1 byte)
    assert_eq!(sec.size, 3);
}

#[test]
fn optional_data_block_becomes_data_section() {
    use v6_core::object::section::{SHF_ALLOC, SHF_WRITE, SHT_PROGBITS};

    let asm = assemble_obj(
        "lxi h, table\n\
         .opt\n\
         table:\n\
         .byte 1, 2, 3\n\
         .endopt\n",
    )
    .unwrap();

    let sec = asm
        .obj
        .sections
        .iter()
        .find(|s| s.name == ".data.table")
        .expect("expected a .data.table section");
    assert_eq!(sec.flags, SHF_ALLOC | SHF_WRITE);
    assert_eq!(sec.sh_type, SHT_PROGBITS);
    assert_eq!(sec.size, 3);
    assert_eq!(sec.bytes, vec![1, 2, 3]);
}

#[test]
fn optional_section_uses_first_referenced_label() {
    // The block defines `internal` first, but only `entry` is referenced from
    // outside, so the section is named after `entry`.
    let asm = assemble_obj(
        "call entry\n\
         .opt\n\
         internal:\n\
         nop\n\
         entry:\n\
         ret\n\
         .endopt\n",
    )
    .unwrap();

    let names: Vec<&str> = asm.obj.sections.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&".text.entry"), "sections: {names:?}");
    assert!(!names.contains(&".text.internal"), "sections: {names:?}");
}

#[test]
fn optional_storage_only_block_becomes_bss_section() {
    use v6_core::object::section::{SHF_ALLOC, SHF_WRITE, SHT_NOBITS};

    // A block that only reserves space with `.storage` (no filler) carries no
    // file bytes and belongs in a `.bss.<label>` (SHT_NOBITS) section.
    let asm = assemble_obj(
        "lxi h, buffer\n\
         .opt\n\
         buffer:\n\
         .storage 8\n\
         .endopt\n",
    )
    .unwrap();

    let sec = asm
        .obj
        .sections
        .iter()
        .find(|s| s.name == ".bss.buffer")
        .expect("expected a .bss.buffer section");
    assert_eq!(sec.flags, SHF_ALLOC | SHF_WRITE);
    assert_eq!(sec.sh_type, SHT_NOBITS);
    assert_eq!(sec.size, 8);
    assert!(sec.bytes.is_empty(), "bss section must store no bytes");
}

#[test]
fn optional_storage_with_filler_stays_in_data_section() {
    use v6_core::object::section::{SHF_ALLOC, SHF_WRITE, SHT_PROGBITS};

    // `.storage N, filler` emits initialized bytes, so it stays in `.data.*`.
    let asm = assemble_obj(
        "lxi h, table\n\
         .opt\n\
         table:\n\
         .storage 3, 0x7E\n\
         .endopt\n",
    )
    .unwrap();

    let sec = asm
        .obj
        .sections
        .iter()
        .find(|s| s.name == ".data.table")
        .expect("expected a .data.table section");
    assert_eq!(sec.flags, SHF_ALLOC | SHF_WRITE);
    assert_eq!(sec.sh_type, SHT_PROGBITS);
    assert_eq!(sec.bytes, vec![0x7E, 0x7E, 0x7E]);
}

#[test]
fn align_before_optional_propagates_to_section() {
    // A `.align` placed just before `.opt` must transfer its alignment to the
    // section the optional block creates, so the linker can honor it.
    let asm = assemble_obj(
        "lxi h, buffer\n\
         .align 256\n\
         .opt\n\
         buffer:\n\
         .storage 8, 0\n\
         .endopt\n",
    )
    .unwrap();

    let sec = asm
        .obj
        .sections
        .iter()
        .find(|s| s.name == ".data.buffer")
        .expect("expected a .data.buffer section");
    assert_eq!(sec.addralign, 256);
}

#[test]
fn align_before_section_propagates_to_section() {
    // The same rule applies to an explicit `.section` switch.
    let asm = assemble_obj(
        ".align 64\n\
         .section .mydata\n\
         .byte 1, 2, 3\n",
    )
    .unwrap();

    let sec = asm
        .obj
        .sections
        .iter()
        .find(|s| s.name == ".mydata")
        .expect("expected a .mydata section");
    assert_eq!(sec.addralign, 64);
}

#[test]
fn align_inside_section_raises_its_alignment() {
    // `.align` within a section raises that section's own alignment.
    let asm = assemble_obj(
        ".section .data\n\
         .byte 1\n\
         .align 16\n\
         .byte 2\n",
    )
    .unwrap();

    let sec = asm
        .obj
        .sections
        .iter()
        .find(|s| s.name == ".data")
        .expect("expected a .data section");
    assert_eq!(sec.addralign, 16);
}

#[test]
fn no_align_leaves_default_alignment() {
    let asm = assemble_obj(
        "lxi h, buffer\n\
         .opt\n\
         buffer:\n\
         .storage 8, 0\n\
         .endopt\n",
    )
    .unwrap();

    let sec = asm
        .obj
        .sections
        .iter()
        .find(|s| s.name == ".data.buffer")
        .expect("expected a .data.buffer section");
    assert_eq!(sec.addralign, 1);
}

#[test]
fn nested_optional_blocks_create_separate_sections() {
    let asm = assemble_obj(
        "call outer\n\
         .opt\n\
         outer:\n\
         call inner\n\
         ret\n\
         .opt\n\
         inner:\n\
         ret\n\
         .endopt\n\
         .endopt\n",
    )
    .unwrap();

    let names: Vec<&str> = asm.obj.sections.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&".text.outer"), "sections: {names:?}");
    assert!(names.contains(&".text.inner"), "sections: {names:?}");
}

#[test]
fn setting_optional_prune_disables_sections_in_obj_mode() {
    // With prune mode, an unreferenced block is dropped and no dedicated
    // section is created.
    let asm = assemble_obj(
        ".setting optional, prune\n\
         .opt\n\
         unused:\n\
         .byte 9\n\
         .endopt\n\
         nop\n",
    )
    .unwrap();

    let names: Vec<&str> = asm.obj.sections.iter().map(|s| s.name.as_str()).collect();
    assert!(
        !names.iter().any(|n| n.starts_with(".data.") || n.starts_with(".text.unused")),
        "expected no dedicated optional section, got: {names:?}"
    );
}

#[test]
fn optional_block_without_symbols_is_an_error() {
    let err = match assemble_obj(
        ".opt\n\
         nop\n\
         .endopt\n",
    ) {
        Ok(_) => panic!("expected an error for a label-less .optional block"),
        Err(e) => e,
    };
    let msg = format!("{}", err).to_lowercase();
    assert!(
        msg.contains("optional"),
        "expected an .optional error, got: {msg}"
    );
}

// ── org rejected in obj mode ─────────────────────────────────────────────────

#[test]
fn org_is_rejected_in_object_mode() {
    let err = match assemble_obj(".org $8000\nnop\n") {
        Ok(_) => panic!("expected .org to be rejected in object mode"),
        Err(e) => e,
    };
    let msg = format!("{}", err);
    assert!(
        msg.to_lowercase().contains("org"),
        "expected an .org error, got: {msg}"
    );
}

// ── .pack blocks ─────────────────────────────────────────────────────────────

#[test]
fn pack_blocks_go_into_bss_pack_nobits_section() {
    use v6_core::object::section::{SHF_ALLOC, SHF_WRITE, SHT_NOBITS};
    // Every logical block is a distinct alignment-1 NOBITS section. The two
    // anchors intentionally have the same visible section name.
    let asm = assemble_obj(
        ".global *\n\
         lxi h, pack_a\n\
         lxi d, pack_b\n\
         lxi b, pack_c\n\
         .pack align\n\
         pack_a:\n\
         .storage 256\n\
         .endpack\n\
         .pack align\n\
         pack_b:\n\
         .storage 256\n\
         .endpack\n\
         .pack\n\
         pack_c:\n\
         .storage 16\n\
         .endpack\n",
    )
    .unwrap();

    let packed: Vec<(usize, _)> = asm
        .obj
        .sections
        .iter()
        .enumerate()
        .filter(|(_, section)| section.name.starts_with(".bss.pack"))
        .collect();
    assert_eq!(packed.len(), 3);
    assert_eq!(packed[0].1.name, ".bss.pack.align");
    assert_eq!(packed[1].1.name, ".bss.pack.align");
    assert_eq!(packed[2].1.name, ".bss.pack");
    assert_eq!(
        packed
            .iter()
            .map(|(_, section)| section.size)
            .collect::<Vec<_>>(),
        vec![256, 256, 16]
    );
    for (_, section) in &packed {
        assert_eq!(section.sh_type, SHT_NOBITS);
        assert_eq!(section.flags, SHF_ALLOC | SHF_WRITE);
        assert!(section.is_nobits());
        assert!(section.bytes.is_empty());
        assert_eq!(section.addralign, 1);
    }

    // Labels are block-relative and owned by distinct section headers.
    let a = asm.symbols.get_global_info("pack_a").unwrap();
    let b = asm.symbols.get_global_info("pack_b").unwrap();
    let c = asm.symbols.get_global_info("pack_c").unwrap();
    assert_eq!(a.section, Some(packed[0].0));
    assert_eq!(b.section, Some(packed[1].0));
    assert_eq!(c.section, Some(packed[2].0));
    assert_eq!(a.value, Some(0));
    assert_eq!(b.value, Some(0));
    assert_eq!(c.value, Some(0));

    // Serialization preserves duplicate visible names as distinct headers,
    // gives each label its block's section index, and keeps one relocation
    // reachability edge to each block.
    let bytes = generate_object(&asm, &ObjConfig::default()).unwrap();
    let e_shoff = read_u32(&bytes, 32) as usize;
    let e_shentsize = read_u16(&bytes, 46) as usize;
    let e_shnum = read_u16(&bytes, 48) as usize;
    let e_shstrndx = read_u16(&bytes, 50) as usize;
    let sh = |index: usize| e_shoff + index * e_shentsize;
    let shstr_off = read_u32(&bytes, sh(e_shstrndx) + 16) as usize;
    let str_at = |base: usize, index: u32| -> String {
        let start = base + index as usize;
        let end = bytes[start..]
            .iter()
            .position(|&c| c == 0)
            .map(|pos| start + pos)
            .unwrap();
        String::from_utf8_lossy(&bytes[start..end]).into_owned()
    };
    let section_names: Vec<_> = (0..e_shnum)
        .map(|index| str_at(shstr_off, read_u32(&bytes, sh(index))))
        .collect();
    let align_headers: Vec<_> = section_names
        .iter()
        .enumerate()
        .filter(|(_, name)| *name == ".bss.pack.align")
        .map(|(index, _)| index)
        .collect();
    let filler_header = section_names
        .iter()
        .position(|name| name == ".bss.pack")
        .unwrap();
    assert_eq!(align_headers.len(), 2);
    assert_ne!(align_headers[0], align_headers[1]);

    let symtab_i = section_names
        .iter()
        .position(|name| name == ".symtab")
        .unwrap();
    let strtab_i = section_names
        .iter()
        .position(|name| name == ".strtab")
        .unwrap();
    let symtab_off = read_u32(&bytes, sh(symtab_i) + 16) as usize;
    let symtab_size = read_u32(&bytes, sh(symtab_i) + 20) as usize;
    let strtab_off = read_u32(&bytes, sh(strtab_i) + 16) as usize;
    let mut symbol_sections = std::collections::HashMap::new();
    for index in 0..symtab_size / 16 {
        let base = symtab_off + index * 16;
        symbol_sections.insert(
            str_at(strtab_off, read_u32(&bytes, base)),
            read_u16(&bytes, base + 14),
        );
    }
    assert_eq!(symbol_sections["pack_a"] as usize, align_headers[0]);
    assert_eq!(symbol_sections["pack_b"] as usize, align_headers[1]);
    assert_eq!(symbol_sections["pack_c"] as usize, filler_header);

    let rela_text_i = section_names
        .iter()
        .position(|name| name == ".rela.text")
        .unwrap();
    let rela_off = read_u32(&bytes, sh(rela_text_i) + 16) as usize;
    let rela_size = read_u32(&bytes, sh(rela_text_i) + 20) as usize;
    let mut targets = Vec::new();
    for index in 0..rela_size / 12 {
        let r_info = read_u32(&bytes, rela_off + index * 12 + 4);
        let symbol_index = (r_info >> 8) as usize;
        targets.push(
            read_u16(&bytes, symtab_off + symbol_index * 16 + 14) as usize,
        );
    }
    assert_eq!(
        targets,
        vec![align_headers[0], align_headers[1], filler_header]
    );
}

#[test]
fn pack_dataset_obj_matches_producer_contract() {
    // The full runtime dataset emits one linker-packable section per block.
    let asm = assemble_obj(include_str!("fixtures/v6_runtime_data.asm")).unwrap();

    // Only the linker knows post-GC arena metrics in object mode.
    assert!(asm.symbols.get_global_info("__PACK_ARENA_SIZE").is_none());
    assert!(asm.symbols.get_global_info("__PACK_WASTE_BYTES").is_none());

    let packed: Vec<_> = asm
        .obj
        .sections
        .iter()
        .filter(|section| section.name.starts_with(".bss.pack"))
        .collect();
    assert_eq!(packed.len(), 23);
    assert_eq!(
        packed.iter().map(|section| section.size).sum::<u32>(),
        0xCA0
    );
    assert_eq!(
        packed
            .iter()
            .filter(|section| section.name == ".bss.pack.align")
            .count(),
        4
    );
    assert_eq!(
        packed
            .iter()
            .filter(|section| section.name == ".bss.pack.window")
            .count(),
        4
    );
    assert_eq!(
        packed
            .iter()
            .filter(|section| section.name == ".bss.pack")
            .count(),
        15
    );
    assert!(packed.iter().all(|section| section.addralign == 1));

    for name in [
        "containers_inst_data_ptrs",
        "resources_inst_data_ptrs",
        "breakables_status",
        "room_tiledata",
        "room_teleports_data",
        "overlays_runtime_data",
        "hero_resources",
        "rooms_spawn_rate",
        "ram_disk_mode",
    ] {
        assert_eq!(asm.symbols.get_global_info(name).unwrap().value, Some(0));
    }
}

#[test]
fn pack_storage_uses_preceding_constant_inside_conditional() -> Result<(), AsmError> {
    let dir = TempDir::new();
    let main_path = dir.root.join("main.asm");
    fs::write(
        &main_path,
        ".setting force_once, true\n\
         WORD_LEN = 2\n\
                 .pack\n\
                 early_data:\n\
                     .storage 1\n\
                 .endpack\n\
         ENABLE = 1\n\
         .if ENABLE\n\
         .include \"sound.asm\"\n\
         .endif\n",
    )
    .unwrap();
    fs::write(
        dir.root.join("sound.asm"),
        ".include \"constants.asm\"\n\
         .include \"runtime-data.asm\"\n",
    )
    .unwrap();
    fs::write(
        dir.root.join("constants.asm"),
        ".global RAM_DISK_MUSIC\n\
         GC_TASKS = 14\n",
    )
    .unwrap();
    fs::write(
        dir.root.join("runtime-data.asm"),
        ".pack\n\
         V6_GC_TASK_SPS_LEN = GC_TASKS * WORD_LEN\n\
         v6_gc_task_sps:\n\
           .storage V6_GC_TASK_SPS_LEN\n\
         .endpack\n",
    )
    .unwrap();

    let mut asm = Assembler::new(CpuMode::I8080, dir.root.clone());
    asm.quiet = true;
    asm.output_format = OutputFormat::Obj;
    let lines = preprocess(&main_path, &dir.root, &[], &mut asm.symbols, &|path| {
        fs::read_to_string(path).map_err(|err| AsmError::new(err.to_string()))
    })?;
    asm.assemble(&lines)?;

    assert_eq!(asm.symbols.resolve("V6_GC_TASK_SPS_LEN"), Some(28));
    let packed = asm
        .obj
        .sections
        .iter()
        .filter(|section| section.name == ".bss.pack")
        .collect::<Vec<_>>();
    assert_eq!(packed.iter().map(|section| section.size).sum::<u32>(), 29);
    Ok(())
}

// ── ELF serialization ────────────────────────────────────────────────────────

fn read_u16(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off + 1]])
}

fn read_u32(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}

#[test]
fn elf_header_is_well_formed() {
    let asm = assemble_obj(
        ".globl entry\n\
         entry:\n\
         jmp entry\n",
    )
    .unwrap();
    let bytes = generate_object(&asm, &ObjConfig::default()).unwrap();

    // e_ident magic.
    assert_eq!(&bytes[0..4], &[0x7f, b'E', b'L', b'F']);
    assert_eq!(bytes[4], 1, "ELFCLASS32");
    assert_eq!(bytes[5], 1, "ELFDATA2LSB");
    assert_eq!(bytes[6], 1, "EI_VERSION");
    assert_eq!(bytes[7], 0, "ELFOSABI_NONE");

    assert_eq!(read_u16(&bytes, 16), 1, "ET_REL");
    assert_eq!(read_u16(&bytes, 18), 0x8080, "EM_V6C");
    assert_eq!(read_u32(&bytes, 20), 1, "EV_CURRENT");
    assert_eq!(read_u32(&bytes, 36), 0, "e_flags");
    assert_eq!(read_u16(&bytes, 40), 52, "e_ehsize");
    assert_eq!(read_u16(&bytes, 46), 40, "e_shentsize");
}

#[test]
fn elf_contains_expected_sections_and_symbols() {
    let asm = assemble_obj(
        ".globl entry\n\
         entry:\n\
         call external\n\
         jmp entry\n",
    )
    .unwrap();
    let bytes = generate_object(&asm, &ObjConfig::default()).unwrap();

    // Parse section header table to collect section names.
    let e_shoff = read_u32(&bytes, 32) as usize;
    let e_shentsize = read_u16(&bytes, 46) as usize;
    let e_shnum = read_u16(&bytes, 48) as usize;
    let e_shstrndx = read_u16(&bytes, 50) as usize;

    // shstrtab location.
    let sh = |i: usize| e_shoff + i * e_shentsize;
    let shstr_off = read_u32(&bytes, sh(e_shstrndx) + 16) as usize;
    let str_at = |base: usize, idx: u32| -> String {
        let start = base + idx as usize;
        let end = bytes[start..]
            .iter()
            .position(|&c| c == 0)
            .map(|p| start + p)
            .unwrap();
        String::from_utf8_lossy(&bytes[start..end]).into_owned()
    };

    let mut section_names = Vec::new();
    for i in 0..e_shnum {
        let name_idx = read_u32(&bytes, sh(i));
        section_names.push(str_at(shstr_off, name_idx));
    }
    assert!(section_names.iter().any(|n| n == ".text"));
    assert!(section_names.iter().any(|n| n == ".rela.text"));
    assert!(section_names.iter().any(|n| n == ".symtab"));
    assert!(section_names.iter().any(|n| n == ".strtab"));
    assert!(section_names.iter().any(|n| n == ".shstrtab"));

    // Locate .symtab and .strtab.
    let symtab_i = section_names.iter().position(|n| n == ".symtab").unwrap();
    let strtab_i = section_names.iter().position(|n| n == ".strtab").unwrap();
    let symtab_off = read_u32(&bytes, sh(symtab_i) + 16) as usize;
    let symtab_size = read_u32(&bytes, sh(symtab_i) + 20) as usize;
    let strtab_off = read_u32(&bytes, sh(strtab_i) + 16) as usize;

    // Each Elf32_Sym is 16 bytes: name(4) value(4) size(4) info(1) other(1) shndx(2)
    let mut names = Vec::new();
    let mut external_undef = false;
    let mut entry_global_defined = false;
    let count = symtab_size / 16;
    for i in 0..count {
        let base = symtab_off + i * 16;
        let name_idx = read_u32(&bytes, base);
        let info = bytes[base + 12];
        let shndx = read_u16(&bytes, base + 14);
        let bind = info >> 4;
        let name = str_at(strtab_off, name_idx);
        if name == "external" {
            external_undef = bind == 1 /* GLOBAL */ && shndx == 0 /* SHN_UNDEF */;
        }
        if name == "entry" {
            entry_global_defined = bind == 1 && shndx != 0;
        }
        names.push(name);
    }

    assert!(names.iter().any(|n| n == "entry"));
    assert!(names.iter().any(|n| n == "external"));
    assert!(external_undef, "external must be GLOBAL + SHN_UNDEF");
    assert!(entry_global_defined, "entry must be GLOBAL + defined");
}

#[test]
fn elf_rela_entries_match_section_relocs() {
    let asm = assemble_obj(
        "entry:\n\
         call external\n\
         jmp entry\n",
    )
    .unwrap();
    let bytes = generate_object(&asm, &ObjConfig::default()).unwrap();

    let e_shoff = read_u32(&bytes, 32) as usize;
    let e_shentsize = read_u16(&bytes, 46) as usize;
    let e_shnum = read_u16(&bytes, 48) as usize;
    let e_shstrndx = read_u16(&bytes, 50) as usize;
    let sh = |i: usize| e_shoff + i * e_shentsize;
    let shstr_off = read_u32(&bytes, sh(e_shstrndx) + 16) as usize;
    let str_at = |base: usize, idx: u32| -> String {
        let start = base + idx as usize;
        let end = bytes[start..]
            .iter()
            .position(|&c| c == 0)
            .map(|p| start + p)
            .unwrap();
        String::from_utf8_lossy(&bytes[start..end]).into_owned()
    };

    let mut rela_i = None;
    for i in 0..e_shnum {
        let name_idx = read_u32(&bytes, sh(i));
        if str_at(shstr_off, name_idx) == ".rela.text" {
            rela_i = Some(i);
        }
    }
    let rela_i = rela_i.expect(".rela.text must exist");
    let rela_off = read_u32(&bytes, sh(rela_i) + 16) as usize;
    let rela_size = read_u32(&bytes, sh(rela_i) + 20) as usize;
    let rela_entsize = read_u32(&bytes, sh(rela_i) + 36) as usize;
    assert_eq!(rela_entsize, 12, "Elf32_Rela is 12 bytes");

    // Two relocations: call external (offset 1), jmp entry (offset 4).
    let count = rela_size / 12;
    assert_eq!(count, 2);

    let mut offsets = Vec::new();
    for i in 0..count {
        let base = rela_off + i * 12;
        let r_offset = read_u32(&bytes, base);
        let r_info = read_u32(&bytes, base + 4);
        let r_type = r_info & 0xff;
        // R_V6C_16 == 2.
        assert_eq!(r_type, 2, "both relocs are 16-bit absolute");
        offsets.push(r_offset);
    }
    offsets.sort();
    assert_eq!(offsets, vec![1, 4]);
}
