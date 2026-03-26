use crate::diagnostics::{AsmError, AsmResult};
use super::{EncodedInstruction, ParsedOperand, Register, RegisterPair, Condition};

/// Encode an i8080 instruction
pub fn encode(mnemonic: &str, operands: &[ParsedOperand]) -> AsmResult<EncodedInstruction> {
    let upper = mnemonic.to_uppercase();
    match upper.as_str() {
        // === Data Transfer Group ===
        "MOV" => encode_mov(operands),
        "MVI" => encode_mvi(operands),
        "LXI" => encode_lxi(operands),
        "LDA" => ok_imm16(0x3A),
        "STA" => ok_imm16(0x32),
        "LHLD" => ok_imm16(0x2A),
        "SHLD" => ok_imm16(0x22),
        "LDAX" => encode_ldax_stax(operands, false),
        "STAX" => encode_ldax_stax(operands, true),
        "XCHG" => ok_implied(0xEB),

        // === Arithmetic Group ===
        "ADD" => encode_alu_reg(operands, 0x80),
        "ADC" => encode_alu_reg(operands, 0x88),
        "SUB" => encode_alu_reg(operands, 0x90),
        "SBB" => encode_alu_reg(operands, 0x98),
        "ANA" => encode_alu_reg(operands, 0xA0),
        "XRA" => encode_alu_reg(operands, 0xA8),
        "ORA" => encode_alu_reg(operands, 0xB0),
        "CMP" => encode_alu_reg(operands, 0xB8),

        "ADI" => ok_imm8(0xC6),
        "ACI" => ok_imm8(0xCE),
        "SUI" => ok_imm8(0xD6),
        "SBI" => ok_imm8(0xDE),
        "ANI" => ok_imm8(0xE6),
        "XRI" => ok_imm8(0xEE),
        "ORI" => ok_imm8(0xF6),
        "CPI" => ok_imm8(0xFE),

        "INR" => encode_inr_dcr(operands, true),
        "DCR" => encode_inr_dcr(operands, false),
        "INX" => encode_inx_dcx(operands, true),
        "DCX" => encode_inx_dcx(operands, false),
        "DAD" => encode_dad(operands),
        "DAA" => ok_implied(0x27),

        // === Logical Group ===
        "CMA" => ok_implied(0x2F),
        "STC" => ok_implied(0x37),
        "CMC" => ok_implied(0x3F),
        "RLC" => ok_implied(0x07),
        "RRC" => ok_implied(0x0F),
        "RAL" => ok_implied(0x17),
        "RAR" => ok_implied(0x1F),

        // === Branch Group ===
        "JMP" => ok_imm16(0xC3),
        "JNZ" => ok_imm16(0xC2),
        "JZ"  => ok_imm16(0xCA),
        "JNC" => ok_imm16(0xD2),
        "JC"  => ok_imm16(0xDA),
        "JPO" => ok_imm16(0xE2),
        "JPE" => ok_imm16(0xEA),
        "JP"  => ok_imm16(0xF2),
        "JM"  => ok_imm16(0xFA),

        "CALL" => ok_imm16(0xCD),
        "CNZ"  => ok_imm16(0xC4),
        "CZ"   => ok_imm16(0xCC),
        "CNC"  => ok_imm16(0xD4),
        "CC"   => ok_imm16(0xDC),
        "CPO"  => ok_imm16(0xE4),
        "CPE"  => ok_imm16(0xEC),
        "CP"   => ok_imm16(0xF4),
        "CM"   => ok_imm16(0xFC),

        "RET" => ok_implied(0xC9),
        "RNZ" => ok_implied(0xC0),
        "RZ"  => ok_implied(0xC8),
        "RNC" => ok_implied(0xD0),
        "RC"  => ok_implied(0xD8),
        "RPO" => ok_implied(0xE0),
        "RPE" => ok_implied(0xE8),
        "RP"  => ok_implied(0xF0),
        "RM"  => ok_implied(0xF8),

        "PCHL" => ok_implied(0xE9),

        // === Stack Group ===
        "PUSH" => encode_push_pop(operands, true),
        "POP"  => encode_push_pop(operands, false),
        "XTHL" => ok_implied(0xE3),
        "SPHL" => ok_implied(0xF9),

        // === I/O and Machine Control ===
        "IN"  => ok_imm8(0xDB),
        "OUT" => ok_imm8(0xD3),
        "HLT" => ok_implied(0x76),
        "NOP" => ok_implied(0x00),
        "DI"  => ok_implied(0xF3),
        "EI"  => ok_implied(0xFB),
        "RST" => encode_rst(operands),

        _ => Err(AsmError::new(format!("Unknown i8080 instruction: {}", mnemonic))),
    }
}

