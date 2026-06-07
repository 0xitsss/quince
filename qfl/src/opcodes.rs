use std::fmt;

// opcode in bits 0-7 (zero-shift dispatch), rd 8-15, rs1 16-23, rs2 24-31, imm 32-63
const OP_MASK: u64 = 0xFF;

pub const OPCODE_BITS: u32 = 8;
pub const REGISTER_BITS: u32 = 8;
pub const IMM_BITS: u32 = 32;

// --- Section: Instruction — 64-bit encoded instruction word ---

/// Raw 64-bit instruction (opcode in bits 0-7 for zero-shift dispatch)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction(u64);

impl Instruction {
    // Constructs a new instruction from individual fields, packing into a u64:
    // bits 0-7: opcode, 8-15: rd, 16-23: rs1, 24-31: rs2, 32-63: imm
    pub fn new(opcode: Opcode, rd: u8, rs1: u8, rs2: u8, imm: u32) -> Self {
        let raw = (opcode as u64)
            | (rd as u64) << 8
            | (rs1 as u64) << 16
            | (rs2 as u64) << 24
            | (imm as u64) << 32;
        Instruction(raw)
    }

    // Returns the raw u64 bit pattern of the instruction
    #[inline(always)]
    pub fn raw(&self) -> u64 {
        self.0
    }

    // Extracts the opcode from the lowest byte (bits 0-7), zero-shift for fast dispatch
    #[inline(always)]
    pub fn opcode(&self) -> Opcode {
        let val = (self.0 & OP_MASK) as u8;
        Opcode::from_u8(val)
    }

    // Extracts the destination register (bits 8-15)
    #[inline(always)]
    pub fn rd(&self) -> u8 {
        ((self.0 >> 8) & 0xFF) as u8
    }

    // Extracts the first source register (bits 16-23)
    #[inline(always)]
    pub fn rs1(&self) -> u8 {
        ((self.0 >> 16) & 0xFF) as u8
    }

    // Extracts the second source register (bits 24-31)
    #[inline(always)]
    pub fn rs2(&self) -> u8 {
        ((self.0 >> 24) & 0xFF) as u8
    }

    // Extracts the unsigned 32-bit immediate value (bits 32-63)
    #[inline(always)]
    pub fn imm(&self) -> u32 {
        (self.0 >> 32) as u32
    }

    // Extracts the immediate as a signed 32-bit value (reinterpret cast)
    #[inline(always)]
    pub fn imm_signed(&self) -> i32 {
        self.imm() as i32
    }

    // Extracts a 40-bit signed immediate from imm (low 32) and rs2 (high 8):
    // assembles a 40-bit value then sign-extends to i64
    #[inline(always)]
    pub fn imm40(&self) -> i64 {
        let low = self.imm() as u64; // imm = bits 32-63 = low 32 of 40-bit val
        let high = self.rs2() as u64; // rs2 = bits 24-31 = high 8 of 40-bit val
        let val = (high << 32) | low; // full 40-bit value
        let sign = (val >> 39) & 1;
        if sign == 1 {
            (val | 0xffffff0000000000) as i64
        } else {
            val as i64
        }
    }

    // Encodes the instruction to a little-endian 8-byte array (for serialization)
    #[inline(always)]
    pub fn encode(&self) -> [u8; 8] {
        self.0.to_le_bytes()
    }

    // Decodes an instruction from a little-endian 8-byte array (for deserialization)
    #[inline(always)]
    pub fn decode(bytes: &[u8; 8]) -> Self {
        Instruction(u64::from_le_bytes(*bytes))
    }
}

// --- Section: Display for Instruction — provides the asm() dump format ---

impl fmt::Display for Instruction {
    // Formats the instruction as a human-readable assembly string:
    //   "OpcodeName rd, rs1, rs2" for RRR
    //   "OpcodeName rd, rs1, imm" for RRI
    //   "OpcodeName rd, imm" for RI/RI40
    //   "OpcodeName rd, rs1" for RR
    //   "OpcodeName" for Single
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?} r{}", self.opcode(), self.rd())?;
        match self.opcode().encoding() {
            InstrEncoding::RRR => write!(f, ", r{}, r{}", self.rs1(), self.rs2())?,
            InstrEncoding::RRI => write!(f, ", r{}, {}", self.rs1(), self.imm_signed())?,
            InstrEncoding::RI => write!(f, ", {}", self.imm_signed())?,
            InstrEncoding::RI40 => write!(f, ", {}", self.imm40())?,
            InstrEncoding::RR => write!(f, ", r{}", self.rs1())?,
            InstrEncoding::Single => {}
        }
        Ok(())
    }
}

