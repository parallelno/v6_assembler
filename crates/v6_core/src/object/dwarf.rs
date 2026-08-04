//! Minimal DWARF v4 metadata for relocatable V6C objects and direct-ROM companions.

use std::collections::BTreeMap;
use std::path::Path;

use crate::assembler::DebugLineRow;

use super::elf::ExtraSection;
use super::section::{Reloc, RelocKind, RelocTarget, SHT_PROGBITS};

/// Source provenance for an immutable absolute assembler constant.
#[derive(Debug, Clone)]
pub struct DebugConstant {
    pub name: String,
    pub file: String,
    pub line: usize,
    pub value: i64,
}

const DW_LNS_COPY: u8 = 1;
const DW_LNS_ADVANCE_LINE: u8 = 3;
const DW_LNS_SET_FILE: u8 = 4;
const DW_LNS_SET_COLUMN: u8 = 5;
const DW_LNE_END_SEQUENCE: u8 = 1;
const DW_LNE_SET_ADDRESS: u8 = 2;

/// Build the small DWARF v4 subset emitted under `v6asm -g -f obj`.
pub fn debug_sections(
    rows: &[DebugLineRow],
    constants: &[DebugConstant],
    compilation_dir: &Path,
) -> Vec<ExtraSection> {
    let mut files: Vec<String> = rows.iter().map(|row| row.file.clone()).collect();
    files.extend(constants.iter().map(|constant| constant.file.clone()));
    files.sort();
    files.dedup();
    let mut directories: Vec<String> = files
        .iter()
        .filter_map(|file| file_directory(file))
        .collect();
    directories.sort();
    directories.dedup();
    let directory_indices: BTreeMap<String, u32> = directories
        .iter()
        .enumerate()
        .map(|(index, directory)| (directory.clone(), index as u32 + 1))
        .collect();
    let file_indices: BTreeMap<String, u32> = files
        .iter()
        .enumerate()
        .map(|(index, file)| (file.clone(), index as u32 + 1))
        .collect();

    let unit_name = files.first().cloned().unwrap_or_default();
    let comp_dir = compilation_dir.to_string_lossy().replace('\\', "/");
    let (debug_str, producer_offset, name_offset, comp_dir_offset, constant_name_offsets) =
        debug_strings(&unit_name, &comp_dir, constants);

    vec![
        ExtraSection {
            name: ".debug_info".to_string(),
            sh_type: SHT_PROGBITS,
            flags: 0,
            addralign: 1,
            link: 0,
            info: 0,
            entsize: 0,
            data: debug_info(
                producer_offset,
                name_offset,
                comp_dir_offset,
                constants,
                &file_indices,
                &constant_name_offsets,
            ),
            relocs: Vec::new(),
        },
        ExtraSection {
            name: ".debug_abbrev".to_string(),
            sh_type: SHT_PROGBITS,
            flags: 0,
            addralign: 1,
            link: 0,
            info: 0,
            entsize: 0,
            data: debug_abbrev(),
            relocs: Vec::new(),
        },
        debug_line(rows, &directories, &directory_indices, &files, &file_indices),
        ExtraSection {
            name: ".debug_str".to_string(),
            sh_type: SHT_PROGBITS,
            flags: 0,
            addralign: 1,
            link: 0,
            info: 0,
            entsize: 0,
            data: debug_str,
            relocs: Vec::new(),
        },
    ]
}

fn file_directory(file: &str) -> Option<String> {
    file.rsplit_once('/').and_then(|(directory, _)| {
        (!directory.is_empty()).then(|| directory.to_string())
    })
}

fn file_name(file: &str) -> &str {
    file.rsplit_once('/').map_or(file, |(_, name)| name)
}

fn debug_strings(
    unit_name: &str,
    comp_dir: &str,
    constants: &[DebugConstant],
) -> (Vec<u8>, u32, u32, u32, Vec<u32>) {
    let mut data = Vec::new();
    let producer_offset = push_string(&mut data, "v6asm");
    let name_offset = push_string(&mut data, unit_name);
    let comp_dir_offset = push_string(&mut data, comp_dir);
    let constant_name_offsets = constants
        .iter()
        .map(|constant| push_string(&mut data, &constant.name))
        .collect();
    (data, producer_offset, name_offset, comp_dir_offset, constant_name_offsets)
}