fn ok_implied(opcode: u8) -> AsmResult<EncodedInstruction> {
    Ok(EncodedInstruction {
        bytes: vec![opcode],
        size: 1,
        has_imm8: false,
        has_imm16: false,
    })
}

fn ok_imm8(opcode: u8) -> AsmResult<EncodedInstruction> {
    Ok(EncodedInstruction {
        bytes: vec![opcode, 0],
        size: 2,
        has_imm8: true,
        has_imm16: false,
    })
}

fn ok_imm16(opcode: u8) -> AsmResult<EncodedInstruction> {
    Ok(EncodedInstruction {
        bytes: vec![opcode, 0, 0],
        size: 3,
        has_imm8: false,
        has_imm16: true,
    })
}

fn reg_code(op: &ParsedOperand) -> AsmResult<u8> {
    match op {
        ParsedOperand::Reg(r) => Ok(r.code()),
        ParsedOperand::Memory => Ok(6), // M = register code 6
        _ => Err(AsmError::new("Expected register or M")),
    }
}

fn encode_mov(operands: &[ParsedOperand]) -> AsmResult<EncodedInstruction> {
    if operands.len() != 2 {
        return Err(AsmError::new("MOV requires two operands"));
    }
    let dst = reg_code(&operands[0])?;
    let src = reg_code(&operands[1])?;
    if dst == 6 && src == 6 {
        return Err(AsmError::new("MOV M,M is not valid (use HLT)"));
    }
    let opcode = 0x40 | (dst << 3) | src;
    ok_implied(opcode)
}

fn encode_mvi(operands: &[ParsedOperand]) -> AsmResult<EncodedInstruction> {
    if operands.len() < 1 {
        return Err(AsmError::new("MVI requires a register and immediate"));
    }
    let dst = reg_code(&operands[0])?;
    let opcode = 0x06 | (dst << 3);
    Ok(EncodedInstruction {
        bytes: vec![opcode, 0],
        size: 2,
        has_imm8: true,
        has_imm16: false,
    })
}

fn encode_lxi(operands: &[ParsedOperand]) -> AsmResult<EncodedInstruction> {
    if operands.is_empty() {
        return Err(AsmError::new("LXI requires a register pair and immediate"));
    }
    let rp = match &operands[0] {
        ParsedOperand::RegPair(rp) => rp.code(),
        _ => return Err(AsmError::new("LXI requires a register pair")),
    };
    let opcode = 0x01 | (rp << 4);
    Ok(EncodedInstruction {
        bytes: vec![opcode, 0, 0],
        size: 3,
        has_imm8: false,
        has_imm16: true,
    })
}

fn encode_ldax_stax(operands: &[ParsedOperand], is_stax: bool) -> AsmResult<EncodedInstruction> {
    if operands.is_empty() {
        return Err(AsmError::new(if is_stax { "STAX requires B or D" } else { "LDAX requires B or D" }));
    }
    let rp = match &operands[0] {
        ParsedOperand::RegPair(rp) => match rp {
            RegisterPair::BC => 0,
            RegisterPair::DE => 1,
            _ => return Err(AsmError::new("LDAX/STAX only supports B or D")),
        },
        _ => return Err(AsmError::new("LDAX/STAX requires a register pair")),
    };
    let opcode = if is_stax { 0x02 | (rp << 4) } else { 0x0A | (rp << 4) };
    ok_implied(opcode)
}

