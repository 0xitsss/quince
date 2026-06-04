/// Constant Folding pass.
///
/// Evaluates constant integer/float expressions at compile time.
/// Operates on basic blocks (bounded by control-flow instructions).

use crate::ir::QfrProgram;
use crate::opcodes::{Instruction, Opcode as O};
use std::collections::HashMap;

/// Run the full optimization pipeline on a compiled program.
pub fn optimize(prog: &QfrProgram) -> QfrProgram {
    constant_fold(prog)
}

/// Constant-folding pass.
/// Folds arithmetic on known-constant registers within each basic block.
pub fn constant_fold(prog: &QfrProgram) -> QfrProgram {
    let mut out = QfrProgram::new();
    out.entries = prog.entries.clone();
    out.const_pool = prog.const_pool.clone();
    out.const_map = prog.const_map.clone();

    let mut known_int: HashMap<u8, i64> = HashMap::new();
    let mut known_float: HashMap<u8, f64> = HashMap::new();

    for instr in &prog.code {
        let op = instr.opcode();

        // On control flow or side-effect: flush known state (basic block boundary)
        if is_control_flow(op) {
            known_int.clear();
            known_float.clear();
            out.code.push(*instr);
            continue;
        }

        match op {
            // Track Ldi
            O::Ldi => {
                let rd = instr.rd();
                let imm = instr.imm_signed() as i64;
                known_int.insert(rd, imm);
                out.code.push(*instr);
            }

            // Track Ldi64
            O::Ldi64 => {
                let rd = instr.rd();
                let imm = instr.imm40();
                known_int.insert(rd, imm);
                out.code.push(*instr);
            }

            // Track Ldc (float constant)
            O::Ldc => {
                let rd = instr.rd();
                let idx = instr.imm() as usize;
                if idx < out.const_pool.len() {
                    if let crate::ir::ConstEntry::F64(val) = &out.const_pool[idx] {
                        known_float.insert(rd, *val);
                    }
                }
                out.code.push(*instr);
            }

            // Track Mov (propagate known constants)
            O::Mov => {
                let rd = instr.rd();
                let rs = instr.rs1();
                if let Some(&val) = known_int.get(&rs) {
                    known_int.insert(rd, val);
                    // Replace Mov with Ldi
                    out.code.push(Instruction::rri(O::Ldi, rd, 0, val as u32));
                } else if let Some(&val) = known_float.get(&rs) {
                    known_float.insert(rd, val);
                    // Replace Mov with Ldc (need to intern the float)
                    let idx = out.intern_f64(val);
                    out.code.push(Instruction::rri(O::Ldc, rd, 0, idx));
                } else {
                    // Can't fold, clear dest from known state
                    known_int.remove(&rd);
                    known_float.remove(&rd);
                    out.code.push(*instr);
                }
            }

            // Int conversion
            O::I2F => {
                let rd = instr.rd();
                let rs = instr.rs1();
                if let Some(&val) = known_int.get(&rs) {
                    let fval = val as f64;
                    known_float.insert(rd, fval);
                    let idx = out.intern_f64(fval);
                    out.code.push(Instruction::rri(O::Ldc, rd, 0, idx));
                } else {
                    known_float.remove(&rd);
                    out.code.push(*instr);
                }
            }
            O::F2I => {
                let rd = instr.rd();
                let rs = instr.rs1();
                if let Some(&val) = known_float.get(&rs) {
                    let ival = val as i64;
                    known_int.insert(rd, ival);
                    out.code.push(Instruction::rri(O::Ldi, rd, 0, ival as u32));
                } else {
                    known_int.remove(&rd);
                    out.code.push(*instr);
                }
            }

            // ── Int arithmetic (RRR) ──
            O::Add | O::Sub | O::Mul | O::Div | O::Mod
            | O::BitAnd | O::BitOr | O::BitXor
            | O::Shl | O::Shr
            | O::Eq | O::Ne | O::Lt | O::Gt | O::Le | O::Ge => {
                fold_int_rrr(&mut out, &mut known_int, instr, op);
            }

            // ── Float arithmetic (RRR) ──
            O::FAdd | O::FSub | O::FMul | O::FDiv
            | O::FEq | O::FNe | O::FLt | O::FGt | O::FLe | O::FGe => {
                fold_float_rrr(&mut out, &mut known_float, instr, op);
            }

            // ── Int unary ──
            O::Neg => {
                let rd = instr.rd();
                let rs = instr.rs1();
                if let Some(&val) = known_int.get(&rs) {
                    let result = val.wrapping_neg();
                    known_int.insert(rd, result);
                    out.code.push(Instruction::rri(O::Ldi, rd, 0, result as u32));
                } else {
                    known_int.remove(&rd);
                    out.code.push(*instr);
                }
            }

            // ── Float unary ──
            O::FNeg => {
                let rd = instr.rd();
                let rs = instr.rs1();
                if let Some(&val) = known_float.get(&rs) {
                    let result = -val;
                    known_float.insert(rd, result);
                    let idx = out.intern_f64(result);
                    out.code.push(Instruction::rri(O::Ldc, rd, 0, idx));
                } else {
                    known_float.remove(&rd);
                    out.code.push(*instr);
                }
            }

            // ── Int immediate ──
            O::AddI => fold_int_rri(&mut out, &mut known_int, instr, |a, b| a.wrapping_add(b as i64), O::AddI),
            O::SubI => fold_int_rri(&mut out, &mut known_int, instr, |a, b| a.wrapping_sub(b as i64), O::SubI),
            O::MulI => fold_int_rri(&mut out, &mut known_int, instr, |a, b| a.wrapping_mul(b as i64), O::MulI),
            O::DivI => fold_int_rri(&mut out, &mut known_int, instr, |a, b| if b == 0 { 0 } else { a / b as i64 }, O::DivI),

            // Comparison with immediate
            O::EqI => fold_int_rri_bool(&mut out, &mut known_int, instr, |a, b| a == b as i64),
            O::LtI => fold_int_rri_bool(&mut out, &mut known_int, instr, |a, b| a < b as i64),
            O::GtI => fold_int_rri_bool(&mut out, &mut known_int, instr, |a, b| a > b as i64),

            // Window ops: may read/write state so we don't fold but clear dest
            O::WindowPush | O::WindowMean | O::WindowStddev
            | O::WindowMin | O::WindowMax | O::WindowSum => {
                known_int.remove(&instr.rd());
                known_float.remove(&instr.rd());
                out.code.push(*instr);
            }

            // Everything else: pass through, clear dest reg
            _ => {
                known_int.remove(&instr.rd());
                known_float.remove(&instr.rd());
                out.code.push(*instr);
            }
        }
    }

    out
}