fn push_string(data: &mut Vec<u8>, value: &str) -> u32 {
    let offset = data.len() as u32;
    data.extend_from_slice(value.as_bytes());
    data.push(0);
    offset
}

fn debug_abbrev() -> Vec<u8> {
    let mut data = Vec::new();
    write_uleb(&mut data, 1); // abbreviation code
    write_uleb(&mut data, 0x11); // DW_TAG_compile_unit
    data.push(1); // DW_CHILDREN_yes
    write_uleb(&mut data, 0x25); // DW_AT_producer
    write_uleb(&mut data, 0x0e); // DW_FORM_strp
    write_uleb(&mut data, 0x13); // DW_AT_language
    write_uleb(&mut data, 0x05); // DW_FORM_data2
    write_uleb(&mut data, 0x03); // DW_AT_name
    write_uleb(&mut data, 0x0e); // DW_FORM_strp
    write_uleb(&mut data, 0x1b); // DW_AT_comp_dir
    write_uleb(&mut data, 0x0e); // DW_FORM_strp
    write_uleb(&mut data, 0x10); // DW_AT_stmt_list
    write_uleb(&mut data, 0x17); // DW_FORM_sec_offset
    data.extend_from_slice(&[0, 0]); // attribute/form terminator
    write_uleb(&mut data, 2); // abbreviation code
    write_uleb(&mut data, 0x34); // DW_TAG_variable
    data.push(0); // DW_CHILDREN_no
    write_uleb(&mut data, 0x03); // DW_AT_name
    write_uleb(&mut data, 0x0e); // DW_FORM_strp
    write_uleb(&mut data, 0x3a); // DW_AT_decl_file
    write_uleb(&mut data, 0x0f); // DW_FORM_udata
    write_uleb(&mut data, 0x3b); // DW_AT_decl_line
    write_uleb(&mut data, 0x0f); // DW_FORM_udata
    write_uleb(&mut data, 0x1c); // DW_AT_const_value
    write_uleb(&mut data, 0x0d); // DW_FORM_sdata
    data.extend_from_slice(&[0, 0]); // attribute/form terminator
    data.push(0); // abbreviation table terminator
    data
}