fn encode_alu_reg(operands: &[ParsedOperand], base_opcode: u8) -> AsmResult<EncodedInstruction> {
    if operands.is_empty() {
        return Err(AsmError::new("ALU instruction requires an operand"));
    }
    let src = reg_code(&operands[0])?;
    ok_implied(base_opcode | src)
}

fn encode_inr_dcr(operands: &[ParsedOperand], is_inr: bool) -> AsmResult<EncodedInstruction> {
    if operands.is_empty() {
        return Err(AsmError::new(if is_inr { "INR requires a register" } else { "DCR requires a register" }));
    }
    let r = reg_code(&operands[0])?;
    let opcode = if is_inr { 0x04 | (r << 3) } else { 0x05 | (r << 3) };
    ok_implied(opcode)
}

fn encode_inx_dcx(operands: &[ParsedOperand], is_inx: bool) -> AsmResult<EncodedInstruction> {
    if operands.is_empty() {
        return Err(AsmError::new(if is_inx { "INX requires a register pair" } else { "DCX requires a register pair" }));
    }
    let rp = match &operands[0] {
        ParsedOperand::RegPair(rp) => rp.code(),
        _ => return Err(AsmError::new("Expected register pair")),
    };
    let opcode = if is_inx { 0x03 | (rp << 4) } else { 0x0B | (rp << 4) };
    ok_implied(opcode)
}

fn encode_dad(operands: &[ParsedOperand]) -> AsmResult<EncodedInstruction> {
    if operands.is_empty() {
        return Err(AsmError::new("DAD requires a register pair"));
    }
    let rp = match &operands[0] {
        ParsedOperand::RegPair(rp) => rp.code(),
        _ => return Err(AsmError::new("DAD requires a register pair")),
    };
    ok_implied(0x09 | (rp << 4))
}

fn encode_push_pop(operands: &[ParsedOperand], is_push: bool) -> AsmResult<EncodedInstruction> {
    if operands.is_empty() {
        return Err(AsmError::new(if is_push { "PUSH requires a register pair" } else { "POP requires a register pair" }));
    }
    let rp = match &operands[0] {
        ParsedOperand::RegPair(rp) => rp.code(),
        _ => return Err(AsmError::new("Expected register pair")),
    };
    let opcode = if is_push { 0xC5 | (rp << 4) } else { 0xC1 | (rp << 4) };
    ok_implied(opcode)
}

fn encode_rst(operands: &[ParsedOperand]) -> AsmResult<EncodedInstruction> {
    if operands.is_empty() {
        return Err(AsmError::new("RST requires a vector number (0-7)"));
    }
    let n = match &operands[0] {
        ParsedOperand::RstNum(n) => *n,
        _ => return Err(AsmError::new("RST requires a vector number (0-7)")),
    };
    if n > 7 {
        return Err(AsmError::new("RST vector must be 0-7"));
    }
    ok_implied(0xC7 | (n << 3))
}