fn is_control_flow(op: O) -> bool {
    matches!(op, O::Jmp | O::Jz | O::Jnz | O::Call | O::Ret | O::SendOrder | O::Halt)
}

fn fold_int_rrr(out: &mut QfrProgram, known: &mut HashMap<u8, i64>, instr: &Instruction, op: O) {
    let rd = instr.rd();
    let rs1 = instr.rs1();
    let rs2 = instr.rs2();
    if let (Some(&a), Some(&b)) = (known.get(&rs1), known.get(&rs2)) {
        let result = match op {
            O::Add => a.wrapping_add(b),
            O::Sub => a.wrapping_sub(b),
            O::Mul => a.wrapping_mul(b),
            O::Div => if b == 0 { 0 } else { a.wrapping_div(b) },
            O::Mod => if b == 0 { 0 } else { a.wrapping_rem(b) },
            O::BitAnd => a & b,
            O::BitOr => a | b,
            O::BitXor => a ^ b,
            O::Shl => a.wrapping_shl(b as u32),
            O::Shr => a.wrapping_shr(b as u32),
            O::Eq => if a == b { 1 } else { 0 },
            O::Ne => if a != b { 1 } else { 0 },
            O::Lt => if a < b { 1 } else { 0 },
            O::Gt => if a > b { 1 } else { 0 },
            O::Le => if a <= b { 1 } else { 0 },
            O::Ge => if a >= b { 1 } else { 0 },
            _ => 0,
        };
        known.insert(rd, result);
        out.code.push(Instruction::rri(O::Ldi, rd, 0, result as u32));
    } else {
        known.remove(&rd);
        out.code.push(*instr);
    }
}