// Helper constructors for each instruction encoding format
impl Instruction {
    // Builds a 3-register instruction (op, rd, rs1, rs2)
    #[inline(always)]
    pub fn rrr(op: Opcode, rd: u8, rs1: u8, rs2: u8) -> Self {
        Self::new(op, rd, rs1, rs2, 0)
    }

    // Builds a 2-register instruction (op, rd, rs1)
    #[inline(always)]
    pub fn rr(op: Opcode, rd: u8, rs1: u8) -> Self {
        Self::new(op, rd, rs1, 0, 0)
    }

    // Builds a register-register-immediate instruction (op, rd, rs1, imm)
    #[inline(always)]
    pub fn rri(op: Opcode, rd: u8, rs1: u8, imm: u32) -> Self {
        Self::new(op, rd, rs1, 0, imm)
    }

    // Builds a register-immediate instruction (op, rd, imm)
    #[inline(always)]
    pub fn ri(op: Opcode, rd: u8, imm: u32) -> Self {
        Self::new(op, rd, 0, 0, imm)
    }

    // Builds a no-operand instruction (op only)
    #[inline(always)]
    pub fn single(op: Opcode) -> Self {
        Self::new(op, 0, 0, 0, 0)
    }

    // Builds a register-40bit-immediate instruction (op, rd, imm):
    // imm is split with low 32 bits stored in the imm field and high 8 bits in rs2
    #[inline(always)]
    pub fn ri40(op: Opcode, rd: u8, imm: i64) -> Self {
        let imm_u64 = imm as u64;
        let low = imm_u64 as u32;
        let high = ((imm_u64 >> 32) & 0xff) as u8;
        // opcode bits 0-7, rd 8-15, rs1=0, rs2=high, imm=low
        Self::new(op, rd, 0, high, low)
    }
}

// --- Section: InstrEncoding — describes the operand layout of each opcode ---

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrEncoding {
    RRR,    // three registers: rd, rs1, rs2
    RR,     // two registers: rd, rs1
    RRI,    // two registers + signed 32-bit immediate: rd, rs1, imm
    RI,     // one register + signed 32-bit immediate: rd, imm
    RI40,   // one register + signed 40-bit immediate: rd, imm40 (split across rs2/imm fields)
    Single, // no operands: opcode only
}