fn debug_info(
    producer_offset: u32,
    name_offset: u32,
    comp_dir_offset: u32,
    constants: &[DebugConstant],
    file_indices: &BTreeMap<String, u32>,
    constant_name_offsets: &[u32],
) -> Vec<u8> {
    let mut data = vec![0; 4];
    data.extend_from_slice(&4u16.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    data.push(2); // V6C address size
    write_uleb(&mut data, 1);
    data.extend_from_slice(&producer_offset.to_le_bytes());
    data.extend_from_slice(&0x8001u16.to_le_bytes()); // DW_LANG_Mips_Assembler
    data.extend_from_slice(&name_offset.to_le_bytes());
    data.extend_from_slice(&comp_dir_offset.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes()); // .debug_line offset
    for (constant, name_offset) in constants.iter().zip(constant_name_offsets) {
        write_uleb(&mut data, 2);
        data.extend_from_slice(&name_offset.to_le_bytes());
        write_uleb(&mut data, file_indices[&constant.file] as u64);
        write_uleb(&mut data, constant.line as u64);
        write_sleb(&mut data, constant.value);
    }
    data.push(0); // End of compilation-unit children.
    write_initial_length(&mut data);
    data
}

fn debug_line(
    rows: &[DebugLineRow],
    directories: &[String],
    directory_indices: &BTreeMap<String, u32>,
    files: &[String],
    file_indices: &BTreeMap<String, u32>,
) -> ExtraSection {
    let mut data = vec![0; 4];
    data.extend_from_slice(&4u16.to_le_bytes());
    let header_length_offset = data.len();
    data.extend_from_slice(&0u32.to_le_bytes());
    let header_start = data.len();
    data.extend_from_slice(&[1, 1, 1, -5i8 as u8, 14, 13]);
    data.extend_from_slice(&[0, 1, 1, 1, 1, 0, 0, 0, 1, 0, 0, 1]);
    for directory in directories {
        data.extend_from_slice(directory.as_bytes());
        data.push(0);
    }
    data.push(0); // include directories terminator
    for file in files {
        data.extend_from_slice(file_name(file).as_bytes());
        data.push(0);
        let directory = file_directory(file);
        let directory_index = directory
            .as_ref()
            .and_then(|directory| directory_indices.get(directory))
            .copied()
            .unwrap_or(0);
        write_uleb(&mut data, directory_index as u64);
        write_uleb(&mut data, 0); // modification time
        write_uleb(&mut data, 0); // size
    }
    data.push(0); // file table terminator
    let header_length = (data.len() - header_start) as u32;
    data[header_length_offset..header_length_offset + 4].copy_from_slice(&header_length.to_le_bytes());

    let mut relocs = Vec::new();
    let mut active_section = None;
    let mut has_sequence = false;
    let mut current_line = 1i64;
    let mut current_file = 1u32;
    let mut current_column = 0u32;
    let mut statement_rows: Vec<&DebugLineRow> = rows.iter().filter(|row| row.is_stmt).collect();
    statement_rows.sort_by_key(|row| {
        (
            row.section,
            row.offset_or_address,
            &row.file,
            row.line_num,
            row.column,
        )
    });
    for row in statement_rows {
        if !has_sequence || active_section != row.section {
            if has_sequence {
                data.extend_from_slice(&[0, 1, DW_LNE_END_SEQUENCE]);
            }
            active_section = row.section;
            has_sequence = true;
            current_line = 1;
            current_file = 1;
            current_column = 0;
        }

        data.extend_from_slice(&[0, 3, DW_LNE_SET_ADDRESS]);
        let reloc_offset = data.len() as u32;
        data.extend_from_slice(&(row.section.map_or(row.offset_or_address as u16, |_| 0)).to_le_bytes());
        if let Some(section) = row.section {
            relocs.push(Reloc {
                offset: reloc_offset,
                kind: RelocKind::Abs16,
                target: RelocTarget::Section(section),
                addend: row.offset_or_address as i64,
            });
        }

        let file = file_indices[&row.file];
        if current_file != file {
            data.push(DW_LNS_SET_FILE);
            write_uleb(&mut data, file as u64);
            current_file = file;
        }
        let line_delta = row.line_num as i64 - current_line;
        if line_delta != 0 {
            data.push(DW_LNS_ADVANCE_LINE);
            write_sleb(&mut data, line_delta);
            current_line = row.line_num as i64;
        }
        if current_column != row.column {
            data.push(DW_LNS_SET_COLUMN);
            write_uleb(&mut data, row.column as u64);
            current_column = row.column;
        }
        data.push(DW_LNS_COPY);
    }
    if has_sequence {
        data.extend_from_slice(&[0, 1, DW_LNE_END_SEQUENCE]);
    }
    write_initial_length(&mut data);

    ExtraSection {
        name: ".debug_line".to_string(),
        sh_type: SHT_PROGBITS,
        flags: 0,
        addralign: 1,
        link: 0,
        info: 0,
        entsize: 0,
        data,
        relocs,
    }
}

fn write_initial_length(data: &mut [u8]) {
    let length = (data.len() - 4) as u32;
    data[..4].copy_from_slice(&length.to_le_bytes());
}

fn write_uleb(data: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        data.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn write_sleb(data: &mut Vec<u8>, mut value: i64) {
    loop {
        let byte = (value as u8) & 0x7f;
        value >>= 7;
        let done = (value == 0 && byte & 0x40 == 0) || (value == -1 && byte & 0x40 != 0);
        data.push(if done { byte } else { byte | 0x80 });
        if done {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::assembler::{DebugLineRow, EmissionKind};

    use super::{debug_sections, write_sleb, write_uleb, DebugConstant, ExtraSection};

    fn row(section: Option<usize>, address: u32, file: &str, line: usize, column: u32) -> DebugLineRow {
        DebugLineRow {
            section,
            offset_or_address: address,
            byte_len: 1,
            file: file.to_string(),
            line_num: line,
            column,
            is_stmt: true,
            kind: EmissionKind::Instruction,
            macro_context: None,
            expansion: Vec::new(),
        }
    }

    fn section<'a>(sections: &'a [ExtraSection], name: &str) -> &'a ExtraSection {
        sections.iter().find(|section| section.name == name).unwrap()
    }

    fn initial_length(data: &[u8]) -> usize {
        u32::from_le_bytes(data[..4].try_into().unwrap()) as usize
    }

    #[test]
    fn leb128_encoders_match_known_values() {
        let mut unsigned = Vec::new();
        write_uleb(&mut unsigned, 624_485);
        assert_eq!(unsigned, [0xe5, 0x8e, 0x26]);

        let mut signed = Vec::new();
        write_sleb(&mut signed, -624_485);
        assert_eq!(signed, [0x9b, 0xf1, 0x59]);
    }

    #[test]
    fn debug_sections_encode_deterministic_dwarf_v4_metadata() {
        let rows = vec![
            row(Some(1), 2, "src/main.asm", 9, 6),
            row(Some(0), 1, "lib/util.asm", 4, 0),
        ];
        let sections = debug_sections(&rows, &[], Path::new("C:\\project"));
        let reversed_rows: Vec<DebugLineRow> = rows.into_iter().rev().collect();
        let reversed = debug_sections(&reversed_rows, &[], Path::new("C:\\project"));

        assert_eq!(sections.len(), 4);
        assert_eq!(
            sections.iter().map(|section| section.name.as_str()).collect::<Vec<_>>(),
            [".debug_info", ".debug_abbrev", ".debug_line", ".debug_str"],
        );
        for name in [".debug_info", ".debug_line"] {
            let section = section(&sections, name);
            assert_eq!(initial_length(&section.data), section.data.len() - 4);
        }
        assert_eq!(&section(&sections, ".debug_info").data[4..11], &[4, 0, 0, 0, 0, 0, 2]);
        assert_eq!(section(&sections, ".debug_abbrev").data.last(), Some(&0));
        assert_eq!(section(&sections, ".debug_str").data, b"v6asm\0lib/util.asm\0C:/project\0");

        let line = section(&sections, ".debug_line");
        assert!(line.data.windows(b"lib\0src\0".len()).any(|window| window == b"lib\0src\0"));
        assert!(line.data.windows(b"util.asm\0\x01\0\0main.asm\0\x02\0\0".len()).any(|window| {
            window == b"util.asm\0\x01\0\0main.asm\0\x02\0\0"
        }));
        assert_eq!(line.relocs.len(), 2);
        let header_length = u32::from_le_bytes(line.data[6..10].try_into().unwrap()) as usize;
        let program = &line.data[10 + header_length..];
        assert_eq!(program.windows(3).filter(|window| *window == [0, 1, 1]).count(), 2);

        for (section, reversed_section) in sections.iter().zip(reversed.iter()) {
            assert_eq!(section.data, reversed_section.data);
            assert_eq!(section.relocs.len(), reversed_section.relocs.len());
        }
    }

    #[test]
    fn direct_rom_line_rows_use_absolute_addresses_without_relocations() {
        let sections = debug_sections(
            &[row(None, 0x1234, "game.asm", 3, 7)],
            &[],
            Path::new("."),
        );
        let line = section(&sections, ".debug_line");

        assert!(line.relocs.is_empty());
        assert!(line.data.windows(5).any(|window| window == [0, 3, 2, 0x34, 0x12]));
        assert!(line.data.windows(3).any(|window| window == [5, 7, 1]));
    }

    #[test]
    fn debug_sections_include_constant_declarations_and_source_files() {
        let constants = [DebugConstant {
            name: "ARRAY_ADDR".to_string(),
            file: "include/constants.asm".to_string(),
            line: 12,
            value: 0x4000,
        }];
        let sections = debug_sections(&[], &constants, Path::new("."));

        assert!(section(&sections, ".debug_str").data.windows(b"ARRAY_ADDR\0".len())
            .any(|window| window == b"ARRAY_ADDR\0"));
        let line = section(&sections, ".debug_line");
        assert!(line.data.windows(b"constants.asm\0\x01\0\0".len())
            .any(|window| window == b"constants.asm\0\x01\0\0"));
        let name_offset = section(&sections, ".debug_str").data
            .windows(b"ARRAY_ADDR\0".len())
            .position(|window| window == b"ARRAY_ADDR\0")
            .unwrap() as u32;
        let mut declaration = vec![2];
        declaration.extend_from_slice(&name_offset.to_le_bytes());
        declaration.extend_from_slice(&[1, 12, 0x80, 0x80, 0x01]);
        let info = &section(&sections, ".debug_info").data;
        assert!(info.windows(declaration.len()).any(|window| window == declaration));
    }
}