fn fold_float_rrr(out: &mut QfrProgram, known: &mut HashMap<u8, f64>, instr: &Instruction, op: O) {
    let rd = instr.rd();
    let rs1 = instr.rs1();
    let rs2 = instr.rs2();
    if let (Some(&a), Some(&b)) = (known.get(&rs1), known.get(&rs2)) {
        let result = match op {
            O::FAdd => a + b,
            O::FSub => a - b,
            O::FMul => a * b,
            O::FDiv => if b == 0.0 { 0.0 } else { a / b },
            O::FEq => if (a - b).abs() < f64::EPSILON { 1.0 } else { 0.0 },
            O::FNe => if (a - b).abs() >= f64::EPSILON { 1.0 } else { 0.0 },
            O::FLt => if a < b { 1.0 } else { 0.0 },
            O::FGt => if a > b { 1.0 } else { 0.0 },
            O::FLe => if a <= b { 1.0 } else { 0.0 },
            O::FGe => if a >= b { 1.0 } else { 0.0 },
            _ => 0.0,
        };
        known.insert(rd, result);
        let idx = out.intern_f64(result);
        out.code.push(Instruction::rri(O::Ldc, rd, 0, idx));
    } else {
        known.remove(&rd);
        out.code.push(*instr);
    }
}

fn fold_int_rri(out: &mut QfrProgram, known: &mut HashMap<u8, i64>, instr: &Instruction, f: fn(i64, u32) -> i64, _orig: O) {
    let rd = instr.rd();
    let rs1 = instr.rs1();
    let imm = instr.imm();
    if let Some(&a) = known.get(&rs1) {
        let result = f(a, imm);
        known.insert(rd, result);
        out.code.push(Instruction::rri(O::Ldi, rd, 0, result as u32));
    } else {
        known.remove(&rd);
        out.code.push(*instr);
    }
}

