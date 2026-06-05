/// Constant Folding pass.
///
/// Evaluates constant integer/float expressions at compile time.
/// Operates on basic blocks (bounded by control-flow instructions).

use crate::ir::QfrProgram;
use crate::opcodes::{InstrEncoding, Instruction, Opcode as O};
use std::collections::HashMap;

/// Run the full optimization pipeline on a compiled program.
pub fn optimize(prog: &QfrProgram) -> QfrProgram {
    let prog = constant_fold(prog);
    let prog = common_subexpr_elim(&prog);
    dead_code_eliminate(&prog)
}

/// Dead Code Elimination pass.
///
/// Removes instructions unreachable from any entry point.
/// Correctly adjusts jump offsets for remaining instructions.
pub fn dead_code_eliminate(prog: &QfrProgram) -> QfrProgram {
    let n = prog.code.len();
    if n == 0 {
        return prog.clone();
    }

    // Trace reachability from each entry point
    let mut reachable = vec![false; n];
    let mut worklist: Vec<usize> = Vec::new();
    for entry in &prog.entries {
        let start = entry.code_offset as usize;
        if start < n {
            worklist.push(start);
        }
    }

    while let Some(mut pc) = worklist.pop() {
        while pc < n {
            if reachable[pc] {
                break;
            }
            reachable[pc] = true;
            let op = prog.code[pc].opcode();

            match op {
                O::Jmp => {
                    // target = pc + 1 + imm (pc points to next after fetch)
                    let imm = prog.code[pc].imm_signed() as i64;
                    let target = (pc as i64 + 1 + imm) as usize;
                    if target < n && !reachable[target] {
                        worklist.push(target);
                    }
                    break; // instructions after unconditional Jmp are unreachable
                }
                O::Jz | O::Jnz => {
                    let imm = prog.code[pc].imm_signed() as i64;
                    let target = (pc as i64 + 1 + imm) as usize;
                    if target < n && !reachable[target] {
                        worklist.push(target);
                    }
                    pc += 1; // fall-through is reachable
                }
                O::Call => {
                    // Call is like Jmp + push return address
                    // The return is handled by Ret, so we can continue tracing
                    let imm = prog.code[pc].imm_signed() as i64;
                    let target = (pc as i64 + 1 + imm) as usize;
                    if target < n && !reachable[target] {
                        worklist.push(target);
                    }
                    pc += 1; // fall-through is reachable (via Ret)
                }
                O::Ret | O::Halt => {
                    break; // execution stops
                }
                _ => {
                    pc += 1;
                }
            }
        }
    }

    // Build offset mapping: old index → new index (or None if removed)
    let mut offset_map: Vec<Option<usize>> = vec![None; n];
    let mut new_len = 0;
    for i in 0..n {
        if reachable[i] {
            offset_map[i] = Some(new_len);
            new_len += 1;
        }
    }

    // Rebuild code with adjusted jump offsets
    let mut new_code = Vec::with_capacity(new_len);
    for i in 0..n {
        if !reachable[i] {
            continue;
        }
        let instr = prog.code[i];
        let op = instr.opcode();
        let encoding = op.encoding();

        let adjusted = match encoding {
            InstrEncoding::RI => {
                // Jmp: imm = relative offset
                let old_imm = instr.imm_signed() as i64;
                let target_old = i as i64 + 1 + old_imm;
                let new_imm = adjust_jump_offset(&offset_map, i, target_old);
                Instruction::ri(op, instr.rd(), new_imm as u32)
            }
            InstrEncoding::RRI => {
                // Jz, Jnz, Call: imm = relative offset
                if matches!(op, O::Jz | O::Jnz | O::Call) {
                    let old_imm = instr.imm_signed() as i64;
                    let target_old = i as i64 + 1 + old_imm;
                    let new_imm = adjust_jump_offset(&offset_map, i, target_old);
                    Instruction::rri(op, instr.rd(), instr.rs1(), new_imm as u32)
                } else {
                    instr // not a jump — keep as-is
                }
            }
            _ => instr,
        };
        new_code.push(adjusted);
    }

    let mut out = QfrProgram::new();
    out.entries = prog.entries.clone();
    out.const_pool = prog.const_pool.clone();
    out.const_map = prog.const_map.clone();
    out.code = new_code;
    out
}