// --- Section: Opcode enum — complete instruction set ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Opcode {
    // Int arithmetic
    Add = 0,  // rd = rs1 + rs2 (i64)
    Sub = 1,  // rd = rs1 - rs2 (i64)
    Mul = 2,  // rd = rs1 * rs2 (i64)
    Div = 3,  // rd = rs1 / rs2 (i64, integer division)
    Mod = 4,  // rd = rs1 % rs2 (i64)
    Neg = 5,  // rd = -rs1 (i64)
    AddI = 6, // rd = rs1 + imm (i64)
    SubI = 7, // rd = rs1 - imm (i64)
    MulI = 8, // rd = rs1 * imm (i64)
    DivI = 9, // rd = rs1 / imm (i64)
    // Float arithmetic
    FAdd = 10, // rd = rs1 + rs2 (f64)
    FSub = 11, // rd = rs1 - rs2 (f64)
    FMul = 12, // rd = rs1 * rs2 (f64)
    FDiv = 13, // rd = rs1 / rs2 (f64)
    FNeg = 14, // rd = -rs1 (f64)
    // Int comparison
    Eq = 15, // rd = (rs1 == rs2) ? 1 : 0 (i64)
    Ne = 16, // rd = (rs1 != rs2) ? 1 : 0 (i64)
    Lt = 17, // rd = (rs1 < rs2) ? 1 : 0 (i64)
    Gt = 18, // rd = (rs1 > rs2) ? 1 : 0 (i64)
    Le = 19, // rd = (rs1 <= rs2) ? 1 : 0 (i64)
    Ge = 20, // rd = (rs1 >= rs2) ? 1 : 0 (i64)
    // Float comparison
    FEq = 21, // rd = (rs1 == rs2) ? 1 : 0 (f64)
    FNe = 22, // rd = (rs1 != rs2) ? 1 : 0 (f64)
    FLt = 23, // rd = (rs1 < rs2) ? 1 : 0 (f64)
    FGt = 24, // rd = (rs1 > rs2) ? 1 : 0 (f64)
    FLe = 25, // rd = (rs1 <= rs2) ? 1 : 0 (f64)
    FGe = 26, // rd = (rs1 >= rs2) ? 1 : 0 (f64)
    // Immediate comparison
    EqI = 27, // rd = (rs1 == imm) ? 1 : 0 (i64)
    LtI = 28, // rd = (rs1 < imm) ? 1 : 0 (i64)
    GtI = 29, // rd = (rs1 > imm) ? 1 : 0 (i64)
    // Bitwise
    BitAnd = 30, // rd = rs1 & rs2
    BitOr = 31,  // rd = rs1 | rs2
    BitXor = 32, // rd = rs1 ^ rs2
    BitNot = 33, // rd = ~rs1
    Shl = 34,    // rd = rs1 << rs2
    Shr = 35,    // rd = rs1 >> rs2 (arithmetic, sign-extending)
    // Control flow
    Jmp = 36,  // unconditional jump to PC + imm
    Jz = 37,   // jump to PC + imm if rs1 == 0
    Jnz = 38,  // jump to PC + imm if rs1 != 0
    Call = 39, // call subroutine at PC + imm (pushes return address)
    Ret = 40,  // return from subroutine (pops return address)
    // Data movement
    Mov = 41,    // rd = rs1 (register-to-register copy)
    Ldi = 42,    // rd = imm (load 32-bit signed immediate, sign-extended)
    Ldi64 = 43,  // rd = imm40 (load 40-bit signed immediate, sign-extended)
    LdcF64 = 44, // rd = const_pool[index] (load f64 from constant pool by index)
    // Type conversion
    I2F = 45, // rd = (f64)rs1 (i64 to f64 conversion)
    F2I = 46, // rd = (i64)rs1 (f64 to i64 truncation)
    // Engine builtins
    GetInd = 47,   // rd = quince.get(symbol_index) — get indicator value by symbol index
    GetPrice = 48, // rd = current market price
    GetPos = 49,   // rd = current position
    GetBal = 50,   // rd = current balance (with symbol index in rs1)
    GetDepthBid = 51, // rd = best bid price at level (rs1, rs2)
    GetDepthAsk = 52, // rd = best ask price at level (rs1, rs2)
    SendOrder = 53, // place order (args from registers), returns order ID
    PersistGet = 54, // rd = persistent variable[index] (load from hot-reload state)
    PersistSet = 55, // persistent variable[index] = rs1 (store to hot-reload state)
    Log = 56,      // log string from symbol table at index imm
    Halt = 57,     // stop VM execution
    // Rolling Window opcodes
    WindowPush = 58,   // push value rs1 onto window identified by imm
    WindowMean = 59,   // rd = mean of window imm
    WindowStddev = 60, // rd = standard deviation of window imm
    WindowMin = 61,    // rd = minimum of window imm
    WindowMax = 62,    // rd = maximum of window imm
    WindowSum = 63,    // rd = sum of window imm
    // Phase 4g: fused feature opcodes
    Ema = 64, // rd = EMA(rs1, rs2) — exponential moving average (alpha in rs2)
    // Phase 4i: log with value
    Log2 = 65, // log string with an associated f64 value
    // Load i64 from const pool (for values > 40-bit signed range)
    LdI64 = 66, // rd = i64_consts[index] — load arbitrary i64 from constant pool
    // Split Ldc opcodes (eliminate runtime branch)
    LdcStr = 67, // rd = string_consts[index] — load string constant address
    // Power operations
    Pow = 68,  // rd = rs1 ^ rs2 (integer power)
    FPow = 69, // rd = rs1 ^ rs2 (float power)
    // Sentinel — must be last, triggers exit from dispatch loop
    Sentinel = 0xFF, // marks end of instruction stream, not a real opcode
}

