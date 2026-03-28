use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;
use serde_json;

use crate::assembler::{Assembler, DebugInfo};
use crate::diagnostics::{AsmError, AsmResult};

// Maximum number of bytes to display in the listing BYTES column
const LISTING_MAX_BYTES: usize = 8;

/// ROM output configuration
pub struct RomConfig {
    pub rom_align: u16,
}

impl Default for RomConfig {
    fn default() -> Self {
        Self { rom_align: 1 }
    }
}

/// Generate the ROM binary from assembled output
pub fn generate_rom(asm: &Assembler, config: &RomConfig) -> Vec<u8> {
    let mut rom = asm.output.extract_rom();

    // Apply ROM alignment (pad end to multiple of rom_align)
    if config.rom_align > 1 {
        let align = config.rom_align as usize;
        let remainder = rom.len() % align;
        if remainder != 0 {
            rom.resize(rom.len() + (align - remainder), 0);
        }
    }

    rom
}

/// Get the start address of the ROM
pub fn rom_start_address(asm: &Assembler) -> u16 {
    asm.output.min_addr().unwrap_or(0)
}

// ---- Debug JSON output ----

#[derive(Serialize)]
struct DebugOutput {
    labels: HashMap<String, DebugLabel>,
    consts: HashMap<String, DebugConst>,
    macros: HashMap<String, DebugMacro>,
    #[serde(rename = "projectFile")]
    project_file: String,
    #[serde(rename = "lineAddresses")]
    line_addresses: HashMap<String, HashMap<String, Vec<String>>>,
    #[serde(rename = "dataLines")]
    data_lines: HashMap<String, HashMap<String, DebugDataLine>>,
    breakpoints: HashMap<String, serde_json::Value>,
}

#[derive(Serialize)]
struct DebugLabel {
    addr: String,
    src: String,
    line: usize,
}

#[derive(Serialize)]
struct DebugConst {
    value: i64,
    hex: String,
    line: usize,
    src: String,
}

#[derive(Serialize)]
struct DebugMacro {
    src: String,
    line: usize,
    params: Vec<String>,
}

#[derive(Serialize)]
struct DebugDataLine {
    addr: String,
    #[serde(rename = "byteLength")]
    byte_length: usize,
    #[serde(rename = "unitBytes")]
    unit_bytes: usize,
}

/// Generate debug JSON string matching the expected format
pub fn generate_debug_json(debug: &DebugInfo, project_file: &str) -> AsmResult<String> {
    let mut labels = HashMap::new();
    for (name, info) in &debug.labels {
        labels.insert(name.clone(), DebugLabel {
            addr: format!("0x{:04X}", info.addr),
            src: info.src.clone(),
            line: info.line,
        });
    }

    let mut consts = HashMap::new();
    for (name, info) in &debug.consts {
        consts.insert(name.clone(), DebugConst {
            value: info.value,
            hex: format!("0x{:04X}", info.value as u16),
            line: info.line,
            src: info.src.clone(),
        });
    }

    let mut macros = HashMap::new();
    for (name, info) in &debug.macros {
        macros.insert(name.clone(), DebugMacro {
            src: info.src.clone(),
            line: info.line,
            params: info.params.clone(),
        });
    }

    // Convert line_addresses: HashMap<String, HashMap<usize, Vec<u16>>>
    // to HashMap<String, HashMap<String, Vec<String>>>
    let mut line_addresses: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();
    for (file, lines) in &debug.line_addresses {
        let mut file_lines = HashMap::new();
        for (line_num, addrs) in lines {
            let addr_strs: Vec<String> = addrs.iter().map(|a| format!("0x{:04X}", a)).collect();
            file_lines.insert(line_num.to_string(), addr_strs);
        }
        line_addresses.insert(file.clone(), file_lines);
    }

    // Convert data_lines
    let mut data_lines: HashMap<String, HashMap<String, DebugDataLine>> = HashMap::new();
    for (file, lines) in &debug.data_lines {
        let mut file_lines = HashMap::new();
        for (line_num, info) in lines {
            file_lines.insert(line_num.to_string(), DebugDataLine {
                addr: format!("0x{:04X}", info.addr),
                byte_length: info.byte_length,
                unit_bytes: info.unit_bytes,
            });
        }
        data_lines.insert(file.clone(), file_lines);
    }

    let output = DebugOutput {
        labels,
        consts,
        macros,
        project_file: project_file.to_string(),
        line_addresses,
        data_lines,
        breakpoints: HashMap::new(),
    };

    serde_json::to_string_pretty(&output)
        .map_err(|e| AsmError::new(format!("Failed to serialize debug JSON: {}", e)))
}

/// Write ROM to file
pub fn write_rom(rom: &[u8], path: &Path) -> AsmResult<()> {
    std::fs::write(path, rom)
        .map_err(|e| AsmError::new(format!("Failed to write ROM file: {}", e)))
}

/// Write debug JSON to file
pub fn write_debug_json(json: &str, path: &Path) -> AsmResult<()> {
    std::fs::write(path, json)
        .map_err(|e| AsmError::new(format!("Failed to write debug file: {}", e)))
}

// ---- Listing file output ----

/// Generate listing file content from assembled data
pub fn generate_listing(asm: &Assembler) -> String {
    let mut out = String::new();
    out.push_str("ADDR   BYTES                    SOURCE\n");

    for entry in &asm.listing_data {
        let addr_str = if entry.byte_count > 0 {
            format!("{:04X}", entry.addr)
        } else {
            "    ".to_string()
        };

        let bytes_str = if entry.byte_count > 0 {
            let display_count = entry.byte_count.min(LISTING_MAX_BYTES);
            let mut hex_parts: Vec<String> = Vec::with_capacity(display_count);
            for i in 0..display_count {
                let addr = entry.addr.wrapping_add(i as u16);
                let b = asm.output.read_byte(addr).unwrap_or(0);
                hex_parts.push(format!("{:02X}", b));
            }
            let hex = hex_parts.join(" ");
            if entry.byte_count > LISTING_MAX_BYTES {
                format!("{:<23}+", hex)
            } else {
                format!("{:<24}", hex)
            }
        } else {
            " ".repeat(24)
        };

        out.push_str(&format!(
            "{}   {} {:>5}  {}\n",
            addr_str, bytes_str, entry.line_num, entry.text
        ));
    }

    out
}

/// Write listing file to disk
pub fn write_listing(listing: &str, path: &Path) -> AsmResult<()> {
    std::fs::write(path, listing)
        .map_err(|e| AsmError::new(format!("Failed to write listing file: {}", e)))
}
