//! Minimal DWARF v4 metadata for relocatable V6C objects.

use std::collections::BTreeMap;
use std::path::Path;

use crate::assembler::DebugLineRow;

use super::elf::ExtraSection;
use super::section::{Reloc, RelocKind, RelocTarget, SHT_PROGBITS};

const DW_LNS_COPY: u8 = 1;
const DW_LNS_ADVANCE_LINE: u8 = 3;
const DW_LNS_SET_FILE: u8 = 4;
const DW_LNE_END_SEQUENCE: u8 = 1;
const DW_LNE_SET_ADDRESS: u8 = 2;

/// Build the small DWARF v4 subset emitted under `v6asm -g -f obj`.
pub fn debug_sections(rows: &[DebugLineRow], compilation_dir: &Path) -> Vec<ExtraSection> {
    let mut files: Vec<String> = rows.iter().map(|row| row.file.clone()).collect();
    files.sort();
    files.dedup();
    let file_indices: BTreeMap<String, u32> = files
        .iter()
        .enumerate()
        .map(|(index, file)| (file.clone(), index as u32 + 1))
        .collect();

    let unit_name = files.first().cloned().unwrap_or_default();
    let comp_dir = compilation_dir.to_string_lossy().replace('\\', "/");
    let (debug_str, producer_offset, name_offset, comp_dir_offset) =
        debug_strings(&unit_name, &comp_dir);

    vec![
        ExtraSection {
            name: ".debug_info".to_string(),
            sh_type: SHT_PROGBITS,
            flags: 0,
            addralign: 1,
            data: debug_info(producer_offset, name_offset, comp_dir_offset),
            relocs: Vec::new(),
        },
        ExtraSection {
            name: ".debug_abbrev".to_string(),
            sh_type: SHT_PROGBITS,
            flags: 0,
            addralign: 1,
            data: debug_abbrev(),
            relocs: Vec::new(),
        },
        debug_line(rows, &files, &file_indices),
        ExtraSection {
            name: ".debug_str".to_string(),
            sh_type: SHT_PROGBITS,
            flags: 0,
            addralign: 1,
            data: debug_str,
            relocs: Vec::new(),
        },
    ]
}

fn debug_strings(unit_name: &str, comp_dir: &str) -> (Vec<u8>, u32, u32, u32) {
    let mut data = Vec::new();
    let producer_offset = push_string(&mut data, "v6asm");
    let name_offset = push_string(&mut data, unit_name);
    let comp_dir_offset = push_string(&mut data, comp_dir);
    (data, producer_offset, name_offset, comp_dir_offset)
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
    data.push(0); // DW_CHILDREN_no
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
    data.push(0); // abbreviation table terminator
    data
}

fn debug_info(producer_offset: u32, name_offset: u32, comp_dir_offset: u32) -> Vec<u8> {
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
    write_initial_length(&mut data);
    data
}

fn debug_line(
    rows: &[DebugLineRow],
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
    data.push(0); // include directories terminator
    for file in files {
        data.extend_from_slice(file.as_bytes());
        data.push(0);
        write_uleb(&mut data, 0); // directory
        write_uleb(&mut data, 0); // modification time
        write_uleb(&mut data, 0); // size
    }
    data.push(0); // file table terminator
    let header_length = (data.len() - header_start) as u32;
    data[header_length_offset..header_length_offset + 4].copy_from_slice(&header_length.to_le_bytes());

    let mut relocs = Vec::new();
    let mut active_section = None;
    let mut current_line = 1i64;
    let mut current_file = 1u32;
    for row in rows.iter().filter(|row| row.is_stmt && row.section.is_some()) {
        if active_section != row.section {
            if active_section.is_some() {
                data.extend_from_slice(&[0, 1, DW_LNE_END_SEQUENCE]);
            }
            active_section = row.section;
            current_line = 1;
            current_file = 1;
        }

        data.extend_from_slice(&[0, 3, DW_LNE_SET_ADDRESS]);
        let reloc_offset = data.len() as u32;
        data.extend_from_slice(&0u16.to_le_bytes());
        relocs.push(Reloc {
            offset: reloc_offset,
            kind: RelocKind::Abs16,
            target: RelocTarget::Section(row.section.expect("checked above")),
            addend: row.offset_or_address as i64,
        });

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
        data.push(DW_LNS_COPY);
    }
    if active_section.is_some() {
        data.extend_from_slice(&[0, 1, DW_LNE_END_SEQUENCE]);
    }
    write_initial_length(&mut data);

    ExtraSection {
        name: ".debug_line".to_string(),
        sh_type: SHT_PROGBITS,
        flags: 0,
        addralign: 1,
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
    use super::{write_sleb, write_uleb};

    #[test]
    fn leb128_encoders_match_known_values() {
        let mut unsigned = Vec::new();
        write_uleb(&mut unsigned, 624_485);
        assert_eq!(unsigned, [0xe5, 0x8e, 0x26]);

        let mut signed = Vec::new();
        write_sleb(&mut signed, -624_485);
        assert_eq!(signed, [0x9b, 0xf1, 0x59]);
    }
}