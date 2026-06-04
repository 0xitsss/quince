use std::fmt;

const OP_MASK: u64 = 0xff << 56;
const RD_MASK: u64 = 0xff << 48;
const RS1_MASK: u64 = 0xff << 40;
const RS2_MASK: u64 = 0xff << 32;
const IMM_MASK: u64 = 0xffffffff;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instruction(u64);

impl Instruction {
    pub fn new(opcode: Opcode, rd: u8, rs1: u8, rs2: u8, imm: u32) -> Self {
        let raw = (opcode as u64) << 56
            | (rd as u64) << 48
            | (rs1 as u64) << 40
            | (rs2 as u64) << 32
            | (imm as u64) & IMM_MASK;
        Instruction(raw)
    }

    pub fn raw(&self) -> u64 {
        self.0
    }

    pub fn opcode(&self) -> Opcode {
        let val = ((self.0 & OP_MASK) >> 56) as u8;
        Opcode::from_u8(val)
    }

    pub fn rd(&self) -> u8 {
        ((self.0 & RD_MASK) >> 48) as u8
    }

    pub fn rs1(&self) -> u8 {
        ((self.0 & RS1_MASK) >> 40) as u8
    }

    pub fn rs2(&self) -> u8 {
        ((self.0 & RS2_MASK) >> 32) as u8
    }

    pub fn imm(&self) -> u32 {
        (self.0 & IMM_MASK) as u32
    }

    pub fn imm_signed(&self) -> i32 {
        self.imm() as i32
    }

    /// 40-bit signed immediate: rs2 << 32 | imm32, sign-extended to i64
    pub fn imm40(&self) -> i64 {
        let low = self.imm() as u64;
        let high = (self.rs2() as u64) << 32;
        let val = high | low;
        // sign extend 40-bit to 64-bit
        let sign = (val >> 39) & 1;
        if sign == 1 {
            (val | 0xffffff0000000000) as i64
        } else {
            val as i64
        }
    }

    pub fn encode(&self) -> [u8; 8] {
        self.0.to_le_bytes()
    }