fn fold_int_rri_bool(out: &mut QfrProgram, known: &mut HashMap<u8, i64>, instr: &Instruction, f: fn(i64, u32) -> bool) {
    let rd = instr.rd();
    let rs1 = instr.rs1();
    let imm = instr.imm();
    if let Some(&a) = known.get(&rs1) {
        let result = if f(a, imm) { 1 } else { 0 };
        known.insert(rd, result);
        out.code.push(Instruction::rri(O::Ldi, rd, 0, result as u32));
    } else {
        known.remove(&rd);
        out.code.push(*instr);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::QfrProgram;
    use crate::opcodes::Instruction as I;
    use crate::opcodes::Opcode as O;

    fn make_prog(code: Vec<I>) -> QfrProgram {
        let mut p = QfrProgram::new();
        p.code = code;
        p
    }

    // ── Int constant folding ──

    #[test]
    fn fold_int_add() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 2),
            I::rri(O::Ldi, 1, 0, 3),
            I::rrr(O::Add, 2, 0, 1),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code.len(), 3);
        assert_eq!(opt.code[2].opcode(), O::Ldi);
        assert_eq!(opt.code[2].rd(), 2);
        assert_eq!(opt.code[2].imm_signed(), 5);
    }

    #[test]
    fn fold_int_sub() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 10),
            I::rri(O::Ldi, 1, 0, 3),
            I::rrr(O::Sub, 2, 0, 1),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].opcode(), O::Ldi);
        assert_eq!(opt.code[2].imm_signed(), 7);
    }

    #[test]
    fn fold_int_mul() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 7),
            I::rri(O::Ldi, 1, 0, 6),
            I::rrr(O::Mul, 2, 0, 1),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].imm_signed(), 42);
    }

    #[test]
    fn fold_int_div() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 50),
            I::rri(O::Ldi, 1, 0, 5),
            I::rrr(O::Div, 2, 0, 1),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].imm_signed(), 10);
    }

    #[test]
    fn fold_int_mod() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 10),
            I::rri(O::Ldi, 1, 0, 3),
            I::rrr(O::Mod, 2, 0, 1),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].imm_signed(), 1);
    }

    #[test]
    fn fold_int_neg() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 42),
            I::rr(O::Neg, 1, 0),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].opcode(), O::Ldi);
        assert_eq!(opt.code[1].imm_signed(), -42);
    }

    #[test]
    fn fold_mov_propagates_lit_to_int() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 100),
            I::rr(O::Mov, 1, 0),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].opcode(), O::Ldi);
        assert_eq!(opt.code[1].rd(), 1);
        assert_eq!(opt.code[1].imm_signed(), 100);
    }

    #[test]
    fn fold_chained_arithmetic() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 2),
            I::rri(O::Ldi, 1, 0, 3),
            I::rrr(O::Add, 2, 0, 1),  // 2+3=5
            I::rri(O::Ldi, 3, 0, 4),
            I::rrr(O::Mul, 4, 2, 3), // 5*4=20
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].opcode(), O::Ldi);
        assert_eq!(opt.code[2].imm_signed(), 5);
        assert_eq!(opt.code[4].opcode(), O::Ldi);
        assert_eq!(opt.code[4].rd(), 4);
        assert_eq!(opt.code[4].imm_signed(), 20);
    }

    #[test]
    fn fold_cmp_eq() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 5),
            I::rri(O::Ldi, 1, 0, 5),
            I::rrr(O::Eq, 2, 0, 1),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].imm_signed(), 1);
    }

    #[test]
    fn fold_cmp_gt() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 10),
            I::rri(O::Ldi, 1, 0, 3),
            I::rrr(O::Gt, 2, 0, 1),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].imm_signed(), 1);
    }

    #[test]
    fn fold_cmp_lt_false() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 3),
            I::rri(O::Ldi, 1, 0, 10),
            I::rrr(O::Gt, 2, 0, 1),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].imm_signed(), 0);
    }

    // ── Bitwise ──

    #[test]
    fn fold_bitwise_and() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 0xff),
            I::rri(O::Ldi, 1, 0, 0x0f),
            I::rrr(O::BitAnd, 2, 0, 1),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].imm_signed(), 0x0f);
    }

    #[test]
    fn fold_bitwise_or() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 0xf0),
            I::rri(O::Ldi, 1, 0, 0x0f),
            I::rrr(O::BitOr, 2, 0, 1),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].imm_signed(), 0xff);
    }

    #[test]
    fn fold_shift_left() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 1),
            I::rri(O::Ldi, 1, 0, 8),
            I::rrr(O::Shl, 2, 0, 1),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].imm_signed(), 256);
    }

    // ── Float constant folding ──

    #[test]
    fn fold_float_add() {
        let mut p = QfrProgram::new();
        let idx1 = p.intern_f64(1.5);
        let idx2 = p.intern_f64(2.5);
        p.code = vec![
            I::rri(O::Ldc, 192, 0, idx1),
            I::rri(O::Ldc, 193, 0, idx2),
            I::rrr(O::FAdd, 194, 192, 193),
        ];
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].opcode(), O::Ldc);

        // Check the folded constant pool entry
        let f_idx = opt.code[2].imm() as usize;
        if let crate::ir::ConstEntry::F64(val) = &opt.const_pool[f_idx] {
            assert!((*val - 4.0).abs() < 0.0001);
        } else {
            panic!("expected F64 const entry");
        }
    }

    #[test]
    fn fold_float_sub() {
        let mut p = QfrProgram::new();
        let idx1 = p.intern_f64(10.0);
        let idx2 = p.intern_f64(3.5);
        p.code = vec![
            I::rri(O::Ldc, 192, 0, idx1),
            I::rri(O::Ldc, 193, 0, idx2),
            I::rrr(O::FSub, 194, 192, 193),
        ];
        let opt = constant_fold(&p);
        let f_idx = opt.code[2].imm() as usize;
        if let crate::ir::ConstEntry::F64(val) = &opt.const_pool[f_idx] {
            assert!((*val - 6.5).abs() < 0.0001);
        } else {
            panic!("expected F64");
        }
    }

    #[test]
    fn fold_float_mul() {
        let mut p = QfrProgram::new();
        let idx1 = p.intern_f64(3.0);
        let idx2 = p.intern_f64(1.5);
        p.code = vec![
            I::rri(O::Ldc, 192, 0, idx1),
            I::rri(O::Ldc, 193, 0, idx2),
            I::rrr(O::FMul, 194, 192, 193),
        ];
        let opt = constant_fold(&p);
        let f_idx = opt.code[2].imm() as usize;
        if let crate::ir::ConstEntry::F64(val) = &opt.const_pool[f_idx] {
            assert!((*val - 4.5).abs() < 0.0001);
        } else {
            panic!("expected F64");
        }
    }

    #[test]
    fn fold_float_neg() {
        let mut p = QfrProgram::new();
        let idx = p.intern_f64(3.14);
        p.code = vec![
            I::rri(O::Ldc, 192, 0, idx),
            I::rr(O::FNeg, 193, 192),
        ];
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].opcode(), O::Ldc);
        let f_idx = opt.code[1].imm() as usize;
        if let crate::ir::ConstEntry::F64(val) = &opt.const_pool[f_idx] {
            assert!((*val - -3.14).abs() < 0.0001);
        } else {
            panic!("expected F64");
        }
    }

    // ── Conversion folding ──

    #[test]
    fn fold_i2f() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 42),
            I::rr(O::I2F, 192, 0),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].opcode(), O::Ldc);
        let f_idx = opt.code[1].imm() as usize;
        if let crate::ir::ConstEntry::F64(val) = &opt.const_pool[f_idx] {
            assert!((*val - 42.0).abs() < 0.0001);
        } else {
            panic!("expected F64");
        }
    }

    #[test]
    fn fold_f2i() {
        let mut p = QfrProgram::new();
        let idx = p.intern_f64(3.99);
        p.code = vec![
            I::rri(O::Ldc, 192, 0, idx),
            I::rr(O::F2I, 1, 192),
        ];
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].opcode(), O::Ldi);
        assert_eq!(opt.code[1].imm_signed(), 3);
    }

    // ── Control flow boundary ──
    // After a branch, known-const state is cleared

    #[test]
    fn control_flow_clears_known_state() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 5),
            I::single(O::Ret),
            I::rri(O::Ldi, 1, 0, 10),
            I::rrr(O::Add, 2, 0, 1), // r0 no longer known after Ret
        ]);
        let opt = constant_fold(&p);
        // After Ret, const state cleared, so Add not folded
        assert_eq!(opt.code[3].opcode(), O::Add);
    }

    #[test]
    fn no_fold_on_unknown_register() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 5),
            I::rrr(O::Add, 2, 0, 1), // r1 is unknown
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].opcode(), O::Add);
    }

    // ── Ldi64 folding ──

    #[test]
    fn fold_ldi64_propagation() {
        let mut p = QfrProgram::new();
        p.code = vec![
            I::rri(O::Ldi, 0, 0, 100),
            I::ri40(O::Ldi64, 1, 1_000_000_000_000i64),
            I::rrr(O::Add, 2, 0, 1), // 100 + 1_000_000_000_000
        ];
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].opcode(), O::Ldi);
        assert_eq!(opt.code[2].imm_signed(), 100 + 1_000_000_000_000i64 as u32 as i32);
    }

    // ── Float comparison folding ──

    #[test]
    fn fold_float_cmp_eq() {
        let mut p = QfrProgram::new();
        let idx = p.intern_f64(3.14);
        p.code = vec![
            I::rri(O::Ldc, 192, 0, idx),
            I::rri(O::Ldc, 193, 0, idx),
            I::rrr(O::FEq, 2, 192, 193),
        ];
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].opcode(), O::Ldc);
    }

    #[test]
    fn fold_float_cmp_lt() {
        let mut p = QfrProgram::new();
        let i1 = p.intern_f64(1.0);
        let i2 = p.intern_f64(2.0);
        p.code = vec![
            I::rri(O::Ldc, 192, 0, i1),
            I::rri(O::Ldc, 193, 0, i2),
            I::rrr(O::FLt, 2, 192, 193),
        ];
        let opt = constant_fold(&p);
        let f_idx = opt.code[2].imm() as usize;
        if let crate::ir::ConstEntry::F64(val) = &opt.const_pool[f_idx] {
            assert!((*val - 1.0).abs() < 0.0001, "1.0 < 2.0 should be true (1.0)");
        } else {
            panic!("expected F64");
        }
    }

    // ── Optimize pipeline ──

    #[test]
    fn optimize_runs_without_panicking() {
        let p = make_prog(vec![I::single(O::Ret)]);
        let opt = optimize(&p);
        assert_eq!(opt.code.len(), 1);
        assert_eq!(opt.code[0].opcode(), O::Ret);
    }

    #[test]
    fn optimize_empty_program() {
        let p = QfrProgram::new();
        let opt = optimize(&p);
        assert!(opt.code.is_empty());
    }

    // ── Realistic strategies ──

    #[test]
    fn fold_realistic_scalper_constants() {
        // Simulate part of a scalper strategy: local spread = 0.002 * ema
        // where these are constants being multiplied
        let mut p = QfrProgram::new();
        let ema_idx = p.intern_f64(50000.0);
        let mult_idx = p.intern_f64(0.002);
        p.code = vec![
            I::rri(O::Ldc, 192, 0, ema_idx),
            I::rri(O::Ldc, 193, 0, mult_idx),
            I::rrr(O::FMul, 194, 192, 193), // 50000 * 0.002 = 100
            I::rrr(O::FMul, 195, 192, 194), // 50000 * 100 = 5_000_000
        ];
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].opcode(), O::Ldc);
        assert_eq!(opt.code[3].opcode(), O::Ldc);
    }

    #[test]
    fn control_flow_jmp_clears_state() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 5),
            I::ri(O::Jmp, 0, 3),     // unconditional jump
            I::rri(O::Ldi, 1, 0, 10),  // after jmp (would be unreachable)
            I::rrr(O::Add, 2, 0, 1),   // after jmp target
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code.len(), 4);
    }

    // ── Immediate op folding ──

    #[test]
    fn fold_addi() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 10),
            I::rri(O::AddI, 1, 0, 5),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].opcode(), O::Ldi);
        assert_eq!(opt.code[1].imm_signed(), 15);
    }

    #[test]
    fn fold_subi() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 20),
            I::rri(O::SubI, 1, 0, 7),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].imm_signed(), 13);
    }

    #[test]
    fn fold_muli() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 6),
            I::rri(O::MulI, 1, 0, 7),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].imm_signed(), 42);
    }

    #[test]
    fn fold_divi() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 100),
            I::rri(O::DivI, 1, 0, 4),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].imm_signed(), 25);
    }

    #[test]
    fn fold_eqi() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 42),
            I::rri(O::EqI, 1, 0, 42),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].imm_signed(), 1);
    }

    #[test]
    fn fold_lti() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 3),
            I::rri(O::LtI, 1, 0, 10),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].imm_signed(), 1);
    }

    #[test]
    fn fold_gti() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 15),
            I::rri(O::GtI, 1, 0, 10),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].imm_signed(), 1);
    }

    // ── Division by zero safety ──

    #[test]
    fn fold_div_by_zero_returns_zero() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 10),
            I::rri(O::Ldi, 1, 0, 0),
            I::rrr(O::Div, 2, 0, 1),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].imm_signed(), 0);
    }

    #[test]
    fn fold_float_div_by_zero_returns_zero() {
        let mut p = QfrProgram::new();
        let i1 = p.intern_f64(10.0);
        let i2 = p.intern_f64(0.0);
        p.code = vec![
            I::rri(O::Ldc, 192, 0, i1),
            I::rri(O::Ldc, 193, 0, i2),
            I::rrr(O::FDiv, 194, 192, 193),
        ];
        let opt = constant_fold(&p);
        let f_idx = opt.code[2].imm() as usize;
        if let crate::ir::ConstEntry::F64(val) = &opt.const_pool[f_idx] {
            assert!((*val - 0.0).abs() < 0.0001);
        } else {
            panic!("expected F64");
        }
    }

    // ── Entry points preserved ──

    #[test]
    fn folding_preserves_entry_points() {
        let mut p = QfrProgram::new();
        p.entries.push(crate::ir::EntryPoint {
            name: "on_trade".into(),
            code_offset: 0,
        });
        p.code = vec![
            I::rri(O::Ldi, 0, 0, 2),
            I::rri(O::Ldi, 1, 0, 3),
            I::rrr(O::Add, 2, 0, 1),
            I::single(O::Ret),
        ];
        let opt = constant_fold(&p);
        assert_eq!(opt.entries.len(), 1);
        assert_eq!(opt.entries[0].name, "on_trade");
        assert_eq!(opt.entries[0].code_offset, 0);
    }

    #[test]
    fn folding_preserves_const_pool() {
        let mut p = QfrProgram::new();
        let s_idx = p.intern_string("test");
        p.code = vec![I::single(O::Ret)];
        let opt = constant_fold(&p);
        assert_eq!(opt.const_pool.len(), 1);
        if let crate::ir::ConstEntry::String(s) = &opt.const_pool[s_idx as usize] {
            assert_eq!(s, "test");
        } else {
            panic!("expected string");
        }
    }
}