impl Opcode {
    // --- Section: from_u8 — maps raw u8 to Opcode enum ---

    // Converts a raw u8 byte to the corresponding Opcode variant.
    // Unknown values map to Halt (safe default).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Opcode::Add,
            1 => Opcode::Sub,
            2 => Opcode::Mul,
            3 => Opcode::Div,
            4 => Opcode::Mod,
            5 => Opcode::Neg,
            6 => Opcode::AddI,
            7 => Opcode::SubI,
            8 => Opcode::MulI,
            9 => Opcode::DivI,
            10 => Opcode::FAdd,
            11 => Opcode::FSub,
            12 => Opcode::FMul,
            13 => Opcode::FDiv,
            14 => Opcode::FNeg,
            15 => Opcode::Eq,
            16 => Opcode::Ne,
            17 => Opcode::Lt,
            18 => Opcode::Gt,
            19 => Opcode::Le,
            20 => Opcode::Ge,
            21 => Opcode::FEq,
            22 => Opcode::FNe,
            23 => Opcode::FLt,
            24 => Opcode::FGt,
            25 => Opcode::FLe,
            26 => Opcode::FGe,
            27 => Opcode::EqI,
            28 => Opcode::LtI,
            29 => Opcode::GtI,
            30 => Opcode::BitAnd,
            31 => Opcode::BitOr,
            32 => Opcode::BitXor,
            33 => Opcode::BitNot,
            34 => Opcode::Shl,
            35 => Opcode::Shr,
            36 => Opcode::Jmp,
            37 => Opcode::Jz,
            38 => Opcode::Jnz,
            39 => Opcode::Call,
            40 => Opcode::Ret,
            41 => Opcode::Mov,
            42 => Opcode::Ldi,
            43 => Opcode::Ldi64,
            44 => Opcode::LdcF64,
            45 => Opcode::I2F,
            46 => Opcode::F2I,
            47 => Opcode::GetInd,
            48 => Opcode::GetPrice,
            49 => Opcode::GetPos,
            50 => Opcode::GetBal,
            51 => Opcode::GetDepthBid,
            52 => Opcode::GetDepthAsk,
            53 => Opcode::SendOrder,
            54 => Opcode::PersistGet,
            55 => Opcode::PersistSet,
            56 => Opcode::Log,
            57 => Opcode::Halt,
            58 => Opcode::WindowPush,
            59 => Opcode::WindowMean,
            60 => Opcode::WindowStddev,
            61 => Opcode::WindowMin,
            62 => Opcode::WindowMax,
            63 => Opcode::WindowSum,
            64 => Opcode::Ema,
            65 => Opcode::Log2,
            66 => Opcode::LdI64,
            67 => Opcode::LdcStr,
            68 => Opcode::Pow,
            69 => Opcode::FPow,
            _ => Opcode::Halt, // unknown opcode -> safe halt
        }
    }

    // --- Section: encoding — returns the operand layout format for each opcode ---

    // Returns the InstrEncoding variant that describes how operands are packed
    // for this opcode. Used for disassembly and validation.
    pub fn encoding(&self) -> InstrEncoding {
        use Opcode::*;
        match self {
            // RRR: three registers (rd, rs1, rs2)
            Add | Sub | Mul | Div | Mod | FAdd | FSub | FMul | FDiv | Eq | Ne | Lt | Gt | Le
            | Ge | FEq | FNe | FLt | FGt | FLe | FGe | BitAnd | BitOr | BitXor | Shl | Shr
            | GetDepthBid | GetDepthAsk => InstrEncoding::RRR,

            // RR: two registers (rd, rs1)
            Neg | FNeg | BitNot | Mov | I2F | F2I => InstrEncoding::RR,

            // RRI: two registers + 32-bit immediate (rd, rs1, imm)
            AddI | SubI | MulI | DivI | EqI | LtI | GtI | Jz | Jnz | Call | GetInd | GetBal
            | PersistGet | PersistSet | Ldi | LdcF64 | LdcStr | WindowPush => InstrEncoding::RRI,

            // RI: one register + 32-bit immediate (rd, imm)
            Jmp | GetPrice | GetPos | Log | WindowMean | WindowStddev | WindowMin | WindowMax
            | WindowSum => InstrEncoding::RI,

            // Fused EMA/Log2/Pow: RRR format (rd, rs1, rs2)
            Ema | Log2 | Pow | FPow => InstrEncoding::RRR,

            // RI40: one register + 40-bit immediate (rd, imm40 split across rs2/imm fields)
            Ldi64 => InstrEncoding::RI40,

            // LdI64 is RI format (rd, index into i64 constant pool)
            LdI64 => InstrEncoding::RI,

            // Single: no register operands
            Ret | Halt | SendOrder | Sentinel => InstrEncoding::Single,
        }
    }
}

