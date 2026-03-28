pub mod i8080;
pub mod z80_compat;

use crate::diagnostics::AsmResult;
use crate::project::CpuMode;

/// Operand types for parsed instructions
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    Register(Register),
    RegisterPair(RegisterPair),
    Immediate,  // 8-bit immediate (expression stored separately)
    Address,    // 16-bit address (expression stored separately)
    Memory,     // M or (HL)
    PortNumber, // 8-bit port number
    RstVector(u8), // RST vector 0-7
    Condition(Condition),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Register {
    A, B, C, D, E, H, L,
}

impl Register {
    pub fn code(self) -> u8 {
        match self {
            Register::B => 0,
            Register::C => 1,
            Register::D => 2,
            Register::E => 3,
            Register::H => 4,
            Register::L => 5,
            Register::A => 7,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_uppercase().as_str() {
            "A" => Some(Register::A),
            "B" => Some(Register::B),
            "C" => Some(Register::C),
            "D" => Some(Register::D),
            "E" => Some(Register::E),
            "H" => Some(Register::H),
            "L" => Some(Register::L),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterPair {
    BC, DE, HL, SP, PSW,
}

impl RegisterPair {
    pub fn code(self) -> u8 {
        match self {
            RegisterPair::BC => 0,
            RegisterPair::DE => 1,
            RegisterPair::HL => 2,
            RegisterPair::SP | RegisterPair::PSW => 3,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_uppercase().as_str() {
            "B" | "BC" => Some(RegisterPair::BC),
            "D" | "DE" => Some(RegisterPair::DE),
            "H" | "HL" => Some(RegisterPair::HL),
            "SP" => Some(RegisterPair::SP),
            "PSW" | "AF" => Some(RegisterPair::PSW),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Condition {
    NZ, Z, NC, C, PO, PE, P, M,
}

impl Condition {
    pub fn code(self) -> u8 {
        match self {
            Condition::NZ => 0,
            Condition::Z => 1,
            Condition::NC => 2,
            Condition::C => 3,
            Condition::PO => 4,
            Condition::PE => 5,
            Condition::P => 6,
            Condition::M => 7,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_uppercase().as_str() {
            "NZ" => Some(Condition::NZ),
            "Z" => Some(Condition::Z),
            "NC" => Some(Condition::NC),
            "C" => Some(Condition::C),
            "PO" => Some(Condition::PO),
            "PE" => Some(Condition::PE),
            "P" => Some(Condition::P),
            "M" => Some(Condition::M),
            _ => None,
        }
    }
}

/// Encoded instruction ready to emit
#[derive(Debug, Clone)]
pub struct EncodedInstruction {
    pub bytes: Vec<u8>,
    pub size: usize,
    /// If true, byte at index 1 is an 8-bit immediate/port to be filled from expression
    pub has_imm8: bool,
    /// If true, bytes at index 1-2 are a 16-bit immediate/address to be filled from expression
    pub has_imm16: bool,
}

/// Encode an instruction given the mnemonic, operands, and CPU mode
pub fn encode_instruction(
    mnemonic: &str,
    operands: &[ParsedOperand],
    cpu_mode: CpuMode,
) -> AsmResult<EncodedInstruction> {
    match cpu_mode {
        CpuMode::I8080 => i8080::encode(mnemonic, operands),
        CpuMode::Z80 => z80_compat::encode(mnemonic, operands),
    }
}

/// A parsed operand from the assembler parser
#[derive(Debug, Clone)]
pub enum ParsedOperand {
    Reg(Register),
    RegPair(RegisterPair),
    Memory,          // M or (HL)
    Mem16,           // (nn) 16-bit memory indirect
    Imm8,            // 8-bit immediate (expression evaluated separately)
    Imm16,           // 16-bit immediate/address
    Port,            // 8-bit port (expression evaluated separately)
    Condition(Condition),
    RstNum(u8),      // RST number 0-7
}

/// Check if a name is a reserved register identifier
pub fn is_reserved_register(name: &str) -> bool {
    let upper = name.to_uppercase();
    matches!(upper.as_str(),
        "A" | "B" | "C" | "D" | "E" | "H" | "L" |
        "BC" | "DE" | "HL" | "SP" | "AF" | "PSW" |
        "M" | "NZ" | "Z" | "NC" | "PO" | "PE" | "P" |
        "IX" | "IY" | "IXH" | "IXL" | "IYH" | "IYL" | "I" | "R"
    )
}
