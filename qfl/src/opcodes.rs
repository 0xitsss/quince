use std::fmt;

// opcode in bits 0-7 (zero-shift dispatch), rd 8-15, rs1 16-23, rs2 24-31, imm 32-63
const OP_MASK: u64 = 0xFF;

pub const OPCODE_BITS: u32 = 8;
pub const REGISTER_BITS: u32 = 8;
pub const IMM_BITS: u32 = 32;

/// Raw 64-bit instruction (opcode in bits 0-7 for zero-shift dispatch)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction(u64);

impl Instruction {
    pub fn new(opcode: Opcode, rd: u8, rs1: u8, rs2: u8, imm: u32) -> Self {
        let raw = (opcode as u64)
            | (rd as u64) << 8
            | (rs1 as u64) << 16
            | (rs2 as u64) << 24
            | (imm as u64) << 32;
        Instruction(raw)
    }

    #[inline(always)]
    pub fn raw(&self) -> u64 { self.0 }

    #[inline(always)]
    pub fn opcode(&self) -> Opcode {
        let val = (self.0 & OP_MASK) as u8;
        Opcode::from_u8(val)
    }

    #[inline(always)]
    pub fn rd(&self) -> u8 { ((self.0 >> 8) & 0xFF) as u8 }

    #[inline(always)]
    pub fn rs1(&self) -> u8 { ((self.0 >> 16) & 0xFF) as u8 }

    #[inline(always)]
    pub fn rs2(&self) -> u8 { ((self.0 >> 24) & 0xFF) as u8 }

    #[inline(always)]
    pub fn imm(&self) -> u32 { (self.0 >> 32) as u32 }

    #[inline(always)]
    pub fn imm_signed(&self) -> i32 { self.imm() as i32 }

    #[inline(always)]
    pub fn imm40(&self) -> i64 {
        let low = self.imm() as u64;          // imm = bits 32-63 = low 32 of 40-bit val
        let high = self.rs2() as u64;         // rs2 = bits 24-31 = high 8 of 40-bit val
        let val = (high << 32) | low;         // full 40-bit value
        let sign = (val >> 39) & 1;
        if sign == 1 { (val | 0xffffff0000000000) as i64 }
        else { val as i64 }
    }

    #[inline(always)]
    pub fn encode(&self) -> [u8; 8] { self.0.to_le_bytes() }

    #[inline(always)]
    pub fn decode(bytes: &[u8; 8]) -> Self { Instruction(u64::from_le_bytes(*bytes)) }
}

impl fmt::Display for Instruction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} r{}", self.opcode(), self.rd())?;
        match self.opcode().encoding() {
            InstrEncoding::RRR => write!(f, ", r{}, r{}", self.rs1(), self.rs2())?,
            InstrEncoding::RRI => write!(f, ", r{}, {}", self.rs1(), self.imm_signed())?,
            InstrEncoding::RI => write!(f, ", {}", self.imm_signed())?,
            InstrEncoding::RI40 => write!(f, ", {}", self.imm40())?,
            InstrEncoding::RR => write!(f, ", r{}", self.rs1())?,
            InstrEncoding::Single => {},
        }
        Ok(())
    }
}

// Helper constructors
impl Instruction {
    #[inline(always)]
    pub fn rrr(op: Opcode, rd: u8, rs1: u8, rs2: u8) -> Self { Self::new(op, rd, rs1, rs2, 0) }

    #[inline(always)]
    pub fn rr(op: Opcode, rd: u8, rs1: u8) -> Self { Self::new(op, rd, rs1, 0, 0) }

    #[inline(always)]
    pub fn rri(op: Opcode, rd: u8, rs1: u8, imm: u32) -> Self { Self::new(op, rd, rs1, 0, imm) }

    #[inline(always)]
    pub fn ri(op: Opcode, rd: u8, imm: u32) -> Self { Self::new(op, rd, 0, 0, imm) }

    #[inline(always)]
    pub fn single(op: Opcode) -> Self { Self::new(op, 0, 0, 0, 0) }

