use v6_core::instructions::i8080::encode;
use v6_core::instructions::{ParsedOperand, Register, RegisterPair};

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