/// Get the byte size of an i8080 instruction by mnemonic (for pass 1)
pub fn instruction_size(mnemonic: &str, operands: &[ParsedOperand]) -> AsmResult<usize> {
    let enc = encode(mnemonic, operands)?;
    Ok(enc.size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nop() {
        let enc = encode("NOP", &[]).unwrap();
        assert_eq!(enc.bytes, vec![0x00]);
        assert_eq!(enc.size, 1);
    }

    #[test]
    fn test_hlt() {
        let enc = encode("HLT", &[]).unwrap();
        assert_eq!(enc.bytes, vec![0x76]);
    }

    #[test]
    fn test_mov_b_a() {
        let enc = encode("MOV", &[ParsedOperand::Reg(Register::B), ParsedOperand::Reg(Register::A)]).unwrap();
        assert_eq!(enc.bytes, vec![0x47]);
    }

    #[test]
    fn test_mov_a_m() {
        let enc = encode("MOV", &[ParsedOperand::Reg(Register::A), ParsedOperand::Memory]).unwrap();
        assert_eq!(enc.bytes, vec![0x7E]);
    }

    #[test]
    fn test_mvi_a() {
        let enc = encode("MVI", &[ParsedOperand::Reg(Register::A), ParsedOperand::Imm8]).unwrap();
        assert_eq!(enc.bytes[0], 0x3E);
        assert_eq!(enc.size, 2);
        assert!(enc.has_imm8);
    }

    #[test]
    fn test_lxi_h() {
        let enc = encode("LXI", &[ParsedOperand::RegPair(RegisterPair::HL), ParsedOperand::Imm16]).unwrap();
        assert_eq!(enc.bytes[0], 0x21);
        assert_eq!(enc.size, 3);
        assert!(enc.has_imm16);
    }

    #[test]
    fn test_lxi_sp() {
        let enc = encode("LXI", &[ParsedOperand::RegPair(RegisterPair::SP), ParsedOperand::Imm16]).unwrap();
        assert_eq!(enc.bytes[0], 0x31);
    }

    #[test]
    fn test_push_psw() {
        let enc = encode("PUSH", &[ParsedOperand::RegPair(RegisterPair::PSW)]).unwrap();
        assert_eq!(enc.bytes, vec![0xF5]);
    }

    #[test]
    fn test_pop_psw() {
        let enc = encode("POP", &[ParsedOperand::RegPair(RegisterPair::PSW)]).unwrap();
        assert_eq!(enc.bytes, vec![0xF1]);
    }

    #[test]
    fn test_add_b() {
        let enc = encode("ADD", &[ParsedOperand::Reg(Register::B)]).unwrap();
        assert_eq!(enc.bytes, vec![0x80]);
    }

    #[test]
    fn test_add_m() {
        let enc = encode("ADD", &[ParsedOperand::Memory]).unwrap();
        assert_eq!(enc.bytes, vec![0x86]);
    }

    #[test]
    fn test_jmp() {
        let enc = encode("JMP", &[]).unwrap();
        assert_eq!(enc.bytes[0], 0xC3);
        assert_eq!(enc.size, 3);
        assert!(enc.has_imm16);
    }

    #[test]
    fn test_call() {
        let enc = encode("CALL", &[]).unwrap();
        assert_eq!(enc.bytes[0], 0xCD);
    }

    #[test]
    fn test_ret() {
        let enc = encode("RET", &[]).unwrap();
        assert_eq!(enc.bytes, vec![0xC9]);
    }

    #[test]
    fn test_in_out() {
        let enc = encode("IN", &[]).unwrap();
        assert_eq!(enc.bytes[0], 0xDB);
        assert_eq!(enc.size, 2);
        assert!(enc.has_imm8);

        let enc = encode("OUT", &[]).unwrap();
        assert_eq!(enc.bytes[0], 0xD3);
    }

    #[test]
    fn test_rst() {
        let enc = encode("RST", &[ParsedOperand::RstNum(0)]).unwrap();
        assert_eq!(enc.bytes, vec![0xC7]);
        let enc = encode("RST", &[ParsedOperand::RstNum(7)]).unwrap();
        assert_eq!(enc.bytes, vec![0xFF]);
    }

    #[test]
    fn test_inr_dcr() {
        let enc = encode("INR", &[ParsedOperand::Reg(Register::A)]).unwrap();
        assert_eq!(enc.bytes, vec![0x3C]);
        let enc = encode("DCR", &[ParsedOperand::Reg(Register::B)]).unwrap();
        assert_eq!(enc.bytes, vec![0x05]);
    }

    #[test]
    fn test_inx_dcx() {
        let enc = encode("INX", &[ParsedOperand::RegPair(RegisterPair::HL)]).unwrap();
        assert_eq!(enc.bytes, vec![0x23]);
        let enc = encode("DCX", &[ParsedOperand::RegPair(RegisterPair::HL)]).unwrap();
        assert_eq!(enc.bytes, vec![0x2B]);
    }

    #[test]
    fn test_ei_di() {
        assert_eq!(encode("EI", &[]).unwrap().bytes, vec![0xFB]);
        assert_eq!(encode("DI", &[]).unwrap().bytes, vec![0xF3]);
    }

    #[test]
    fn test_rotate() {
        assert_eq!(encode("RLC", &[]).unwrap().bytes, vec![0x07]);
        assert_eq!(encode("RRC", &[]).unwrap().bytes, vec![0x0F]);
        assert_eq!(encode("RAL", &[]).unwrap().bytes, vec![0x17]);
        assert_eq!(encode("RAR", &[]).unwrap().bytes, vec![0x1F]);
    }

    #[test]
    fn test_stc_cmc_cma() {
        assert_eq!(encode("STC", &[]).unwrap().bytes, vec![0x37]);
        assert_eq!(encode("CMC", &[]).unwrap().bytes, vec![0x3F]);
        assert_eq!(encode("CMA", &[]).unwrap().bytes, vec![0x2F]);
    }

    #[test]
    fn test_conditional_jumps() {
        assert_eq!(encode("JNZ", &[]).unwrap().bytes[0], 0xC2);
        assert_eq!(encode("JZ", &[]).unwrap().bytes[0], 0xCA);
        assert_eq!(encode("JNC", &[]).unwrap().bytes[0], 0xD2);
        assert_eq!(encode("JC", &[]).unwrap().bytes[0], 0xDA);
        assert_eq!(encode("JPO", &[]).unwrap().bytes[0], 0xE2);
        assert_eq!(encode("JPE", &[]).unwrap().bytes[0], 0xEA);
        assert_eq!(encode("JP", &[]).unwrap().bytes[0], 0xF2);
        assert_eq!(encode("JM", &[]).unwrap().bytes[0], 0xFA);
    }

    #[test]
    fn test_conditional_calls() {
        assert_eq!(encode("CNZ", &[]).unwrap().bytes[0], 0xC4);
        assert_eq!(encode("CZ", &[]).unwrap().bytes[0], 0xCC);
        assert_eq!(encode("CNC", &[]).unwrap().bytes[0], 0xD4);
        assert_eq!(encode("CC", &[]).unwrap().bytes[0], 0xDC);
        assert_eq!(encode("CPO", &[]).unwrap().bytes[0], 0xE4);
        assert_eq!(encode("CPE", &[]).unwrap().bytes[0], 0xEC);
        assert_eq!(encode("CP", &[]).unwrap().bytes[0], 0xF4);
        assert_eq!(encode("CM", &[]).unwrap().bytes[0], 0xFC);
    }

    #[test]
    fn test_conditional_returns() {
        assert_eq!(encode("RNZ", &[]).unwrap().bytes, vec![0xC0]);
        assert_eq!(encode("RZ", &[]).unwrap().bytes, vec![0xC8]);
        assert_eq!(encode("RNC", &[]).unwrap().bytes, vec![0xD0]);
        assert_eq!(encode("RC", &[]).unwrap().bytes, vec![0xD8]);
        assert_eq!(encode("RPO", &[]).unwrap().bytes, vec![0xE0]);
        assert_eq!(encode("RPE", &[]).unwrap().bytes, vec![0xE8]);
        assert_eq!(encode("RP", &[]).unwrap().bytes, vec![0xF0]);
        assert_eq!(encode("RM", &[]).unwrap().bytes, vec![0xF8]);
    }

    #[test]
    fn test_dad() {
        assert_eq!(encode("DAD", &[ParsedOperand::RegPair(RegisterPair::BC)]).unwrap().bytes, vec![0x09]);
        assert_eq!(encode("DAD", &[ParsedOperand::RegPair(RegisterPair::HL)]).unwrap().bytes, vec![0x29]);
        assert_eq!(encode("DAD", &[ParsedOperand::RegPair(RegisterPair::SP)]).unwrap().bytes, vec![0x39]);
    }

    #[test]
    fn test_mov_m_a() {
        let enc = encode("MOV", &[ParsedOperand::Memory, ParsedOperand::Reg(Register::A)]).unwrap();
        assert_eq!(enc.bytes, vec![0x77]);
    }

    #[test]
    fn test_inr_l() {
        let enc = encode("INR", &[ParsedOperand::Reg(Register::L)]).unwrap();
        assert_eq!(enc.bytes, vec![0x2C]);
    }
}