fn adjust_jump_offset(offset_map: &[Option<usize>], from_old: usize, target_old: i64) -> i64 {
    let new_idx = offset_map[from_old].expect("jump instruction should be reachable");
    if target_old < 0 || target_old as usize >= offset_map.len() {
        // Target out of bounds — leave offset as-is (will trap at runtime)
        return target_old as i64 - from_old as i64 - 1;
    }
    match offset_map[target_old as usize] {
        Some(target_new) => target_new as i64 - new_idx as i64 - 1,
        None => 0, // target was removed (shouldn't happen), fall through
    }
}

/// Common Subexpression Elimination pass.
///
/// Within a basic block, replaces repeated identical computations
/// with Mov from the first result register.
pub fn common_subexpr_elim(prog: &QfrProgram) -> QfrProgram {
    let mut out = QfrProgram::new();
    out.entries = prog.entries.clone();
    out.const_pool = prog.const_pool.clone();
    out.const_map = prog.const_map.clone();

    // Cache: (op, rs1, operand2_u32) → rd
    let mut cache: HashMap<(O, u8, u32), u8> = HashMap::new();

    for instr in &prog.code {
        let op = instr.opcode();
        let rd = instr.rd();

        // Control flow: clear cache (basic block boundary)
        if is_control_flow(op) {
            cache.clear();
            out.code.push(*instr);
            continue;
        }

        // Invalidate cache entries that depend on rd (dest reg just changed)
        invalidate_for_reg(&mut cache, rd);

        // Try CSE match for eligible operations
        let key_opt = cse_key(op, instr);
        if let Some(key) = key_opt {
            let (rs1, operand2) = key;
            if let Some(&cached_rd) = cache.get(&(op, rs1, operand2)) {
                // CSE hit: emit Mov instead
                out.code.push(Instruction::rr(O::Mov, rd, cached_rd));
                // Update cache to point to new register too
                cache.insert((op, rs1, operand2), rd);
            } else {
                // Cache this computation
                cache.insert((op, rs1, operand2), rd);
                out.code.push(*instr);
            }
        } else {
            out.code.push(*instr);
        }
    }

    out
}