    #[inline(always)]
    pub fn ri40(op: Opcode, rd: u8, imm: i64) -> Self {
        let imm_u64 = imm as u64;
        let low = imm_u64 as u32;
        let high = ((imm_u64 >> 32) & 0xff) as u8;
        // opcode bits 0-7, rd 8-15, rs1=0, rs2=high, imm=low
        Self::new(op, rd, 0, high, low)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrEncoding {
    RRR, RR, RRI, RI, RI40, Single,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Opcode {
    // Int arithmetic
    Add = 0, Sub = 1, Mul = 2, Div = 3, Mod = 4, Neg = 5,
    AddI = 6, SubI = 7, MulI = 8, DivI = 9,
    // Float arithmetic
    FAdd = 10, FSub = 11, FMul = 12, FDiv = 13, FNeg = 14,
    // Int comparison
    Eq = 15, Ne = 16, Lt = 17, Gt = 18, Le = 19, Ge = 20,
    // Float comparison
    FEq = 21, FNe = 22, FLt = 23, FGt = 24, FLe = 25, FGe = 26,
    // Immediate comparison
    EqI = 27, LtI = 28, GtI = 29,
    // Bitwise
    BitAnd = 30, BitOr = 31, BitXor = 32, BitNot = 33, Shl = 34, Shr = 35,
    // Control flow
    Jmp = 36, Jz = 37, Jnz = 38, Call = 39, Ret = 40,
    // Data movement
    Mov = 41, Ldi = 42, Ldi64 = 43, Ldc = 44,
    // Type conversion
    I2F = 45, F2I = 46,
    // Engine builtins
    GetInd = 47, GetPrice = 48, GetPos = 49, GetBal = 50,
    GetDepthBid = 51, GetDepthAsk = 52, SendOrder = 53,
    PersistGet = 54, PersistSet = 55, Log = 56, Halt = 57,
    // Rolling Window opcodes
    WindowPush = 58, WindowMean = 59, WindowStddev = 60,
    WindowMin = 61, WindowMax = 62, WindowSum = 63,
    // Phase 4g: fused feature opcodes
    Ema = 64,
    // Phase 4i: log with value
    Log2 = 65,
    // Sentinel — must be last, triggers exit from dispatch loop
    Sentinel = 0xFF,
}

impl Opcode {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Opcode::Add, 1 => Opcode::Sub, 2 => Opcode::Mul,
            3 => Opcode::Div, 4 => Opcode::Mod, 5 => Opcode::Neg,
            6 => Opcode::AddI, 7 => Opcode::SubI, 8 => Opcode::MulI, 9 => Opcode::DivI,
            10 => Opcode::FAdd, 11 => Opcode::FSub, 12 => Opcode::FMul,
            13 => Opcode::FDiv, 14 => Opcode::FNeg,
            15 => Opcode::Eq, 16 => Opcode::Ne, 17 => Opcode::Lt,
            18 => Opcode::Gt, 19 => Opcode::Le, 20 => Opcode::Ge,
            21 => Opcode::FEq, 22 => Opcode::FNe, 23 => Opcode::FLt,
            24 => Opcode::FGt, 25 => Opcode::FLe, 26 => Opcode::FGe,
            27 => Opcode::EqI, 28 => Opcode::LtI, 29 => Opcode::GtI,
            30 => Opcode::BitAnd, 31 => Opcode::BitOr, 32 => Opcode::BitXor,
            33 => Opcode::BitNot, 34 => Opcode::Shl, 35 => Opcode::Shr,
            36 => Opcode::Jmp, 37 => Opcode::Jz, 38 => Opcode::Jnz,
            39 => Opcode::Call, 40 => Opcode::Ret,
            41 => Opcode::Mov, 42 => Opcode::Ldi, 43 => Opcode::Ldi64, 44 => Opcode::Ldc,
            45 => Opcode::I2F, 46 => Opcode::F2I,
            47 => Opcode::GetInd, 48 => Opcode::GetPrice, 49 => Opcode::GetPos,
            50 => Opcode::GetBal, 51 => Opcode::GetDepthBid, 52 => Opcode::GetDepthAsk,
            53 => Opcode::SendOrder, 54 => Opcode::PersistGet, 55 => Opcode::PersistSet,
            56 => Opcode::Log, 57 => Opcode::Halt,
            58 => Opcode::WindowPush, 59 => Opcode::WindowMean, 60 => Opcode::WindowStddev,
            61 => Opcode::WindowMin, 62 => Opcode::WindowMax, 63 => Opcode::WindowSum,
            64 => Opcode::Ema,
            65 => Opcode::Log2,
            _ => Opcode::Halt,
        }
    }

    pub fn encoding(&self) -> InstrEncoding {
        use Opcode::*;
        match self {
            Add | Sub | Mul | Div | Mod
            | FAdd | FSub | FMul | FDiv
            | Eq | Ne | Lt | Gt | Le | Ge
            | FEq | FNe | FLt | FGt | FLe | FGe
            | BitAnd | BitOr | BitXor | Shl | Shr
            | GetDepthBid | GetDepthAsk => InstrEncoding::RRR,

            Neg | FNeg | BitNot | Mov | I2F | F2I => InstrEncoding::RR,

            AddI | SubI | MulI | DivI
            | EqI | LtI | GtI
            | Jz | Jnz | Call
            | GetInd | GetBal
            | PersistGet | PersistSet | Ldi | Ldc
            | WindowPush => InstrEncoding::RRI,

            Jmp | GetPrice | GetPos | Log
            | WindowMean | WindowStddev | WindowMin | WindowMax | WindowSum => InstrEncoding::RI,

            Ema | Log2 => InstrEncoding::RRR,

            Ldi64 => InstrEncoding::RI40,

            Ret | Halt | SendOrder | Sentinel => InstrEncoding::Single,
        }
    }
}

impl fmt::Display for Opcode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

// Sentinel marker value for Jump Table dispatch
pub const SENTINEL_OPCODE: u8 = 0xFF;

// ── Jump Table ──
// Dispatch via function pointer array instead of match.
// Opcode in bits 0-7 → table[instr & 0xFF] with zero shift.

use crate::vm::Vm;

/// Handler signature: process one instruction in the VM.
type OpcodeHandler = unsafe extern "C" fn(vm: &mut Vm, instr: u64);

// Import all handler functions from vm module
use crate::vm::handlers::*;

/// Jump table indexed by opcode (0..=255). Unused slots point to `vm_halt`.
pub const JUMP_TABLE: [OpcodeHandler; 256] = {
    let mut table: [OpcodeHandler; 256] = [vm_halt; 256];
    table[0] = vm_add;
    table[1] = vm_sub;
    table[2] = vm_mul;
    table[3] = vm_div;
    table[4] = vm_mod;
    table[5] = vm_neg;
    table[6] = vm_addi;
    table[7] = vm_subi;
    table[8] = vm_muli;
    table[9] = vm_divi;
    table[10] = vm_fadd;
    table[11] = vm_fsub;
    table[12] = vm_fmul;
    table[13] = vm_fdiv;
    table[14] = vm_fneg;
    table[15] = vm_eq;
    table[16] = vm_ne;
    table[17] = vm_lt;
    table[18] = vm_gt;
    table[19] = vm_le;
    table[20] = vm_ge;
    table[21] = vm_feq;
    table[22] = vm_fne;
    table[23] = vm_flt;
    table[24] = vm_fgt;
    table[25] = vm_fle;
    table[26] = vm_fge;
    table[27] = vm_eqi;
    table[28] = vm_lti;
    table[29] = vm_gti;
    table[30] = vm_bitand;
    table[31] = vm_bitor;
    table[32] = vm_bitxor;
    table[33] = vm_bitnot;
    table[34] = vm_shl;
    table[35] = vm_shr;
    table[36] = vm_jmp;
    table[37] = vm_jz;
    table[38] = vm_jnz;
    table[39] = vm_call;
    table[40] = vm_ret;
    table[41] = vm_mov;
    table[42] = vm_ldi;
    table[43] = vm_ldi64;
    table[44] = vm_ldc;
    table[45] = vm_i2f;
    table[46] = vm_f2i;
    table[47] = vm_getind;
    table[48] = vm_getprice;
    table[49] = vm_getpos;
    table[50] = vm_getbal;
    table[51] = vm_getdepthbid;
    table[52] = vm_getdepthask;
    table[53] = vm_sendorder;
    table[54] = vm_persistget;
    table[55] = vm_persistset;
    table[56] = vm_log;
    table[57] = vm_halt;
    table[58] = vm_windowpush;
    table[59] = vm_windowmean;
    table[60] = vm_windowstddev;
    table[61] = vm_windowmin;
    table[62] = vm_windowmax;
    table[63] = vm_windowsum;
    table[64] = vm_ema;
    table[65] = vm_log2;
    table
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rrr_instruction_encode_decode_roundtrip_preserves_all_fields() {
        let instr = Instruction::rrr(Opcode::Add, 1, 2, 3);
        let bytes = instr.encode();
        let decoded = Instruction::decode(&bytes);
        assert_eq!(instr, decoded);
        assert_eq!(decoded.opcode(), Opcode::Add);
        assert_eq!(decoded.rd(), 1);
        assert_eq!(decoded.rs1(), 2);
        assert_eq!(decoded.rs2(), 3);
    }

    #[test]
    fn rri_instruction_stores_opcode_rd_rs1_and_immediate() {
        let instr = Instruction::rri(Opcode::AddI, 1, 2, 42);
        assert_eq!(instr.opcode(), Opcode::AddI);
        assert_eq!(instr.rd(), 1);
        assert_eq!(instr.rs1(), 2);
        assert_eq!(instr.imm(), 42);
    }

    #[test]
    fn ri40_encodes_positive_40bit_immediate() {
        let val: i64 = 0x7a_5b_3c_1d;
        let instr = Instruction::ri40(Opcode::Ldi64, 1, val);
        assert_eq!(instr.opcode(), Opcode::Ldi64);
        assert_eq!(instr.rd(), 1);
        assert_eq!(instr.imm40(), val);
    }

    #[test]
    fn ri40_encodes_negative_one_as_40bit_sign_extended() {
        let val: i64 = -1;
        let instr = Instruction::ri40(Opcode::Ldi64, 1, val);
        assert_eq!(instr.imm40(), val);
    }

    #[test]
    fn ri40_encodes_max_positive_40bit_immediate() {
        let val: i64 = (1i64 << 39) - 1;
        let instr = Instruction::ri40(Opcode::Ldi64, 1, val);
        assert_eq!(instr.imm40(), val);
    }

    #[test]
    fn single_instruction_ret_encode_decode_roundtrip() {
        let instr = Instruction::single(Opcode::Ret);
        let bytes = instr.encode();
        let decoded = Instruction::decode(&bytes);
        assert_eq!(decoded.opcode(), Opcode::Ret);
    }

    #[test]
    fn all_58_opcodes_from_u8_roundtrip_correctly() {
        for i in 0..58 {
            let op = Opcode::from_u8(i);
            assert_eq!(op as u8, i);
        }
    }

    #[test]
    fn sentinel_opcode_is_0xff() {
        assert_eq!(Opcode::Sentinel as u8, 0xFF);
    }

    #[test]
    fn opcode_in_bits_0_to_7_no_shift_needed_for_dispatch() {
        let instr = Instruction::rri(Opcode::AddI, 5, 3, 42);
        // opcode is in the lowest byte — test raw access
        assert_eq!((instr.raw() & 0xFF) as u8, Opcode::AddI as u8);
        assert_eq!((instr.raw() >> 8) & 0xFF, 5);  // rd
        assert_eq!((instr.raw() >> 16) & 0xFF, 3); // rs1
    }
}