impl fmt::Display for Opcode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

// --- Section: Jump Table Dispatch ---

// Sentinel marker value for Jump Table dispatch
pub const SENTINEL_OPCODE: u8 = 0xFF;

// ── Jump Table ──
// Dispatch via function pointer array instead of match.
// Opcode in bits 0-7 → table[instr & 0xFF] with zero shift.

use crate::vm::Vm;

/// Handler signature: process one instruction in the VM.
type OpcodeHandler = unsafe fn(vm: &mut Vm, instr: u64);

// Import all handler functions from vm module
use crate::vm::handlers::*;

/// Jump table indexed by opcode (0..=255). Unused slots point to `vm_halt`.
pub const JUMP_TABLE: [OpcodeHandler; 256] = {
    // Initialize all 256 slots to vm_halt (safe fallback for undefined opcodes)
    let mut table: [OpcodeHandler; 256] = [vm_halt; 256];
    // --- Int arithmetic ---
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
    // --- Float arithmetic ---
    table[10] = vm_fadd;
    table[11] = vm_fsub;
    table[12] = vm_fmul;
    table[13] = vm_fdiv;
    table[14] = vm_fneg;
    // --- Int comparison ---
    table[15] = vm_eq;
    table[16] = vm_ne;
    table[17] = vm_lt;
    table[18] = vm_gt;
    table[19] = vm_le;
    table[20] = vm_ge;
    // --- Float comparison ---
    table[21] = vm_feq;
    table[22] = vm_fne;
    table[23] = vm_flt;
    table[24] = vm_fgt;
    table[25] = vm_fle;
    table[26] = vm_fge;
    // --- Immediate comparison ---
    table[27] = vm_eqi;
    table[28] = vm_lti;
    table[29] = vm_gti;
    // --- Bitwise ---
    table[30] = vm_bitand;
    table[31] = vm_bitor;
    table[32] = vm_bitxor;
    table[33] = vm_bitnot;
    table[34] = vm_shl;
    table[35] = vm_shr;
    // --- Control flow ---
    table[36] = vm_jmp;
    table[37] = vm_jz;
    table[38] = vm_jnz;
    table[39] = vm_call;
    table[40] = vm_ret;
    // --- Data movement ---
    table[41] = vm_mov;
    table[42] = vm_ldi;
    table[43] = vm_ldi64;
    table[44] = vm_ldcf64;
    // --- Type conversion ---
    table[45] = vm_i2f;
    table[46] = vm_f2i;
    // --- Engine builtins ---
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
    // --- Rolling Window ---
    table[58] = vm_windowpush;
    table[59] = vm_windowmean;
    table[60] = vm_windowstddev;
    table[61] = vm_windowmin;
    table[62] = vm_windowmax;
    table[63] = vm_windowsum;
    // --- Fused / extended opcodes ---
    table[64] = vm_ema;
    table[65] = vm_log2;
    table[66] = vm_ldi64_c;
    table[67] = vm_ldcstr;
    table[68] = vm_pow;
    table[69] = vm_fpow;
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
    fn opcodes_from_u8_roundtrip_correctly() {
        for i in 0..70 {
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
        assert_eq!((instr.raw() >> 8) & 0xFF, 5); // rd
        assert_eq!((instr.raw() >> 16) & 0xFF, 3); // rs1
    }
}