/// Build CSE key for an instruction if it is eligible.
/// Returns Some((rs1, operand2)) where operand2 = rs2 (RRR) or imm (RRI).
fn cse_key(op: O, instr: &Instruction) -> Option<(u8, u32)> {
    match op.encoding() {
        InstrEncoding::RRR => {
            match op {
                O::Add | O::Sub | O::Mul | O::Div | O::Mod
                | O::FAdd | O::FSub | O::FMul | O::FDiv
                | O::Eq | O::Ne | O::Lt | O::Gt | O::Le | O::Ge
                | O::FEq | O::FNe | O::FLt | O::FGt | O::FLe | O::FGe
                | O::BitAnd | O::BitOr | O::BitXor
                | O::Shl | O::Shr => {
                    Some((instr.rs1(), instr.rs2() as u32))
                }
                _ => None,
            }
        }
        InstrEncoding::RRI => {
            match op {
                O::AddI | O::SubI | O::MulI | O::DivI
                | O::EqI | O::LtI | O::GtI => {
                    Some((instr.rs1(), instr.imm()))
                }
                _ => None,
            }
        }
        InstrEncoding::RR => {
            match op {
                O::Neg | O::FNeg | O::BitNot => {
                    Some((instr.rs1(), 0))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Remove all cache entries that depend on register `reg`.
fn invalidate_for_reg(cache: &mut HashMap<(O, u8, u32), u8>, reg: u8) {
    cache.retain(|key, val| {
        let &(op, rs1, op2) = key;
        let cached_rd = *val;
        if rs1 == reg { return false; }
        if cached_rd == reg { return false; }
        if op.encoding() == InstrEncoding::RRR || op.encoding() == InstrEncoding::RR {
            if op2 as u8 == reg { return false; }
        }
        true
    });
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
        let mut p = QfrProgram::new();
        p.entries.push(crate::ir::EntryPoint { name: "main".into(), code_offset: 0 });
        p.code = vec![I::single(O::Ret)];
        let opt = optimize(&p);
        assert_eq!(opt.code.len(), 1);
        assert_eq!(opt.code[0].opcode(), O::Ret);
    }

    #[test]
    fn optimize_with_no_entries_removes_all_code() {
        let p = make_prog(vec![I::single(O::Ret)]);
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

    // ── Dead Code Elimination ──

    #[test]
    fn dce_retains_all_when_everything_reachable() {
        let mut p = QfrProgram::new();
        p.entries.push(crate::ir::EntryPoint { name: "main".into(), code_offset: 0 });
        p.code = vec![
            I::rri(O::Ldi, 0, 0, 1),
            I::rri(O::Ldi, 1, 0, 2),
            I::rrr(O::Add, 2, 0, 1),
            I::single(O::Ret),
        ];
        let opt = dead_code_eliminate(&p);
        assert_eq!(opt.code.len(), 4);
    }

    #[test]
    fn dce_removes_code_after_unconditional_jmp() {
        let mut p = QfrProgram::new();
        p.entries.push(crate::ir::EntryPoint { name: "main".into(), code_offset: 0 });
        p.code = vec![
            I::ri(O::Jmp, 0, 3),     // [0] jump to [4]
            I::rri(O::Ldi, 0, 0, 1), // [1] unreachable
            I::rri(O::Ldi, 1, 0, 2), // [2] unreachable
            I::rrr(O::Add, 2, 0, 1), // [3] unreachable
            I::rri(O::Ldi, 3, 0, 42), // [4] target
            I::single(O::Ret),        // [5]
        ];
        // Reachable: [0], [4], [5]
        // New: 0→0, 4→1, 5→2
        let opt = dead_code_eliminate(&p);
        assert_eq!(opt.code.len(), 3);
        assert_eq!(opt.code[0].opcode(), O::Jmp);
        // Jmp at new[0], target old[4]→new[1]: offset = 1 - 0 - 1 = 0
        assert_eq!(opt.code[0].imm_signed(), 0);
        assert_eq!(opt.code[1].opcode(), O::Ldi);
        assert_eq!(opt.code[2].opcode(), O::Ret);
    }

    #[test]
    fn dce_preserves_both_branches_of_jz() {
        let mut p = QfrProgram::new();
        p.entries.push(crate::ir::EntryPoint { name: "main".into(), code_offset: 0 });
        p.code = vec![
            I::rri(O::Ldi, 0, 0, 1),  // [0]
            I::rri(O::Jz, 0, 0, 2),   // [1] if r0==0 → jump to [4]
            I::rri(O::Ldi, 1, 0, 10), // [2] fall-through
            I::single(O::Ret),         // [3]
            I::rri(O::Ldi, 2, 0, 20), // [4] branch target
            I::single(O::Ret),         // [5]
        ];
        let opt = dead_code_eliminate(&p);
        assert_eq!(opt.code.len(), 6); // nothing removed
    }

    #[test]
    fn dce_removes_dead_code_between_entry_points() {
        let mut p = QfrProgram::new();
        p.entries.push(crate::ir::EntryPoint { name: "fn1".into(), code_offset: 0 });
        p.entries.push(crate::ir::EntryPoint { name: "fn2".into(), code_offset: 4 });
        p.code = vec![
            I::rri(O::Ldi, 0, 0, 1),  // [0] fn1 entry
            I::single(O::Ret),         // [1]
            I::rri(O::Ldi, 1, 0, 99), // [2] dead — between fn1 and fn2
            I::rri(O::Ldi, 2, 0, 99), // [3] dead
            I::rri(O::Ldi, 3, 0, 42), // [4] fn2 entry
            I::single(O::Ret),         // [5]
        ];
        let opt = dead_code_eliminate(&p);
        assert_eq!(opt.code.len(), 4);
        assert_eq!(opt.code[0].opcode(), O::Ldi);
        assert_eq!(opt.code[1].opcode(), O::Ret);
        assert_eq!(opt.code[2].opcode(), O::Ldi);
        assert_eq!(opt.code[2].rd(), 3);
        assert_eq!(opt.code[3].opcode(), O::Ret);
    }

    #[test]
    fn dce_jz_still_preserves_both_paths() {
        let mut p = QfrProgram::new();
        p.entries.push(crate::ir::EntryPoint { name: "main".into(), code_offset: 0 });
        p.code = vec![
            I::rri(O::Ldi, 0, 0, 1),   // [0]
            I::rri(O::Ldi, 1, 0, 0),   // [1]
            I::rri(O::Jz, 0, 1, 3),    // [2] if r1==0 → jump to [6]
            I::rri(O::Ldi, 2, 0, 10),  // [3] fall-through
            I::single(O::Ret),          // [4]
            I::rri(O::Ldi, 3, 0, 99),  // [5] after Ret, unreachable
            I::rri(O::Ldi, 4, 0, 42),  // [6] branch target
            I::single(O::Ret),          // [7]
        ];
        // Reachable: [0][1][2][3][4][6][7], dead: [5]
        // New: 0→0, 1→1, 2→2, 3→3, 4→4, 5→removed, 6→5, 7→6
        let opt = dead_code_eliminate(&p);
        assert_eq!(opt.code.len(), 7);
        // Jz at new[2]: target old[6]→new[5], offset = 5-2-1 = 2
        assert_eq!(opt.code[2].opcode(), O::Jz);
        assert_eq!(opt.code[2].imm_signed(), 2);
    }

    #[test]
    fn dce_handles_backward_jump_loop() {
        let mut p = QfrProgram::new();
        p.entries.push(crate::ir::EntryPoint { name: "main".into(), code_offset: 0 });
        p.code = vec![
            I::rri(O::Ldi, 0, 0, 5),    // [0]
            I::rri(O::Jz, 0, 0, 3),     // [1] if r0==0 → exit to [5]
            I::rri(O::AddI, 0, 0, (-1i32) as u32),  // [2] r0 -= 1
            I::ri(O::Jmp, 0, (-3i32) as u32), // [3] back to [1] (3+1+(-3)=1)
            I::rri(O::Ldi, 1, 0, 99),   // [4] dead (after Jmp)
            I::rri(O::Ldi, 2, 0, 42),   // [5] exit
            I::single(O::Ret),           // [6]
        ];
        // Reachable: [0][1][2][3][5][6], dead: [4]
        // New: 0→0, 1→1, 2→2, 3→3, 4→removed, 5→4, 6→5
        let opt = dead_code_eliminate(&p);
        assert_eq!(opt.code.len(), 6);
        // Jmp at new[3]: target old[1]→new[1]: offset = 1-3-1 = -3
        assert_eq!(opt.code[3].opcode(), O::Jmp);
        assert_eq!(opt.code[3].imm_signed(), -3);
    }

    #[test]
    fn dce_empty_program_unchanged() {
        let p = QfrProgram::new();
        let opt = dead_code_eliminate(&p);
        assert!(opt.code.is_empty());
    }

    #[test]
    fn dce_no_entry_points_retains_all() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 1),
            I::single(O::Ret),
        ]);
        // No entries → no reachable code → all removed
        let opt = dead_code_eliminate(&p);
        assert!(opt.code.is_empty());
    }

    #[test]
    fn dce_preserves_const_pool() {
        let mut p = QfrProgram::new();
        p.entries.push(crate::ir::EntryPoint { name: "main".into(), code_offset: 0 });
        let s_idx = p.intern_string("hello");
        p.code = vec![
            I::rri(O::Ldc, 0, 0, s_idx),
            I::single(O::Ret),
        ];
        let opt = dead_code_eliminate(&p);
        assert_eq!(opt.const_pool.len(), 1);
        assert!(opt.const_map.contains_key("hello"));
    }

    #[test]
    fn dce_preserves_entry_points() {
        let mut p = QfrProgram::new();
        p.entries.push(crate::ir::EntryPoint { name: "on_trade".into(), code_offset: 0 });
        p.code = vec![I::single(O::Ret)];
        let opt = dead_code_eliminate(&p);
        assert_eq!(opt.entries.len(), 1);
        assert_eq!(opt.entries[0].name, "on_trade");
        assert_eq!(opt.entries[0].code_offset, 0);
    }

    #[test]
    fn optimize_includes_dce_in_pipeline() {
        let mut p = QfrProgram::new();
        p.entries.push(crate::ir::EntryPoint { name: "main".into(), code_offset: 0 });
        p.code = vec![
            I::rri(O::Ldi, 0, 0, 1),     // [0]
            I::ri(O::Jmp, 0, 2),          // [1] jump to [4]
            I::rri(O::Ldi, 1, 0, 99),    // [2] dead
            I::rri(O::Ldi, 2, 0, 99),    // [3] dead
            I::rri(O::Ldi, 3, 0, 42),    // [4] target
            I::single(O::Ret),            // [5]
        ];
        // Reachable: [0], [1], [4], [5]
        let opt = optimize(&p);
        assert_eq!(opt.code.len(), 4);
    }

    // ── Common Subexpression Elimination ──

    #[test]
    fn cse_same_add_replaced_with_mov() {
        let p = make_prog(vec![
            I::rrr(O::Add, 2, 0, 1), // r2 = r0 + r1
            I::rrr(O::Add, 3, 0, 1), // r3 = r0 + r1 → Mov r3, r2
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code.len(), 2);
        assert_eq!(opt.code[0].opcode(), O::Add);
        assert_eq!(opt.code[1].opcode(), O::Mov);
        assert_eq!(opt.code[1].rd(), 3);
        assert_eq!(opt.code[1].rs1(), 2);
    }

    #[test]
    fn cse_same_fadd_replaced() {
        let p = make_prog(vec![
            I::rrr(O::FAdd, 194, 192, 193),
            I::rrr(O::FAdd, 195, 192, 193),
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code.len(), 2);
        assert_eq!(opt.code[1].opcode(), O::Mov);
    }

    #[test]
    fn cse_different_op_not_eliminated() {
        let p = make_prog(vec![
            I::rrr(O::Add, 2, 0, 1),
            I::rrr(O::Sub, 3, 0, 1), // different op → not eliminated
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code.len(), 2);
        assert_eq!(opt.code[1].opcode(), O::Sub);
    }

    #[test]
    fn cse_different_regs_not_eliminated() {
        let p = make_prog(vec![
            I::rrr(O::Add, 2, 0, 1),
            I::rrr(O::Add, 3, 0, 2), // different rs2 → not eliminated
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code.len(), 2);
        assert_eq!(opt.code[1].opcode(), O::Add);
    }

    #[test]
    fn cse_cleared_on_control_flow() {
        let p = make_prog(vec![
            I::rrr(O::Add, 2, 0, 1),
            I::single(O::Ret),
            I::rrr(O::Add, 3, 0, 1), // after Ret → cache cleared → not eliminated
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code.len(), 3);
        assert_eq!(opt.code[2].opcode(), O::Add);
    }

    #[test]
    fn cse_thrice_twice_replaced() {
        let p = make_prog(vec![
            I::rrr(O::Mul, 2, 0, 1), // cached
            I::rrr(O::Mul, 3, 0, 1), // Mov r3, r2
            I::rrr(O::Mul, 4, 0, 1), // Mov r4, r3 (cache updated to r3)
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code.len(), 3);
        assert_eq!(opt.code[0].opcode(), O::Mul);
        assert_eq!(opt.code[1].opcode(), O::Mov);
        assert_eq!(opt.code[1].rs1(), 2);
        assert_eq!(opt.code[2].opcode(), O::Mov);
    }

    #[test]
    fn cse_addi_eliminated() {
        let p = make_prog(vec![
            I::rri(O::AddI, 1, 0, 5),
            I::rri(O::AddI, 2, 0, 5), // same r0, imm=5 → Mov r2, r1
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code.len(), 2);
        assert_eq!(opt.code[1].opcode(), O::Mov);
    }

    #[test]
    fn cse_muli_with_different_imm_not_eliminated() {
        let p = make_prog(vec![
            I::rri(O::MulI, 1, 0, 5),
            I::rri(O::MulI, 2, 0, 3), // different imm → not eliminated
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code.len(), 2);
        assert_eq!(opt.code[1].opcode(), O::MulI);
    }

    #[test]
    fn cse_neg_eliminated() {
        let p = make_prog(vec![
            I::rr(O::Neg, 1, 0),
            I::rr(O::Neg, 2, 0), // same r0 → Mov r2, r1
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code.len(), 2);
        assert_eq!(opt.code[1].opcode(), O::Mov);
    }

    #[test]
    fn cse_bitwise_eliminated() {
        let p = make_prog(vec![
            I::rrr(O::BitAnd, 2, 0, 1),
            I::rrr(O::BitAnd, 3, 0, 1),
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code.len(), 2);
        assert_eq!(opt.code[1].opcode(), O::Mov);
    }

    #[test]
    fn cse_comparison_eliminated() {
        let p = make_prog(vec![
            I::rrr(O::Gt, 2, 0, 1),
            I::rrr(O::Gt, 3, 0, 1),
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code.len(), 2);
        assert_eq!(opt.code[1].opcode(), O::Mov);
    }

    #[test]
    fn cse_invalidated_when_source_reg_overwritten() {
        let p = make_prog(vec![
            I::rrr(O::Add, 2, 0, 1), // cache (Add, r0, r1) → r2
            I::rri(O::Ldi, 0, 0, 5), // r0 overwritten → invalidates cache
            I::rrr(O::Add, 3, 0, 1), // no longer matches → full computation
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code.len(), 3);
        assert_eq!(opt.code[2].opcode(), O::Add); // not Mov
    }

    #[test]
    fn cse_invalidated_when_rs2_overwritten() {
        let p = make_prog(vec![
            I::rrr(O::Add, 2, 0, 1), // cache (Add, r0, r1) → r2
            I::rri(O::Ldi, 1, 0, 10), // r1 overwritten → invalidates
            I::rrr(O::Add, 3, 0, 1), // no match → full Add
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code[2].opcode(), O::Add);
    }

    #[test]
    fn cse_invalidated_when_cached_rd_overwritten() {
        let p = make_prog(vec![
            I::rrr(O::Add, 2, 0, 1), // cache (Add, r0, r1) → r2
            I::rri(O::Ldi, 2, 0, 99), // r2 overwritten → cache entry invalid
            I::rrr(O::Add, 3, 0, 1), // no match → full Add (r2's value lost)
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code[2].opcode(), O::Add);
    }

    #[test]
    fn cse_preserves_entry_points_and_const_pool() {
        let mut p = QfrProgram::new();
        p.entries.push(crate::ir::EntryPoint { name: "main".into(), code_offset: 0 });
        let _idx = p.intern_string("test");
        p.code = vec![
            I::rrr(O::Add, 2, 0, 1),
            I::rrr(O::Add, 3, 0, 1),
            I::single(O::Ret),
        ];
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.entries.len(), 1);
        assert!(opt.const_map.contains_key("test"));
        assert_eq!(opt.code.len(), 3);
        assert_eq!(opt.code[1].opcode(), O::Mov);
    }

    #[test]
    fn cse_float_neg_eliminated() {
        let p = make_prog(vec![
            I::rr(O::FNeg, 193, 192),
            I::rr(O::FNeg, 194, 192),
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code[1].opcode(), O::Mov);
    }

    #[test]
    fn cse_eq_eliminated() {
        let p = make_prog(vec![
            I::rrr(O::Eq, 2, 0, 1),
            I::rrr(O::Eq, 3, 0, 1),
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code[1].opcode(), O::Mov);
    }

    #[test]
    fn cse_empty_program() {
        let p = QfrProgram::new();
        let opt = common_subexpr_elim(&p);
        assert!(opt.code.is_empty());
    }

    #[test]
    fn cse_shl_eliminated() {
        let p = make_prog(vec![
            I::rrr(O::Shl, 2, 0, 1),
            I::rrr(O::Shl, 3, 0, 1),
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code.len(), 2);
        assert_eq!(opt.code[1].opcode(), O::Mov);
    }

    #[test]
    fn cse_chain_keeps_working_after_invalidation() {
        // Add r2, r0, r1 → cache
        // Ldi r0, 5 → invalidates
        // Add r3, r0, r1 → full Add (new cache)
        // Add r4, r0, r1 → Mov r4, r3
        let p = make_prog(vec![
            I::rrr(O::Add, 2, 0, 1),
            I::rri(O::Ldi, 0, 0, 5),
            I::rrr(O::Add, 3, 0, 1),
            I::rrr(O::Add, 4, 0, 1),
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code.len(), 4);
        assert_eq!(opt.code[0].opcode(), O::Add);
        assert_eq!(opt.code[1].opcode(), O::Ldi);
        assert_eq!(opt.code[2].opcode(), O::Add);
        assert_eq!(opt.code[3].opcode(), O::Mov);
    }

    #[test]
    fn fold_bitxor_with_constants() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 0b1100),
            I::rri(O::Ldi, 1, 0, 0b1010),
            I::rrr(O::BitXor, 2, 0, 1),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].imm_signed(), 0b0110);
    }

    #[test]
    fn fold_shr_with_constants() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 256),
            I::rri(O::Ldi, 1, 0, 8),
            I::rrr(O::Shr, 2, 0, 1),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].imm_signed(), 1);
    }

    #[test]
    fn fold_ne_with_constants() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 5),
            I::rri(O::Ldi, 1, 0, 3),
            I::rrr(O::Ne, 2, 0, 1),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].imm_signed(), 1);
    }

    #[test]
    fn fold_le_with_constants() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 3),
            I::rri(O::Ldi, 1, 0, 5),
            I::rrr(O::Le, 2, 0, 1),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].imm_signed(), 1);
    }

    #[test]
    fn fold_ge_with_constants() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 5),
            I::rri(O::Ldi, 1, 0, 3),
            I::rrr(O::Ge, 2, 0, 1),
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].imm_signed(), 1);
    }

    #[test]
    fn cse_fadd_eliminated_large_regs() {
        let p = make_prog(vec![
            I::rrr(O::FAdd, 200, 192, 193),
            I::rrr(O::FAdd, 201, 192, 193),
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code.len(), 2);
        assert_eq!(opt.code[1].opcode(), O::Mov);
    }

    #[test]
    fn dce_no_entries_removes_all() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 1),
            I::single(O::Ret),
        ]);
        let opt = dead_code_eliminate(&p);
        assert!(opt.code.is_empty());
    }
}