    pub fn decode(bytes: &[u8; 8]) -> Self {
        Instruction(u64::from_le_bytes(*bytes))
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrEncoding {
    RRR,
    RR,
    RRI,
    RI,
    RI40,
    Single,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Opcode {
    // Int arithmetic RRR
    Add = 0,
    Sub = 1,
    Mul = 2,
    Div = 3,
    Mod = 4,
    Neg = 5,

    // Int immediate RRI
    AddI = 6,
    SubI = 7,
    MulI = 8,
    DivI = 9,

    // Float arithmetic RRR
    FAdd = 10,
    FSub = 11,
    FMul = 12,
    FDiv = 13,
    FNeg = 14,

    // Int comparison RRR
    Eq = 15,
    Ne = 16,
    Lt = 17,
    Gt = 18,
    Le = 19,
    Ge = 20,

    // Float comparison RRR
    FEq = 21,
    FNe = 22,
    FLt = 23,
    FGt = 24,
    FLe = 25,
    FGe = 26,

    // Immediate comparison RRI
    EqI = 27,
    LtI = 28,
    GtI = 29,

    // Bitwise RRR
    BitAnd = 30,
    BitOr = 31,
    BitXor = 32,
    BitNot = 33,
    Shl = 34,
    Shr = 35,

    // Control flow
    Jmp = 36,
    Jz = 37,
    Jnz = 38,
    Call = 39,
    Ret = 40,

    // Data movement
    Mov = 41,
    Ldi = 42,
    Ldi64 = 43,
    Ldc = 44,

    // Type conversion
    I2F = 45,
    F2I = 46,

    // Engine builtins
    GetInd = 47,
    GetPrice = 48,
    GetPos = 49,
    GetBal = 50,
    GetDepthBid = 51,
    GetDepthAsk = 52,
    SendOrder = 53,
    PersistGet = 54,
    PersistSet = 55,
    Log = 56,
    Halt = 57,

    // Rolling Window opcodes
    WindowPush = 58,    // RRI: push rs1 into window[imm]
    WindowMean = 59,    // RI:  rd = mean(window[imm])
    WindowStddev = 60,  // RI:  rd = stddev(window[imm])
    WindowMin = 61,     // RI:  rd = min(window[imm])
    WindowMax = 62,     // RI:  rd = max(window[imm])
    WindowSum = 63,     // RI:  rd = sum(window[imm])

    // Sentinel for array sizing
    MaxOpcode,
}

impl Opcode {
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
            44 => Opcode::Ldc,
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
            _ => Opcode::Halt,
        }
    }

    pub fn encoding(&self) -> InstrEncoding {
        use Opcode::*;
        match self {
            // RRR: 3 register
            Add | Sub | Mul | Div | Mod
            | FAdd | FSub | FMul | FDiv
            | Eq | Ne | Lt | Gt | Le | Ge
            | FEq | FNe | FLt | FGt | FLe | FGe
            | BitAnd | BitOr | BitXor | Shl | Shr
            | GetDepthBid | GetDepthAsk => InstrEncoding::RRR,

            // RR: 2 register (no imm)
            Neg | FNeg | BitNot
            | Mov | I2F | F2I => InstrEncoding::RR,

            // RRI: 2 register + imm32
            AddI | SubI | MulI | DivI
            | EqI | LtI | GtI
            | Jz | Jnz | Call
            | GetInd | GetBal
            | PersistGet | PersistSet
            | Ldi | Ldc => InstrEncoding::RRI,

            // RI: 1 register + imm32
            Jmp | GetPrice | GetPos | Log
            | WindowMean | WindowStddev | WindowMin | WindowMax | WindowSum => InstrEncoding::RI,

            // RRI: push val into window
            WindowPush => InstrEncoding::RRI,

            // RI40: 1 register + 40-bit imm
            Ldi64 => InstrEncoding::RI40,

            // Single: no operands
            Ret | Halt => InstrEncoding::Single,

            // Special: register conventions
            SendOrder => InstrEncoding::Single,

            MaxOpcode => InstrEncoding::Single,
        }
    }
}

// Helper constructors for common instruction patterns
impl Instruction {
    pub fn rrr(op: Opcode, rd: u8, rs1: u8, rs2: u8) -> Self {
        Self::new(op, rd, rs1, rs2, 0)
    }

    pub fn rr(op: Opcode, rd: u8, rs1: u8) -> Self {
        Self::new(op, rd, rs1, 0, 0)
    }

    pub fn rri(op: Opcode, rd: u8, rs1: u8, imm: u32) -> Self {
        Self::new(op, rd, rs1, 0, imm)
    }

    pub fn ri(op: Opcode, rd: u8, imm: u32) -> Self {
        Self::new(op, rd, 0, 0, imm)
    }

    pub fn single(op: Opcode) -> Self {
        Self::new(op, 0, 0, 0, 0)
    }

    pub fn ri40(op: Opcode, rd: u8, imm: i64) -> Self {
        let imm_u64 = imm as u64;
        let low = imm_u64 as u32;
        let high = ((imm_u64 >> 32) & 0xff) as u8;
        let rs2 = ((high as u64) << 56) >> 56; // extract high 8 bits
        Self::new(op, rd, 0, rs2 as u8, low)
    }
}

impl fmt::Display for Opcode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

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
        let val: i64 = 0x7a_5b_3c_1d; // fits in 40 bits
        let instr = Instruction::ri40(Opcode::Ldi64, 1, val);
        assert_eq!(instr.opcode(), Opcode::Ldi64);
        assert_eq!(instr.rd(), 1);
        assert_eq!(instr.imm40(), val);
    }

    #[test]
    fn ri40_encodes_negative_one_as_40bit_sign_extended() {
        let val: i64 = -1; // 40-bit sign-extended
        let instr = Instruction::ri40(Opcode::Ldi64, 1, val);
        assert_eq!(instr.imm40(), val);
    }

    #[test]
    fn ri40_encodes_max_positive_40bit_immediate() {
        let val: i64 = (1i64 << 39) - 1; // max positive 40-bit
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
}
