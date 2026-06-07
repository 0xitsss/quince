/// Constant Folding pass.
///
/// Evaluates constant integer/float expressions at compile time.
/// Operates on basic blocks (bounded by control-flow instructions).
use crate::ir::QfrProgram;
use crate::opcodes::{InstrEncoding, Instruction, Opcode as O};
use std::collections::HashMap;

// --- Section: Optimization Pipeline Orchestration ---

/// Run the full optimization pipeline on a compiled program.
/// Pipeline order (each pass feeds the next):
///   1. constant_fold    — evaluate constant expressions within blocks
///   2. cfg_simplify     — merge blocks, remove unreachable code, simplify jumps
///   3. sccp             — sparse conditional constant propagation (cross-block)
///   4. cse              — common subexpression elimination (per-block)
///   5. local_shadowing  — PersistGet/Set forwarding within blocks
///   6. licm             — loop-invariant code motion
///   7. loop_unroll      — unroll small constant-iteration loops
///   8. fused_lowering   — peephole patterns (Mov chains, zero-based idioms)
///   9. gvn              — global value numbering (cross-block CSE via dominators)
///  10. dce              — dead code elimination (instruction-level reachability)
///  11. persist_coalesce — redundant PersistGet/Set removal (slot-shadowing)
pub fn optimize(prog: &mut QfrProgram) {
    // Take ownership via replace to avoid pipeline-level re-cloning.
    let mut p = std::mem::replace(prog, QfrProgram::new());
    p = constant_fold(&p);
    p = cfg_simplify(&p);
    p = sccp(&p);
    p = common_subexpr_elim(&p);
    p = local_shadowing(&p);
    p = licm(&p);
    p = loop_unroll(&p);
    p = fused_lowering(&p);
    p = global_value_numbering(&p);
    p = dead_code_eliminate(&p);
    p = persist_coalesce(&p);
    *prog = p;
}

// --- Section: DCE (Dead Code Elimination) ---

/// Dead Code Elimination pass.
///
/// Removes instructions unreachable from any entry point.
/// Uses instruction-level reachability tracing (unlike CFG-based which traces blocks).
/// Correctly adjusts jump offsets for remaining instructions.
pub fn dead_code_eliminate(prog: &QfrProgram) -> QfrProgram {
    let n = prog.code.len();
    if n == 0 {
        return prog.clone();
    }

    // Trace reachability from each entry point through the instruction stream.
    // Follows control flow edges (jumps, calls) and falls through otherwise.
    let mut reachable = vec![false; n];
    let mut worklist: Vec<usize> = Vec::new();
    for entry in &prog.entries {
        let start = entry.code_offset as usize;
        if start < n {
            worklist.push(start);
        }
    }

    while let Some(mut pc) = worklist.pop() {
        // Walk sequentially from pc until we hit a terminator or already-visited instruction
        while pc < n {
            if reachable[pc] {
                break; // already traced through this path
            }
            reachable[pc] = true;
            let op = prog.code[pc].opcode();

            match op {
                O::Jmp => {
                    // Unconditional jump: record target and stop (sequential fall-through stops here)
                    let imm = prog.code[pc].imm_signed() as i64;
                    let target = (pc as i64 + 1 + imm) as usize;
                    if target < n && !reachable[target] {
                        worklist.push(target);
                    }
                    break;
                }
                O::Jz | O::Jnz => {
                    // Conditional jump: both target and fall-through are reachable
                    let imm = prog.code[pc].imm_signed() as i64;
                    let target = (pc as i64 + 1 + imm) as usize;
                    if target < n && !reachable[target] {
                        worklist.push(target);
                    }
                    pc += 1; // continue tracing fall-through
                }
                O::Call => {
                    // Call: target is reachable (will return via Ret), and fall-through is reachable
                    let imm = prog.code[pc].imm_signed() as i64;
                    let target = (pc as i64 + 1 + imm) as usize;
                    if target < n && !reachable[target] {
                        worklist.push(target);
                    }
                    pc += 1; // fall-through after call return
                }
                O::Ret | O::Halt => {
                    break; // execution terminates here
                }
                _ => {
                    pc += 1; // data instruction: keep tracing forward
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
    #[allow(clippy::needless_range_loop)]
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

    // Adjust entry point offsets after code removal
    let mut adjusted_entries = prog.entries.clone();
    for entry in &mut adjusted_entries {
        let old = entry.code_offset as usize;
        if old < offset_map.len() {
            if let Some(new_off) = offset_map[old] {
                entry.code_offset = new_off as u32;
            } else {
                // Entry offset was removed — code unreachable, set to 0 (will Ret immediately)
                entry.code_offset = 0;
            }
        }
    }

    let mut out = QfrProgram::new();
    out.entries = adjusted_entries;
    out.const_pool = prog.const_pool.clone();
    out.const_map = prog.const_map.clone();
    out.ema_alphas = prog.ema_alphas.clone();
    out.f64_consts = prog.f64_consts.clone();
    out.i64_consts = prog.i64_consts.clone();
    out.string_consts = prog.string_consts.clone();
    out.code = new_code;
    out
}

// Adjusts a jump offset from old code positions to new code positions.
// Returns the new relative offset (target_new - from_new - 1).
// If target is out of bounds, calculates offset based on current position.
fn adjust_jump_offset(offset_map: &[Option<usize>], from_old: usize, target_old: i64) -> i64 {
    let new_idx = offset_map[from_old].expect("jump instruction should be reachable");
    if target_old < 0 || target_old as usize >= offset_map.len() {
        // Target out of bounds — leave offset as-is (will trap at runtime)
        return target_old - from_old as i64 - 1;
    }
    match offset_map[target_old as usize] {
        Some(target_new) => target_new as i64 - new_idx as i64 - 1,
        None => 0, // target was removed (shouldn't happen), fall through
    }
}

// --- Section: CSE (Common Subexpression Elimination) ---

/// Common Subexpression Elimination pass.
///
/// Within a basic block, replaces repeated identical computations
/// with Mov from the first result register. Uses a hashmap keyed on
/// (opcode, rs1, operand2) to detect duplicates within the block.
pub fn common_subexpr_elim(prog: &QfrProgram) -> QfrProgram {
    let mut out = QfrProgram::new();
    out.entries = prog.entries.clone();
    out.const_pool = prog.const_pool.clone();
    out.const_map = prog.const_map.clone();
    out.ema_alphas = prog.ema_alphas.clone();
    out.f64_consts = prog.f64_consts.clone();
    out.i64_consts = prog.i64_consts.clone();
    out.string_consts = prog.string_consts.clone();

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

// Builds a value-numbering key (rs1, operand2) for CSE matching.
// Returns None for ineligible opcodes (side-effecting, control-flow, etc.).
// operand2 is rs2 for RRR encoding or imm for RRI encoding.
fn cse_key(op: O, instr: &Instruction) -> Option<(u8, u32)> {
    match op.encoding() {
        InstrEncoding::RRR => match op {
            O::Add
            | O::Sub
            | O::Mul
            | O::Div
            | O::Mod
            | O::FAdd
            | O::FSub
            | O::FMul
            | O::FDiv
            | O::Eq
            | O::Ne
            | O::Lt
            | O::Gt
            | O::Le
            | O::Ge
            | O::FEq
            | O::FNe
            | O::FLt
            | O::FGt
            | O::FLe
            | O::FGe
            | O::BitAnd
            | O::BitOr
            | O::BitXor
            | O::Shl
            | O::Shr => Some((instr.rs1(), instr.rs2() as u32)),
            _ => None,
        },
        InstrEncoding::RRI => match op {
            O::AddI | O::SubI | O::MulI | O::DivI | O::EqI | O::LtI | O::GtI => {
                Some((instr.rs1(), instr.imm()))
            }
            _ => None,
        },
        InstrEncoding::RR => match op {
            O::Neg | O::FNeg | O::BitNot => Some((instr.rs1(), 0)),
            _ => None,
        },
        _ => None,
    }
}

// Removes all CSE cache entries that depend on register `reg`.
// An entry depends on reg if reg is rs1, rs2, imm (for RRR/RR), or the cached rd.
fn invalidate_for_reg(cache: &mut HashMap<(O, u8, u32), u8>, reg: u8) {
    cache.retain(|key, val| {
        let &(op, rs1, op2) = key;
        let cached_rd = *val;
        if rs1 == reg {
            return false;
        }
        if cached_rd == reg {
            return false;
        }
        if (op.encoding() == InstrEncoding::RRR || op.encoding() == InstrEncoding::RR) && op2 as u8 == reg {
            return false;
        }
        true
    });
}

// --- Section: Constant Folding (constant_fold) ---

/// Constant-folding pass.
/// Folds arithmetic on known-constant registers within each basic block.
pub fn constant_fold(prog: &QfrProgram) -> QfrProgram {
    let mut out = QfrProgram::new();
    out.entries = prog.entries.clone();
    out.const_pool = prog.const_pool.clone();
    out.const_map = prog.const_map.clone();
    out.ema_alphas = prog.ema_alphas.clone();
    out.f64_consts = prog.f64_consts.clone();
    out.i64_consts = prog.i64_consts.clone();
    out.string_consts = prog.string_consts.clone();

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

            // Track LdcF64 (float constant)
            O::LdcF64 => {
                let rd = instr.rd();
                let idx = instr.imm() as usize;
                if idx < out.f64_consts.len() {
                    known_float.insert(rd, out.f64_consts[idx]);
                }
                out.code.push(*instr);
            }

            // Track Mov (propagate known constants)
            O::Mov => {
                let rd = instr.rd();
                let rs = instr.rs1();
                if let Some(&val) = known_int.get(&rs) {
                    known_int.insert(rd, val);
                    // Replace Mov with appropriate load instruction
                    if val >= i32::MIN as i64 && val <= i32::MAX as i64 {
                        out.code.push(Instruction::rri(O::Ldi, rd, 0, val as u32));
                    } else if (-(1i64 << 39)..(1i64 << 39)).contains(&val) {
                        out.code.push(Instruction::ri40(O::Ldi64, rd, val));
                    } else {
                        let idx = out.intern_f64(val as f64);
                        out.code.push(Instruction::rri(O::LdcF64, rd, 0, idx));
                    }
                } else if let Some(&val) = known_float.get(&rs) {
                    known_float.insert(rd, val);
                    // Replace Mov with Ldc (need to intern the float)
                    let idx = out.intern_f64(val);
                    out.code.push(Instruction::rri(O::LdcF64, rd, 0, idx));
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
                    out.code.push(Instruction::rri(O::LdcF64, rd, 0, idx));
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
                    emit_ldi_value(&mut out, rd, ival);
                } else {
                    known_int.remove(&rd);
                    out.code.push(*instr);
                }
            }

            // ── Int arithmetic (RRR) ──
            O::Add
            | O::Sub
            | O::Mul
            | O::Div
            | O::Mod
            | O::BitAnd
            | O::BitOr
            | O::BitXor
            | O::Shl
            | O::Shr
            | O::Eq
            | O::Ne
            | O::Lt
            | O::Gt
            | O::Le
            | O::Ge => {
                fold_int_rrr(&mut out, &mut known_int, instr, op);
            }

            // ── Float arithmetic (RRR) ──
            O::FAdd
            | O::FSub
            | O::FMul
            | O::FDiv
            | O::FEq
            | O::FNe
            | O::FLt
            | O::FGt
            | O::FLe
            | O::FGe => {
                fold_float_rrr(&mut out, &mut known_float, instr, op);
            }

            // ── Int unary ──
            O::Neg => {
                let rd = instr.rd();
                let rs = instr.rs1();
                if let Some(&val) = known_int.get(&rs) {
                    let result = val.wrapping_neg();
                    known_int.insert(rd, result);
                    emit_ldi_value(&mut out, rd, result);
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
                    out.code.push(Instruction::rri(O::LdcF64, rd, 0, idx));
                } else {
                    known_float.remove(&rd);
                    out.code.push(*instr);
                }
            }

            // ── Int immediate ──
            O::AddI => fold_int_rri(
                &mut out,
                &mut known_int,
                instr,
                |a, b| a.wrapping_add(b as i64),
                O::AddI,
            ),
            O::SubI => fold_int_rri(
                &mut out,
                &mut known_int,
                instr,
                |a, b| a.wrapping_sub(b as i64),
                O::SubI,
            ),
            O::MulI => fold_int_rri(
                &mut out,
                &mut known_int,
                instr,
                |a, b| a.wrapping_mul(b as i64),
                O::MulI,
            ),
            O::DivI => fold_int_rri(
                &mut out,
                &mut known_int,
                instr,
                |a, b| if b == 0 { 0 } else { a / b as i64 },
                O::DivI,
            ),

            // Comparison with immediate
            O::EqI => fold_int_rri_bool(&mut out, &mut known_int, instr, |a, b| a == b as i64),
            O::LtI => fold_int_rri_bool(&mut out, &mut known_int, instr, |a, b| a < b as i64),
            O::GtI => fold_int_rri_bool(&mut out, &mut known_int, instr, |a, b| a > b as i64),

            // Window ops: may read/write state so we don't fold but clear dest
            O::WindowPush
            | O::WindowMean
            | O::WindowStddev
            | O::WindowMin
            | O::WindowMax
            | O::WindowSum => {
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

// Emits the most compact encoding for an integer constant: Ldi (32-bit), Ldi64 (40-bit), or LdcF64 (64-bit via const pool).
fn emit_ldi_value(out: &mut QfrProgram, rd: u8, val: i64) {
    if val >= i32::MIN as i64 && val <= i32::MAX as i64 {
        out.code.push(Instruction::rri(O::Ldi, rd, 0, val as u32));
    } else if (-(1i64 << 39)..(1i64 << 39)).contains(&val) {
        out.code.push(Instruction::ri40(O::Ldi64, rd, val));
    } else {
        let idx = out.intern_f64(val as f64);
        out.code.push(Instruction::rri(O::LdcF64, rd, 0, idx));
    }
}

// --- Section: Local Shadowing & Store Forwarding ---
// Eliminates redundant PersistGet/PersistSet pairs within basic blocks.
// Tracks which persist slots are live in registers; replaces redundant
// PersistGet with Mov and coalesces multiple PersistSet to the final one.

fn local_shadowing(prog: &QfrProgram) -> QfrProgram {
    let mut out = prog.clone();
    out.code.clear();

    let n = prog.code.len();
    if n == 0 {
        return prog.clone();
    }

    // Find block leaders (same logic as build_cfg)
    let mut is_leader = vec![false; n];
    for entry in &prog.entries {
        let off = entry.code_offset as usize;
        if off < n {
            is_leader[off] = true;
        }
    }
    if !prog.entries.is_empty() {
        is_leader[0] = true;
    }
    for i in 0..n {
        let op = prog.code[i].opcode();
        if is_terminator(op) && i + 1 < n {
            is_leader[i + 1] = true;
        }
    }

    // Build blocks from leaders
    let leaders: Vec<usize> = is_leader
        .iter()
        .enumerate()
        .filter(|(_, &l)| l)
        .map(|(i, _)| i)
        .collect();

    let mut blocks: Vec<(usize, usize)> = Vec::new();
    for w in leaders.windows(2) {
        if w[0] < w[1] {
            blocks.push((w[0], w[1]));
        }
    }
    if let Some(&last) = leaders.last() {
        if last < n {
            blocks.push((last, n));
        }
    }

    // Process each basic block independently
    for (start, end) in blocks {
        let mut slot_to_reg: HashMap<u32, u8> = HashMap::new();
        let mut dirty: std::collections::HashSet<u32> = std::collections::HashSet::new();
        let block_instrs: Vec<Instruction> = prog.code[start..end].to_vec();

        for instr in &block_instrs {
            let op = instr.opcode();
            match op {
                O::PersistGet => {
                    let slot = instr.imm();
                    let rd = instr.rd();
                    if let Some(&cached_reg) = slot_to_reg.get(&slot) {
                        out.code.push(Instruction::rr(O::Mov, rd, cached_reg));
                    } else {
                        slot_to_reg.insert(slot, rd);
                        out.code.push(*instr);
                    }
                }
                O::PersistSet => {
                    let slot = instr.imm();
                    let rs = instr.rd();
                    slot_to_reg.insert(slot, rs);
                    dirty.insert(slot);
                }
                O::Jmp | O::Jz | O::Jnz | O::Call | O::Ret | O::Halt => {
                    emit_persistset_coalesced(&mut out, &slot_to_reg, &dirty);
                    dirty.clear();
                    out.code.push(*instr);
                }
                _ => {
                    let rd = instr.rd();
                    slot_to_reg.retain(|_, &mut r| r != rd);
                    out.code.push(*instr);
                }
            }
        }
        // Emit any remaining coalesced sets at block end
        emit_persistset_coalesced(&mut out, &slot_to_reg, &dirty);
        dirty.clear();
    }

    out
}

// Emits PersistSet instructions for all dirty slots at a block boundary.
// Only emits the final value for each slot (coalescing redundant writes).
fn emit_persistset_coalesced(
    out: &mut QfrProgram,
    slot_to_reg: &HashMap<u32, u8>,
    dirty: &std::collections::HashSet<u32>,
) {
    for &slot in dirty {
        if let Some(&reg) = slot_to_reg.get(&slot) {
            out.code.push(Instruction::rri(O::PersistSet, reg, 0, slot));
        }
    }
}

// --- Section: LICM (Loop-Invariant Code Motion) ---
// Hoists loop-invariant instructions out of loop bodies into a pre-header block.
// Uses dominator-based natural loop detection: a back edge (v→h) where h dominates v.
// An instruction is invariant if all source registers are defined outside the loop
// or defined by other invariant instructions within the loop.

fn licm(prog: &QfrProgram) -> QfrProgram {
    let n = prog.code.len();
    if n == 0 {
        return prog.clone();
    }

    // Build CFG
    let cfg = build_cfg(&prog.code, &prog.entries);

    // For each block, find its immediate dominator using the simple iterative algorithm
    let dom = compute_dominators(&cfg);

    // Find natural loops via back edges: edge (latch → header) where header dominates latch.
    // A natural loop = header + all blocks reachable from latch without passing through header.
    let mut loops: Vec<(usize, Vec<usize>)> = Vec::new();
    for block_id in 0..cfg.blocks.len() {
        for &succ in &cfg.blocks[block_id].succ {
            if succ < dom.len() && dom[block_id].contains(&succ) {
                // Back edge found: block_id is latch, succ is loop header
                // Collect loop body by walking backwards from latch to header
                let mut body: std::collections::HashSet<usize> = std::collections::HashSet::new();
                body.insert(succ); // header is always in the body
                let mut worklist = vec![block_id];
                while let Some(bid) = worklist.pop() {
                    if body.insert(bid) {
                        for &p in &cfg.blocks[bid].pred {
                            if !body.contains(&p) {
                                worklist.push(p);
                            }
                        }
                    }
                }
                loops.push((succ, body.into_iter().collect()));
            }
        }
    }

    if loops.is_empty() {
        return prog.clone();
    }

    let mut out = prog.clone();
    // LICM needs to modify code — for simplicity, we identify loop-invariant
    // instructions and insert them before the loop header with a new pre-header.
    // Given the complexity of rewriting offsets, for now we do a simpler approach:
    // identify and mark invariants, then hoist.
    // Full implementation would need to:
    //   1. Create pre-header block
    //   2. Move invariants there
    //   3. Fix all jump offsets
    // For this pass, we do it on a cloned program.

    // For each loop, find instructions whose operands are all defined outside the loop
    for (header, body) in &loops {
        let body_set: std::collections::HashSet<usize> = body.iter().copied().collect();
        let header_block = &cfg.blocks[*header];
        let header_start = header_block.start;

        // Find loop-invariant instructions in the body
        // An instruction is invariant if all source registers are:
        //   (a) defined before the loop, or
        //   (b) defined by an invariant instruction within the loop

        // First, find all registers defined before the loop
        let mut defined_before: std::collections::HashSet<u8> = std::collections::HashSet::new();
        for i in 0..header_start {
            let op = out.code[i].opcode();
            if !matches!(
                op,
                O::Jmp | O::Jz | O::Jnz | O::Ret | O::Halt | O::Call | O::SendOrder
            ) {
                defined_before.insert(out.code[i].rd());
            }
        }

        // Iteratively find invariants
        let mut invariant_instrs: Vec<usize> = Vec::new();
        let mut invariant_defs: std::collections::HashSet<u8> = std::collections::HashSet::new();
        let mut changed = true;
        while changed {
            changed = false;
            for &pc in &body_set {
                if invariant_instrs.contains(&pc) {
                    continue;
                }
                let instr = out.code[pc];
                let op = instr.opcode();
                if is_terminator(op)
                    || matches!(
                        op,
                        O::PersistGet | O::PersistSet | O::SendOrder | O::Log | O::Log2
                    )
                {
                    continue; // side-effecting or control flow
                }
                if matches!(
                    op,
                    O::GetInd
                        | O::GetPrice
                        | O::GetPos
                        | O::GetBal
                        | O::GetDepthBid
                        | O::GetDepthAsk
                ) {
                    continue; // engine queries — not invariant
                }
                let rs1 = instr.rs1();
                let _rs2 = instr.rs2();
                let enc = instr.opcode().encoding();
                let rs1_inv = rs1 == 0 && enc == InstrEncoding::RI
                    || defined_before.contains(&rs1)
                    || invariant_defs.contains(&rs1);
                let rs2_inv = match enc {
                    InstrEncoding::RRR | InstrEncoding::RRI | InstrEncoding::RR => {
                        let rs2_val = if matches!(enc, InstrEncoding::RR) {
                            0
                        } else {
                            instr.rs2()
                        };
                        rs2_val == 0
                            || defined_before.contains(&rs2_val)
                            || invariant_defs.contains(&rs2_val)
                    }
                    _ => true,
                };
                if rs1_inv && rs2_inv {
                    let rd = instr.rd();
                    if !matches!(op, O::Jmp | O::Jz | O::Jnz | O::Ret | O::Halt) {
                        invariant_instrs.push(pc);
                        invariant_defs.insert(rd);
                        changed = true;
                    }
                }
            }
        }

        // Hoist invariants to pre-header (before the loop header)
        if !invariant_instrs.is_empty() {
            let mut new_code: Vec<Instruction> =
                Vec::with_capacity(out.code.len() + invariant_instrs.len());
            // Copy instructions before the loop
            for i in 0..header_start {
                new_code.push(out.code[i]);
            }
            // Insert invariants
            for &pc in &invariant_instrs {
                new_code.push(out.code[pc]);
            }
            // Copy loop + after, skipping invariant instructions
            let inv_set: std::collections::HashSet<usize> =
                invariant_instrs.iter().copied().collect();
            for i in header_start..out.code.len() {
                if inv_set.contains(&i) {
                    continue;
                }
                let instr = out.code[i];
                // Note: jump offset adjustment needed for full LICM correctness.
                // For the initial implementation, offsets should be recalculated
                // by a later pass (cfg_simplify + dce).
                new_code.push(instr);
            }
            // Replace code with hoisted version
            let hoisted = !invariant_instrs.is_empty();
            out.code = new_code;
            if hoisted {
                // Adjust jump offsets that shifted due to hoisting.
                // Each instruction's old position = new position + number of
                // hoisted instructions that were before it in the old code.
                fn old_pos(new_pos: usize, hoisted: &[usize]) -> usize {
                    let mut pos = new_pos as i64;
                    loop {
                        let before = hoisted.iter().filter(|&&h| (h as i64) < pos).count() as i64;
                        let next = new_pos as i64 + before;
                        if next == pos {
                            return pos as usize;
                        }
                        pos = next;
                    }
                }
                for i in 0..out.code.len() {
                    let op = out.code[i].opcode();
                    if matches!(op, O::Jmp | O::Jz | O::Jnz | O::Call) {
                        let old_instr = out.code[i];
                        let old_imm = old_instr.imm_signed() as i64;
                        let old_j = old_pos(i, &invariant_instrs) as i64;
                        let old_target = old_j + 1 + old_imm;
                        let h_before_target = invariant_instrs
                            .iter()
                            .filter(|&&h| (h as i64) < old_target)
                            .count() as i64;
                        let new_target = old_target - h_before_target;
                        let new_imm = (new_target - i as i64 - 1) as i32;
                        out.code[i] =
                            Instruction::rri(op, old_instr.rd(), old_instr.rs1(), new_imm as u32);
                    }
                }
            }
        }
    }

    out
}

// --- Section: Dominator Computation ---
// Simple iterative dataflow: dom(b) = {b} ∪ (∩ dom(p) for p in preds(b))
// Converges in O(n²) iterations for reducible flow graphs.
// Returns a list per block of all blocks that dominate it (including itself).

fn compute_dominators(cfg: &Cfg) -> Vec<Vec<usize>> {
    let n_blocks = cfg.blocks.len();
    if n_blocks == 0 {
        return vec![];
    }

    let mut dom: Vec<Vec<usize>> = vec![vec![]; n_blocks];

    // Init: all blocks dominate themselves
    let all_blocks: Vec<usize> = (0..n_blocks).collect();

    // Entry block dominates only itself
    for d in dom.iter_mut() {
        *d = all_blocks.clone();
        d.sort();
    }

    // Entry blocks: only themselves
    for &eid in &cfg.entry_ids {
        let mut d = vec![eid];
        d.sort();
        dom[eid] = d;
    }

    let mut changed = true;
    while changed {
        changed = false;
        for bid in 0..n_blocks {
            if cfg.entry_ids.contains(&bid) {
                continue;
            }
            let preds = &cfg.blocks[bid].pred;
            if preds.is_empty() {
                continue;
            }

            // NewDom = {bid} ∪ (∩ dom(p) for p in preds)
            let mut new_dom: Vec<usize> = dom[preds[0]].clone();
            for &p in &preds[1..] {
                new_dom.retain(|x| dom[p].contains(x));
            }
            if !new_dom.contains(&bid) {
                new_dom.push(bid);
            }
            new_dom.sort();

            if new_dom != dom[bid] {
                dom[bid] = new_dom;
                changed = true;
            }
        }
    }

    dom
}

// --- Section: Loop Unrolling (loop_unroll) ---
// Unrolls loops with known small constant iteration counts.
// Detects single-block loops with a counter that is incremented by ±1
// each iteration and compared against a constant bound.

fn loop_unroll(prog: &QfrProgram) -> QfrProgram {
    const MAX_UNROLL: usize = 8;
    let n = prog.code.len();
    if n == 0 {
        return prog.clone();
    }

    let cfg = build_cfg(&prog.code, &prog.entries);
    let dom = compute_dominators(&cfg);

    let mut loop_bodies: Vec<(usize, usize, Vec<usize>)> = Vec::new();
    for block_id in 0..cfg.blocks.len() {
        for &succ in &cfg.blocks[block_id].succ {
            if succ < dom.len() && dom[block_id].contains(&succ) {
                let mut body: Vec<usize> = Vec::new();
                let mut worklist = vec![block_id];
                let mut visited = std::collections::HashSet::new();
                visited.insert(succ);
                while let Some(bid) = worklist.pop() {
                    if visited.insert(bid) {
                        body.push(bid);
                        for &p in &cfg.blocks[bid].pred {
                            if !visited.contains(&p) {
                                worklist.push(p);
                            }
                        }
                    }
                }
                body.sort();
                loop_bodies.push((succ, block_id, body));
            }
        }
    }

    if loop_bodies.is_empty() {
        return prog.clone();
    }

    // ── Simple loop unrolling ──
    // Handle single-block loops where the counter is a register
    // initialized with a constant before the loop, incremented by ±1
    // each iteration, and compared against a constant bound.
    //
    // Pattern after constant_fold:
    //   Ldi r_cnt, init       (pre-header)
    //   Ldi r_bound, bound    (pre-header or within block)
    //   Gt r_tmp, r_cnt, r_bound
    //   Jz r_tmp, exit
    //   ...body (no internal control flow)...
    //   AddI r_cnt, r_cnt, step (±1)
    //   Jmp header

    let mut out = prog.clone();
    out.code.clear();

    let mut unrolled_any = false;
    let mut skip_instrs: Vec<bool> = vec![false; n];

    for (header, _latch, body_blocks) in &loop_bodies {
        if body_blocks.len() != 1 {
            continue;
        }
        let bid = *header;
        let b = &cfg.blocks[bid];
        let start = b.start;
        let end = b.end;
        if end - start < 4 {
            continue;
        } // need at least Gt+Jz+body+Jmp

        // Find the loop pattern
        let mut found_gt = false;
        let mut found_jz = false;
        let mut found_addi = false;
        let mut found_jmp = false;
        let mut cnt_reg = 0u8;
        let mut bound_reg = 0u8;
        let mut step = 0i64;
        let mut body_start = 0usize;
        let mut body_end = 0usize;

        for i in start..end {
            let op = prog.code[i].opcode();
            let ri = &prog.code[i];
            match op {
                O::Gt if i + 1 < end => {
                    let next = prog.code[i + 1].opcode();
                    if next == O::Jz || next == O::Jnz {
                        let jz_op = next;
                        if jz_op == O::Jz {
                            let jz_rs = prog.code[i + 1].rs1();
                            if jz_rs == ri.rd() {
                                found_gt = true;
                                found_jz = true;
                                cnt_reg = ri.rs1();
                                bound_reg = ri.rs2();
                                body_start = i + 2;
                            }
                        }
                    }
                }
                O::AddI if ri.rd() == ri.rs1() => {
                    // rX = rX + imm
                    let imm = ri.imm_signed() as i64;
                    if imm == 1 || imm == -1 {
                        found_addi = true;
                        step = imm;
                    }
                }
                _ => {}
            }
            if found_addi && i + 1 < end && prog.code[i + 1].opcode() == O::Jmp {
                let jmp_target = compute_target(&prog.code, i + 1);
                if jmp_target == Some(start) {
                    found_jmp = true;
                    body_end = i; // body ends before AddI
                }
            }
        }

        if !(found_gt && found_jz && found_addi && found_jmp) {
            continue;
        }
        // Verify body has no control flow
        let mut has_internal_cf = false;
        for i in body_start..body_end {
            let op = prog.code[i].opcode();
            if is_terminator(op) || op == O::Call || op == O::SendOrder {
                has_internal_cf = true;
                break;
            }
        }
        if has_internal_cf {
            continue;
        }

        // Trace counter and bound to constant Ldi
        let init_val = trace_reg_value(&prog.code, start, cnt_reg, 0);
        let bound_val = trace_reg_value(&prog.code, start, bound_reg, 0);
        let (init, bound) = match (init_val, bound_val) {
            (Some(a), Some(b)) if step > 0 => (a, b),
            (Some(a), Some(b)) if step < 0 => (b, a),
            _ => continue,
        };
        if step == 0 {
            continue;
        }
        let iter_count = if bound >= init && step != 0 {
            match (bound.checked_sub(init), step.checked_abs()) {
                (Some(diff), Some(abs_step)) if abs_step > 0 => {
                    let ic = diff / abs_step + 1;
                    if ic > MAX_UNROLL as i64 {
                        MAX_UNROLL + 1
                    } else {
                        ic as usize
                    }
                }
                _ => MAX_UNROLL + 1, // overflow: skip unrolling
            }
        } else {
            0
        };
        if iter_count == 0 || iter_count > MAX_UNROLL {
            continue;
        }

        // Unroll: copy body `iter_count` times
        for _iter in 0..iter_count {
            for i in body_start..body_end {
                let instr = prog.code[i];
                // Adjust internal jumps: add offset of how much was already emitted
                let adjusted = if is_jump(instr.opcode()) {
                    let old_imm = instr.imm_signed() as i64;
                    let target_old = i as i64 + 1 + old_imm;
                    let effective_old = target_old as usize;
                    if effective_old >= body_start && effective_old < body_end {
                        // Internal jump within the body — leave as-is (same relative layout within each copy)
                        instr
                    } else {
                        instr // external jump — leave for later DCE pass
                    }
                } else {
                    instr
                };
                out.code.push(adjusted);
            }
            // Add counter increment (except on last iteration)
            if _iter + 1 < iter_count {
                out.code
                    .push(Instruction::rri(O::AddI, cnt_reg, cnt_reg, step as u32));
            }
        }

        // Mark original loop instructions as skipped
        for s in skip_instrs[start..end].iter_mut() {
            *s = true;
        }
        unrolled_any = true;
    }

    if !unrolled_any {
        return prog.clone();
    }

    // Build output: skip marked instructions, adjust offsets
    let mut old_to_new: Vec<Option<usize>> = vec![None; n];
    for i in 0..n {
        if !skip_instrs[i] {
            old_to_new[i] = Some(out.code.len());
            out.code.push(prog.code[i]);
        }
    }

    // Adjust jump offsets in the final code
    for ni in 0..out.code.len() {
        let op = out.code[ni].opcode();
        if is_jump(op) {
            // Find the old index to compute original target
            let mut old_i = None;
            for (oi, &no) in old_to_new.iter().enumerate() {
                if no == Some(ni) {
                    old_i = Some(oi);
                    break;
                }
            }
            if let Some(oi) = old_i {
                let old_imm = prog.code[oi].imm_signed() as i64;
                let old_target = oi as i64 + 1 + old_imm;
                if old_target >= 0 && (old_target as usize) < n {
                    if let Some(new_target) = old_to_new[old_target as usize] {
                        let new_imm = new_target as i64 - ni as i64 - 1;
                        let rd = out.code[ni].rd();
                        let rs = out.code[ni].rs1();
                        out.code[ni] = Instruction::rri(op, rd, rs, new_imm as u32);
                    }
                }
            }
        }
    }

    // Update entry points
    let mut new_entries = out.entries.clone();
    for entry in &mut new_entries {
        let old_off = entry.code_offset as usize;
        if let Some(new_off) = old_to_new.get(old_off).copied().flatten() {
            entry.code_offset = new_off as u32;
        } else {
            entry.code_offset = 0;
        }
    }
    out.entries = new_entries;

    out
}

// Traces a register back through Mov chains to find its defining Ldi/Ldi64 constant.
// Scans backwards from `start` and returns None if the value cannot be determined
// (e.g., crosses a control-flow boundary, hits a mutation, or exceeds max depth).
fn trace_reg_value(code: &[Instruction], start: usize, reg: u8, depth: usize) -> Option<i64> {
    if depth > 16 {
        return None;
    }
    let mut i = start as isize - 1;
    while i >= 0 {
        let idx = i as usize;
        let instr = &code[idx];
        let op = instr.opcode();
        if instr.rd() == reg {
            return match op {
                O::Ldi => Some(instr.imm_signed() as i64),
                O::Ldi64 => Some(instr.imm40()),
                O::Mov => trace_reg_value(code, idx, instr.rs1(), depth + 1),
                O::AddI if instr.rs1() == reg => {
                    // rX += imm — not a definition but a mutation, stop
                    None
                }
                _ => None,
            };
        }
        if is_terminator(op) {
            return None;
        }
        i -= 1;
    }
    None
}

// Returns true if op is a jump or call instruction (has a branch target).
fn is_jump(op: O) -> bool {
    matches!(op, O::Jmp | O::Jz | O::Jnz | O::Call)
}

// --- Section: Fused Opcode Lowering (fused_lowering) ---
// Pattern-matches multi-instruction sequences and replaces with fused opcodes.

fn fused_lowering(prog: &QfrProgram) -> QfrProgram {
    let n = prog.code.len();
    if n < 2 {
        return prog.clone();
    }

    let mut out = prog.clone();
    out.code.clear();
    let mut i = 0;

    while i < n {
        let instr = prog.code[i];
        let op = instr.opcode();

        // Pattern 1: FMA (fused multiply-add):
        //   FMul rX, rA, rB   ; rd, rs1, rs2
        //   FAdd rD, rX, rC   ; rd=rD, rs1=rX, rs2=rC
        // → need new opcode
        // For now, we skip as it requires new VM opcode + handlers

        // Pattern 2: Compare + Branch:
        //   Lt rTmp, rA, rB   ; rd=rTmp
        //   Jnz rTmp, target
        // → Can't easily fuse without new opcodes + VM changes

        // For now, we focus on simple peephole patterns in the existing instruction set

        // Pattern: Consecutive WindowPush + WindowMean → no fusion needed,
        // these already exist as separate opcodes.

        // Pattern: redundant Mov chains
        // Mov rA, rB; Mov rB, rC → Mov rA, rC
        if i + 1 < n && op == O::Mov {
            let next = prog.code[i + 1];
            if next.opcode() == O::Mov {
                let r1 = instr.rd();
                let _s1 = instr.rs1();
                let r2 = next.rd();
                let s2 = next.rs1();
                if r1 == s2 {
                    // Mov rA, rB; Mov rB, rC → NOP (first), Mov rB, rC → Mov rA, rC
                    // Actually: Mov rA, rB; Mov rB, rC
                    // After: Mov rA, rC (skip the middle)
                    out.code.push(Instruction::rr(O::Mov, r1, s2));
                    out.code.push(Instruction::rr(O::Mov, r2, s2));
                    i += 2;
                    continue;
                }
            }
        }

        // Pattern: Ldi rX, 0; AddI rY, rX, imm → Ldi rY, imm (when rX is only used once)
        if i + 1 < n && op == O::Ldi {
            let rd = instr.rd();
            let val = instr.imm_signed();
            if val == 0 {
                let next = prog.code[i + 1];
                let next_op = next.opcode();
                if (next_op == O::AddI
                    || next_op == O::SubI
                    || next_op == O::MulI
                    || next_op == O::DivI)
                    && next.rs1() == rd && next.rd() != rd
                {
                    // Ldi r0, 0; AddI r1, r0, 5 → Ldi r1, 5
                    match next_op {
                        O::AddI => {
                            out.code
                                .push(Instruction::rri(O::Ldi, next.rd(), 0, next.imm()));
                            i += 2;
                            continue;
                        }
                        O::SubI => {
                            out.code.push(Instruction::rri(
                                O::Ldi,
                                next.rd(),
                                0,
                                (-(next.imm_signed() as i64)) as u32,
                            ));
                            i += 2;
                            continue;
                        }
                        O::MulI => {
                            /* 0 * imm = 0 */
                            out.code.push(Instruction::rri(O::Ldi, next.rd(), 0, 0));
                            i += 2;
                            continue;
                        }
                        O::DivI => {
                            /* 0 / imm = 0 */
                            out.code.push(Instruction::rri(O::Ldi, next.rd(), 0, 0));
                            i += 2;
                            continue;
                        }
                        _ => {}
                    }
                }
            }
        }

        out.code.push(instr);
        i += 1;
    }

    out
}

// --- Section: GVN (Global Value Numbering) ---
// Extends CSE across basic blocks using a dominator-tree-based value numbering.
// Walks blocks in linear order, but inherits the CSE cache from the immediate
// dominator rather than clearing it at block boundaries.

// Global Value Numbering: eliminates redundant computations across basic blocks.
// Walks blocks in linear order, inheriting the expression cache from the
// immediate dominator rather than clearing at block boundaries (unlike CSE).
fn global_value_numbering(prog: &QfrProgram) -> QfrProgram {
    let n = prog.code.len();
    if n == 0 {
        return prog.clone();
    }

    // Build CFG and compute dominator tree
    let cfg = build_cfg(&prog.code, &prog.entries);
    let dom = compute_dominators(&cfg);
    let n_blocks = cfg.blocks.len();
    if n_blocks == 0 {
        return prog.clone();
    }

    // Assign value numbers to expressions using hash consing.
    // We walk blocks and track which value numbers are available at each point.
    // The key insight: the CSE cache propagates from immediate dominator to
    // its children, so a computation in block A is available in block B if
    // A dominates B.

    let mut out = prog.clone();
    out.code.clear();

    // Map each instruction index to its containing block ID
    let mut idx_to_block = vec![usize::MAX; n];
    for (bid, b) in cfg.blocks.iter().enumerate() {
        for idx in idx_to_block[b.start..b.end].iter_mut() {
            *idx = bid;
        }
    }

    // Cache: (opcode as u8, rs1, operand2) → original instruction index
    type Cache = HashMap<(u8, u8, u32), usize>;

    let mut instr_map: Vec<Option<usize>> = vec![None; n];
    let idom = compute_idom(&dom, &cfg.entry_ids);
    let mut block_caches: Vec<Option<Cache>> = vec![None; n_blocks];

    // Process blocks in linear order (which respects dominator tree order)
    for bid in 0..n_blocks {
        // Start with the cache from our immediate dominator (or empty for entry blocks)
        let parent_cache = if cfg.entry_ids.contains(&bid) {
            Cache::new() // entry blocks start fresh
        } else if let Some(&p) = idom.get(&bid) {
            block_caches[p].as_ref().cloned().unwrap_or_default()
        } else {
            Cache::new()
        };

        let block = &cfg.blocks[bid];
        let mut cache = parent_cache;

        // Walk instructions in the block
        #[allow(clippy::needless_range_loop)]
        for i in block.start..block.end {
            let instr = prog.code[i];
            let op = instr.opcode();

            // Control flow resets cache (basic block boundary within dominator scope)
            if is_control_flow(op) {
                cache.clear();
                out.code.push(instr);
                instr_map[i] = Some(out.code.len() - 1);
                continue;
            }

            // Invalidate cache entries that depend on the destination register
            let rd = instr.rd();
            cache.retain(|_, &mut v| {
                let ci = prog.code[v];
                ci.rd() != rd && ci.rs1() != rd && ci.rs2() != rd
            });

            // Check if this opcode is eligible for CSE
            let is_cse_candidate = matches!(
                op,
                O::Add
                    | O::Sub
                    | O::Mul
                    | O::Div
                    | O::Mod
                    | O::FAdd
                    | O::FSub
                    | O::FMul
                    | O::FDiv
                    | O::Eq
                    | O::Ne
                    | O::Lt
                    | O::Gt
                    | O::Le
                    | O::Ge
                    | O::FEq
                    | O::FNe
                    | O::FLt
                    | O::FGt
                    | O::FLe
                    | O::FGe
                    | O::BitAnd
                    | O::BitOr
                    | O::BitXor
                    | O::Shl
                    | O::Shr
                    | O::AddI
                    | O::SubI
                    | O::MulI
                    | O::DivI
                    | O::EqI
                    | O::LtI
                    | O::GtI
                    | O::Neg
                    | O::FNeg
                    | O::BitNot
            );

            if is_cse_candidate {
                // Build key from (opcode, rs1, operand2) where operand2 is rs2 or imm
                let enc = instr.opcode().encoding();
                let (rs1, op2) = match enc {
                    InstrEncoding::RRR => (instr.rs1(), instr.rs2() as u32),
                    InstrEncoding::RRI => (instr.rs1(), instr.imm()),
                    InstrEncoding::RR => (instr.rs1(), 0),
                    _ => (0, 0),
                };
                let key = (op as u8, rs1, op2);
                // If we've seen this computation before and its block dominates the current one,
                // replace with Mov from the earlier result
                if let Some(&orig_idx) = cache.get(&key) {
                    if orig_idx < n && idx_to_block[orig_idx] < n_blocks {
                        let orig_block = idx_to_block[orig_idx];
                        // Only reuse if orig_block dominates bid (or it's the same block)
                        if dom[bid].contains(&orig_block) || orig_block == bid {
                            let orig_instr = prog.code[orig_idx];
                            out.code.push(Instruction::rr(O::Mov, rd, orig_instr.rd()));
                            instr_map[i] = Some(out.code.len() - 1);
                            cache.insert(key, i); // update cache to point to this later occurrence
                            continue;
                        }
                    }
                }
                cache.insert(key, i);
            }

            out.code.push(instr);
            instr_map[i] = Some(out.code.len() - 1);
        }

        block_caches[bid] = Some(cache);
    }

    // Adjust jump offsets after code restructuring
    let offset_map: Vec<Option<usize>> = instr_map;
    let new_code = &mut out.code;
    #[allow(clippy::needless_range_loop)]
    for i in 0..new_code.len() {
        let op = new_code[i].opcode();
        if matches!(op, O::Jmp | O::Jz | O::Jnz | O::Call) {
            let old_target = i as i64 + 1 + new_code[i].imm_signed() as i64;
            let new_target = if old_target >= 0 && (old_target as usize) < n {
                offset_map
                    .get(old_target as usize)
                    .copied()
                    .flatten()
                    .unwrap_or(old_target as usize)
            } else {
                old_target as usize
            };
            let new_imm = (new_target as i64 - i as i64 - 1) as i32;
            let old_instr = new_code[i];
            new_code[i] = Instruction::rri(op, old_instr.rd(), old_instr.rs1(), new_imm as u32);
        }
    }

    out
}

// Computes the immediate dominator for each block from the full dominator set.
// The immediate dominator is the unique strict dominator closest to the block.
// Uses the property: idom(b) = the dominator of b that is dominated by all other dominators.
fn compute_idom(
    dom: &[Vec<usize>],
    entry_ids: &[usize],
) -> std::collections::HashMap<usize, usize> {
    let mut idom = std::collections::HashMap::new();
    for (bid, dlist) in dom.iter().enumerate() {
        if entry_ids.contains(&bid) {
            continue;
        }
        // The immediate dominator is the dominator that is dominated by all other dominators
        // (excl. self). In the dominance frontier, it's the one closest to bid.
        // For our simple dom list, the immediate dominator is the entry in dlist
        // that appears right before bid (the second-to-last if bid is last, etc.)
        if dlist.len() >= 2 {
            // The immediate dominator is the one closest to bid (largest in dom order doesn't work)
            // In the iterative algorithm, idom = the predecessor that dominates all others
            // We approximate: idom = the one in dlist that is != bid and closest
            for &d in dlist {
                if d != bid {
                    // Check if d dominates all other dominators of bid except bid itself
                    let _dominates_all = dlist
                        .iter()
                        .all(|&x| x == bid || x == d || dom[d].contains(&x));
                    if dlist.len() == 2 {
                        idom.insert(bid, d);
                        break;
                    }
                }
            }
        }
    }
    // For blocks not found, use entry
    for (bid, dlist) in dom.iter().enumerate() {
        if !idom.contains_key(&bid) && !entry_ids.contains(&bid) && dlist.len() >= 2 {
            idom.insert(bid, dlist[0]);
        }
    }
    idom
}

// Returns true if op is a control-flow instruction (jump, call, ret, halt, send).
fn is_control_flow(op: O) -> bool {
    matches!(
        op,
        O::Jmp | O::Jz | O::Jnz | O::Call | O::Ret | O::SendOrder | O::Halt
    )
}

// Returns true if op terminates a basic block (unconditional/conditional jump, ret, halt).
fn is_terminator(op: O) -> bool {
    matches!(op, O::Jmp | O::Jz | O::Jnz | O::Ret | O::Halt)
}

// --- Section: Constant Folding Helper Functions ---

// Folds an integer RRR instruction when both operand registers are known constants.
// Computes the result with wrapping arithmetic and emits an Ldi or equivalent.
fn fold_int_rrr(out: &mut QfrProgram, known: &mut HashMap<u8, i64>, instr: &Instruction, op: O) {
    let rd = instr.rd();
    let rs1 = instr.rs1();
    let rs2 = instr.rs2();
    if let (Some(&a), Some(&b)) = (known.get(&rs1), known.get(&rs2)) {
        let result = match op {
            O::Add => a.wrapping_add(b),
            O::Sub => a.wrapping_sub(b),
            O::Mul => a.wrapping_mul(b),
            O::Div => {
                if b == 0 {
                    0
                } else {
                    a.wrapping_div(b)
                }
            }
            O::Mod => {
                if b == 0 {
                    0
                } else {
                    a.wrapping_rem(b)
                }
            }
            O::BitAnd => a & b,
            O::BitOr => a | b,
            O::BitXor => a ^ b,
            O::Shl => a.wrapping_shl(b as u32),
            O::Shr => a.wrapping_shr(b as u32),
            O::Eq => (a == b) as i64,
            O::Ne => (a != b) as i64,
            O::Lt => (a < b) as i64,
            O::Gt => (a > b) as i64,
            O::Le => (a <= b) as i64,
            O::Ge => (a >= b) as i64,
            _ => 0,
        };
        known.insert(rd, result);
        emit_ldi_value(out, rd, result);
    } else {
        known.remove(&rd);
        out.code.push(*instr);
    }
}

// Folds a float RRR instruction when both operand registers are known constants.
// Emits a LdcF64 referencing the constant pool.
fn fold_float_rrr(out: &mut QfrProgram, known: &mut HashMap<u8, f64>, instr: &Instruction, op: O) {
    let rd = instr.rd();
    let rs1 = instr.rs1();
    let rs2 = instr.rs2();
    if let (Some(&a), Some(&b)) = (known.get(&rs1), known.get(&rs2)) {
        let result = match op {
            O::FAdd => a + b,
            O::FSub => a - b,
            O::FMul => a * b,
            O::FDiv => {
                if b == 0.0 {
                    0.0
                } else {
                    a / b
                }
            }
            O::FEq => f64::from((a - b).abs() < f64::EPSILON),
            O::FNe => f64::from((a - b).abs() >= f64::EPSILON),
            O::FLt => f64::from(a < b),
            O::FGt => f64::from(a > b),
            O::FLe => f64::from(a <= b),
            O::FGe => f64::from(a >= b),
            _ => 0.0,
        };
        known.insert(rd, result);
        let idx = out.intern_f64(result);
        out.code.push(Instruction::rri(O::LdcF64, rd, 0, idx));
    } else {
        known.remove(&rd);
        out.code.push(*instr);
    }
}

// Folds an integer RRI instruction (e.g., AddI, SubI) when rs1 is a known constant.
// Applies the given binary function `f` to the known value and the immediate.
fn fold_int_rri(
    out: &mut QfrProgram,
    known: &mut HashMap<u8, i64>,
    instr: &Instruction,
    f: fn(i64, u32) -> i64,
    _orig: O,
) {
    let rd = instr.rd();
    let rs1 = instr.rs1();
    let imm = instr.imm();
    if let Some(&a) = known.get(&rs1) {
        let result = f(a, imm);
        known.insert(rd, result);
        emit_ldi_value(out, rd, result);
    } else {
        known.remove(&rd);
        out.code.push(*instr);
    }
}

// Folds an integer RRI comparison instruction (EqI, LtI, GtI)
// when rs1 is a known constant. Emits Ldi with 0 or 1 as the result.
fn fold_int_rri_bool(
    out: &mut QfrProgram,
    known: &mut HashMap<u8, i64>,
    instr: &Instruction,
    f: fn(i64, u32) -> bool,
) {
    let rd = instr.rd();
    let rs1 = instr.rs1();
    let imm = instr.imm();
    if let Some(&a) = known.get(&rs1) {
        let result = if f(a, imm) { 1 } else { 0 };
        known.insert(rd, result);
        out.code
            .push(Instruction::rri(O::Ldi, rd, 0, result as u32));
    } else {
        known.remove(&rd);
        out.code.push(*instr);
    }
}

// --- Section: CFG Building & Simplification ---

// A basic block: contiguous range of instructions with single-entry, single-exit
// (except for the terminator which may branch to multiple successors).
#[derive(Debug, Clone)]
struct Block {
    start: usize,     // first instruction index (inclusive)
    end: usize,       // last instruction index (exclusive)
    succ: Vec<usize>, // successor block IDs
    pred: Vec<usize>, // predecessor block IDs
}

// Control Flow Graph: a set of basic blocks with entry points.
#[derive(Debug)]
struct Cfg {
    blocks: Vec<Block>,
    entry_ids: Vec<usize>,
    // Instruction indices removed during CFG simplification
    // (e.g., Jmps eliminated by block merging)
    removed_instrs: Vec<usize>,
}

// Computes the absolute target PC of a jump/call instruction at `pc`.
fn compute_target(code: &[Instruction], pc: usize) -> Option<usize> {
    let op = code[pc].opcode();
    match op {
        O::Jmp => {
            let imm = code[pc].imm_signed() as i64;
            let t = pc as i64 + 1 + imm;
            if t >= 0 && (t as usize) < code.len() {
                Some(t as usize)
            } else {
                None
            }
        }
        O::Jz | O::Jnz | O::Call => {
            let imm = code[pc].imm_signed() as i64;
            let t = pc as i64 + 1 + imm;
            if t >= 0 && (t as usize) < code.len() {
                Some(t as usize)
            } else {
                None
            }
        }
        _ => None,
    }
}

// Builds a Control Flow Graph from linear instruction code.
// Identifies basic blocks by finding leaders (entry points, jump targets, fall-through from terminators).
fn build_cfg(code: &[Instruction], entries: &[crate::ir::EntryPoint]) -> Cfg {
    let n = code.len();
    if n == 0 {
        return Cfg {
            blocks: vec![],
            entry_ids: vec![],
            removed_instrs: vec![],
        };
    }

    // Find all block leaders: instructions that start a new basic block
    let mut is_leader = vec![false; n];

    // Entry points are leaders
    for entry in entries {
        let off = entry.code_offset as usize;
        if off < n {
            is_leader[off] = true;
        }
    }

    // Instruction 0 is always a leader (if any entries exist)
    if !entries.is_empty() {
        is_leader[0] = true;
    }

    // Find leaders from jump targets and fall-through
    for i in 0..n {
        let op = code[i].opcode();
        if is_terminator(op) {
            // Fall-through: next instruction is a leader
            if i + 1 < n {
                is_leader[i + 1] = true;
            }
            // Jump target is a leader
            if let Some(target) = compute_target(code, i) {
                is_leader[target] = true;
            }
        }
    }

    // Build blocks from leaders
    let leader_positions: Vec<usize> = is_leader
        .iter()
        .enumerate()
        .filter(|(_, &l)| l)
        .map(|(i, _)| i)
        .collect();

    let mut blocks: Vec<Block> = Vec::new();
    for w in leader_positions.windows(2) {
        let start = w[0];
        let end = w[1];
        if start < end {
            blocks.push(Block {
                start,
                end,
                succ: vec![],
                pred: vec![],
            });
        }
    }
    // Last block (from last leader to end)
    if let Some(&last) = leader_positions.last() {
        if last < n {
            blocks.push(Block {
                start: last,
                end: n,
                succ: vec![],
                pred: vec![],
            });
        }
    }

    if blocks.is_empty() {
        return Cfg {
            blocks: vec![],
            entry_ids: vec![],
            removed_instrs: vec![],
        };
    }

    // Map instruction index to block ID
    let mut idx_to_block = vec![usize::MAX; n];
    for (bid, b) in blocks.iter().enumerate() {
        for idx in idx_to_block[b.start..b.end].iter_mut() {
            *idx = bid;
        }
    }

    // Add edges
    for bid in 0..blocks.len() {
        let b = &blocks[bid];
        let last_pc = b.end - 1;
        let op = code[last_pc].opcode();

        match op {
            O::Jmp => {
                if let Some(target) = compute_target(code, last_pc) {
                    let target_bid = idx_to_block[target];
                    if target_bid < blocks.len() {
                        blocks[bid].succ.push(target_bid);
                        blocks[target_bid].pred.push(bid);
                    }
                }
            }
            O::Jz | O::Jnz => {
                // Taken successor
                if let Some(target) = compute_target(code, last_pc) {
                    let target_bid = idx_to_block[target];
                    if target_bid < blocks.len() {
                        blocks[bid].succ.push(target_bid);
                        blocks[target_bid].pred.push(bid);
                    }
                }
                // Fall-through successor (next block in linear order)
                let next_pc = last_pc + 1;
                if next_pc < n {
                    let target_bid = idx_to_block[next_pc];
                    if target_bid < blocks.len() {
                        blocks[bid].succ.push(target_bid);
                        blocks[target_bid].pred.push(bid);
                    }
                }
            }
            O::Ret | O::Halt => {
                // No successors
            }
            _ => {
                // No terminator at end of block (end of program), or Call (treated as non-terminator)
                // Fall-through to next block if not at end
                if last_pc + 1 < n {
                    let next_bid = idx_to_block[last_pc + 1];
                    if next_bid < blocks.len() {
                        blocks[bid].succ.push(next_bid);
                        blocks[next_bid].pred.push(bid);
                    }
                }
            }
        }
    }

    // Map entries to block IDs
    let mut entry_ids = Vec::new();
    for entry in entries {
        let off = entry.code_offset as usize;
        if off < n {
            let bid = idx_to_block[off];
            if bid < blocks.len() && !entry_ids.contains(&bid) {
                entry_ids.push(bid);
            }
        }
    }
    if entry_ids.is_empty() && !blocks.is_empty() {
        entry_ids.push(0);
    }

    Cfg {
        blocks,
        entry_ids,
        removed_instrs: vec![],
    }
}

// Merges adjacent basic blocks A → B where A has a single successor (B),
// B has a single predecessor (A), and A's terminator is a Jmp to B.
// The Jmp is recorded as removed; A extends to cover B's instruction range.
fn cfg_merge_blocks(cfg: &Cfg, code: &[Instruction]) -> Cfg {
    let mut blocks = cfg.blocks.clone();
    let mut removed = vec![false; blocks.len()];
    let mut removed_instrs: Vec<usize> = Vec::new();

    loop {
        let mut changed = false;
        let mut i = 0;
        while i < blocks.len() {
            if removed[i] {
                i += 1;
                continue;
            }

            let end = blocks[i].end;
            // Find the next non-removed block after i
            let mut j = i + 1;
            while j < blocks.len() && removed[j] {
                j += 1;
            }
            if j >= blocks.len() {
                i += 1;
                continue;
            }
            if blocks[j].start != end {
                i += 1;
                continue;
            }

            // Check merge condition: blocks[i] has only successor j,
            // and j has only predecessor i
            if blocks[i].succ.len() != 1 {
                i += 1;
                continue;
            }
            if blocks[i].succ[0] != j {
                i += 1;
                continue;
            }
            if blocks[j].pred.len() != 1 {
                i += 1;
                continue;
            }
            if blocks[j].pred[0] != i {
                i += 1;
                continue;
            }

            // The terminator of block i must be a Jmp (the only way to have a single successor
            // that goes to an adjacent block)
            let term_pc = blocks[i].end - 1;
            let term_op = code[term_pc].opcode();
            if term_op != O::Jmp {
                i += 1;
                continue;
            }

            // Verify the Jmp goes to block j (should always be true given succ check above)
            if compute_target(code, term_pc) != Some(blocks[j].start) {
                i += 1;
                continue;
            }

            // Merge: extend block i to include block j's range, adopt j's successors
            let j_succ = blocks[j].succ.clone();
            let _j_pred = blocks[j].pred.clone();

            // Update predecessors of j's successors to point to i
            for &s in &j_succ {
                if s < blocks.len() && !removed[s] {
                    for p in blocks[s].pred.iter_mut() {
                        if *p == j {
                            *p = i;
                        }
                    }
                }
            }
            // Remove i from j's pred list (it was the only one since we checked)
            // Update i's succ and pred
            // Record the Jmp instruction that will be removed
            removed_instrs.push(term_pc);

            blocks[i].succ = j_succ;
            blocks[i].end = blocks[j].end;

            removed[j] = true;
            changed = true;
        }
        if !changed {
            break;
        }
    }

    // Rebuild blocks (compact)
    let mut new_blocks: Vec<Block> = Vec::new();
    let mut old_to_new: Vec<Option<usize>> = vec![None; blocks.len()];
    for i in 0..blocks.len() {
        if !removed[i] {
            old_to_new[i] = Some(new_blocks.len());
            new_blocks.push(Block {
                start: blocks[i].start,
                end: blocks[i].end,
                succ: vec![],
                pred: vec![],
            });
        }
    }

    // Remap edges
    for i in 0..blocks.len() {
        if removed[i] {
            continue;
        }
        let new_i = old_to_new[i].unwrap();
        for &s in &blocks[i].succ {
            if let Some(new_s) = old_to_new[s] {
                if !new_blocks[new_i].succ.contains(&new_s) {
                    new_blocks[new_i].succ.push(new_s);
                    if !new_blocks[new_s].pred.contains(&new_i) {
                        new_blocks[new_s].pred.push(new_i);
                    }
                }
            }
        }
    }

    // Remap entry_ids
    let entry_ids: Vec<usize> = cfg
        .entry_ids
        .iter()
        .filter_map(|&e| old_to_new[e])
        .collect();

    Cfg {
        blocks: new_blocks,
        entry_ids,
        removed_instrs,
    }
}

// Removes blocks not reachable from any entry point via the CFG edge graph.
// Uses simple graph traversal (not instruction-level tracing like DCE).
fn cfg_remove_unreachable(cfg: &Cfg) -> Cfg {
    let n_blocks = cfg.blocks.len();
    let mut reachable = vec![false; n_blocks];
    let mut worklist: Vec<usize> = cfg.entry_ids.clone();

    while let Some(bid) = worklist.pop() {
        if reachable[bid] {
            continue;
        }
        reachable[bid] = true;
        for &s in &cfg.blocks[bid].succ {
            if !reachable[s] {
                worklist.push(s);
            }
        }
    }

    let mut new_blocks: Vec<Block> = Vec::new();
    let mut old_to_new: Vec<Option<usize>> = vec![None; n_blocks];
    for i in 0..n_blocks {
        if reachable[i] {
            old_to_new[i] = Some(new_blocks.len());
            new_blocks.push(Block {
                start: cfg.blocks[i].start,
                end: cfg.blocks[i].end,
                succ: vec![],
                pred: vec![],
            });
        }
    }

    // Remap edges
    for i in 0..n_blocks {
        if !reachable[i] {
            continue;
        }
        let new_i = old_to_new[i].unwrap();
        for &s in &cfg.blocks[i].succ {
            if let Some(new_s) = old_to_new[s] {
                if !new_blocks[new_i].succ.contains(&new_s) {
                    new_blocks[new_i].succ.push(new_s);
                    if !new_blocks[new_s].pred.contains(&new_i) {
                        new_blocks[new_s].pred.push(new_i);
                    }
                }
            }
        }
    }

    let entry_ids: Vec<usize> = cfg
        .entry_ids
        .iter()
        .filter_map(|&e| old_to_new[e])
        .collect();

    Cfg {
        blocks: new_blocks,
        entry_ids,
        removed_instrs: cfg.removed_instrs.clone(),
    }
}

// Simplifies jump-to-jump chains: if block A's successor B has only one
// successor (an unconditional Jmp) and B has only A as predecessor,
// redirect A to skip B and point directly to B's target.
fn cfg_simplify_jump_chains(cfg: &Cfg, _code: &[Instruction]) -> Cfg {
    let mut blocks = cfg.blocks.clone();

    for i in 0..blocks.len() {
        let succ = blocks[i].succ.clone();
        for &s in &succ {
            // If block i's successor s has only one successor (a Jmp), and
            // s has only i as predecessor, redirect i to skip s.
            if blocks[s].succ.len() == 1 && blocks[s].pred.len() == 1 && blocks[s].pred[0] == i {
                let grandchild = blocks[s].succ[0];
                if grandchild != s {
                    // avoid self-loop
                    // Update block i's succ: replace s with grandchild
                    if let Some(pos) = blocks[i].succ.iter().position(|&x| x == s) {
                        if !blocks[i].succ.contains(&grandchild) {
                            blocks[i].succ[pos] = grandchild;
                            blocks[grandchild].pred.push(i);
                        }
                    }
                    // Remove i from s's pred
                    if let Some(pos) = blocks[s].pred.iter().position(|&x| x == i) {
                        blocks[s].pred.remove(pos);
                    }
                }
            }
        }
    }

    Cfg {
        blocks,
        entry_ids: cfg.entry_ids.clone(),
        removed_instrs: cfg.removed_instrs.clone(),
    }
}

// Checks if an instruction is a Jmp to the very next instruction (a no-op).
fn is_jmp_to_next(code: &[Instruction], i: usize) -> bool {
    if code[i].opcode() != O::Jmp {
        return false;
    }
    compute_target(code, i) == Some(i + 1)
}

// Emits linear instruction code from a CFG.
// Removes eliminated Jmps, skips removed instructions, and recalculates
// all jump offsets to reflect the new instruction positions.
// Emits linear instruction code from a CFG.
// Removes eliminated Jmps, skips removed instructions, and recalculates
// all jump offsets to reflect the new instruction positions.
fn emit_cfg(cfg: &Cfg, prog: &QfrProgram) -> QfrProgram {
    let code = &prog.code;
    let n_blocks = cfg.blocks.len();

    if n_blocks == 0 {
        let mut out = QfrProgram::new();
        out.entries = prog.entries.clone();
        out.const_pool = prog.const_pool.clone();
        out.const_map = prog.const_map.clone();
        out.ema_alphas = prog.ema_alphas.clone();
        out.f64_consts = prog.f64_consts.clone();
        out.i64_consts = prog.i64_consts.clone();
        out.string_consts = prog.string_consts.clone();
        return out;
    }

    // Pass 1: determine which instructions to keep and their new positions.
    // Skip instructions recorded as removed (e.g., Jmps eliminated by block merging)
    // and Jmp-to-next-instruction (no-ops).
    let mut old_to_new: Vec<Option<usize>> = vec![None; code.len()];
    let mut new_code: Vec<Instruction> = Vec::new();

    let removed_set: std::collections::HashSet<usize> =
        cfg.removed_instrs.iter().copied().collect();

    // Walk blocks and emit body instructions + terminator, skipping removed ones
    for bid in 0..n_blocks {
        let b = &cfg.blocks[bid];
        // Body = all instructions except the last (which is the terminator)
        let body_end = if b.end > b.start { b.end - 1 } else { b.start };
        for i in b.start..body_end {
            if removed_set.contains(&i) {
                continue;
            }
            if is_jmp_to_next(code, i) {
                continue; // skip no-op Jmp to next instruction
            }
            old_to_new[i] = Some(new_code.len());
            new_code.push(code[i]);
        }

        // Emit the terminator if it wasn't removed
        if b.end > b.start {
            let term_pc = b.end - 1;
            if removed_set.contains(&term_pc) {
                continue;
            }
            if is_jmp_to_next(code, term_pc) {
                continue; // skip Jmp-to-next as terminator
            }
            old_to_new[term_pc] = Some(new_code.len());
            new_code.push(code[term_pc]);
        }
    }

    // Pass 2: recalculate jump offsets in new_code
    // Build a reverse index: new position → old position
    let mut new_to_old: Vec<Option<usize>> = vec![None; new_code.len()];
    for (old, new) in old_to_new.iter().enumerate() {
        if let Some(n) = new {
            new_to_old[*n] = Some(old);
        }
    }

    // Recalculate each terminator's relative offset to match the new layout
    for ni in 0..new_code.len() {
        let old_i = new_to_old[ni].unwrap();
        let op = new_code[ni].opcode();
        if !is_terminator(op) {
            continue;
        }

        match op {
            O::Jmp => {
                let old_target = compute_target(code, old_i);
                if let Some(ot) = old_target {
                    if let Some(nt) = old_to_new[ot] {
                        let offset = nt as i64 - ni as i64 - 1;
                        new_code[ni] = Instruction::rri(O::Jmp, 0, 0, offset as u32);
                    }
                }
            }
            O::Jz | O::Jnz => {
                let old_target = compute_target(code, old_i);
                if let Some(ot) = old_target {
                    if let Some(nt) = old_to_new[ot] {
                        let offset = nt as i64 - ni as i64 - 1;
                        let cond_reg = code[old_i].rs1();
                        new_code[ni] = Instruction::rri(op, 0, cond_reg, offset as u32);
                    }
                }
            }
            _ => {}
        }
    }

    // Update entry points
    let mut new_entries = prog.entries.clone();
    for entry in &mut new_entries {
        let old_off = entry.code_offset as usize;
        if let Some(new_off) = old_to_new.get(old_off).copied().flatten() {
            entry.code_offset = new_off as u32;
        } else {
            entry.code_offset = 0;
        }
    }

    let mut out = QfrProgram::new();
    out.entries = new_entries;
    out.const_pool = prog.const_pool.clone();
    out.const_map = prog.const_map.clone();
    out.ema_alphas = prog.ema_alphas.clone();
    out.f64_consts = prog.f64_consts.clone();
    out.i64_consts = prog.i64_consts.clone();
    out.string_consts = prog.string_consts.clone();
    out.code = new_code;
    out
}

// --- Section: CFG Simplification (cfg_simplify) ---

/// CFG Simplification pass.
///
/// Builds a control flow graph, merges consecutive basic blocks,
/// removes unreachable blocks, and simplifies jump chains.
pub fn cfg_simplify(prog: &QfrProgram) -> QfrProgram {
    let n = prog.code.len();
    if n == 0 {
        return prog.clone();
    }

    let code = &prog.code;

    // Step 1: Build the CFG from linear code
    let cfg = build_cfg(code, &prog.entries);

    // Step 2: Merge consecutive blocks (A→Jmp→B where A's only succ is B and B's only pred is A)
    let cfg = cfg_merge_blocks(&cfg, code);

    // Step 3: Remove blocks unreachable from any entry point
    let cfg = cfg_remove_unreachable(&cfg);

    // Step 4: Collapse Jmp→Jmp→target chains into Jmp→target
    let cfg = cfg_simplify_jump_chains(&cfg, code);

    // Step 5: Emit linear code back from the optimized CFG
    emit_cfg(&cfg, prog)
}

// --- Section: SCCP (Sparse Conditional Constant Propagation) ---

// Three-point lattice for each register:
//   Top    = value is unknown / not yet defined (optimistic start)
//   Int/Flt = value is a known concrete constant
//   Bottom = value is overdefined / conflicting (no single constant)
#[derive(Debug, Clone, Copy, PartialEq)]
enum Lattice {
    Top,
    Int(i64),
    Flt(f64),
    Bottom,
}

impl Lattice {
    // Meet (glb) operation: combines two lattice values at CFG join points.
    //   Top meet x = x             (unknown + something = something)
    //   Bottom meet _ = Bottom     (conflict propagates)
    //   Int(a) meet Int(b) = Int(a) if a == b, else Bottom
    //   Flt(a) meet Flt(b) = Flt(a) if a == b (bitwise), else Bottom
    fn meet(self, other: Lattice) -> Lattice {
        match (self, other) {
            (Lattice::Top, x) | (x, Lattice::Top) => x,
            (Lattice::Bottom, _) | (_, Lattice::Bottom) => Lattice::Bottom,
            (Lattice::Int(a), Lattice::Int(b)) if a == b => Lattice::Int(a),
            (Lattice::Flt(a), Lattice::Flt(b)) if a.to_bits() == b.to_bits() => Lattice::Flt(a),
            _ => Lattice::Bottom, // conflicting constants → Bottom
        }
    }
}

/// Sparse Conditional Constant Propagation.
///
/// Uses a lattice (Top → Constant → Bottom) per register, propagating
/// across the CFG.  Conditional branches with constant predicates are
/// folded: the unreachable successor is marked non-executable.
///
/// After convergence, known-constant expressions are replaced with
/// Ldi/Ldi64/Ldc, and blocks gated by a folded branch are removed.
pub fn sccp(prog: &QfrProgram) -> QfrProgram {
    let n = prog.code.len();
    if n == 0 {
        return prog.clone();
    }

    let code = &prog.code;
    let cfg = build_cfg(code, &prog.entries);
    if cfg.blocks.is_empty() {
        return prog.clone();
    }

    // Register lattice (192 int + 64 float = 256 total)
    let mut reg: Vec<Lattice> = vec![Lattice::Top; 256];
    let mut executable: Vec<bool> = vec![false; cfg.blocks.len()];
    let mut worklist: Vec<usize> = Vec::new();

    // Mark entry blocks as executable and seed the worklist
    for &eid in &cfg.entry_ids {
        if eid < cfg.blocks.len() {
            executable[eid] = true;
            worklist.push(eid);
        }
    }

    // Iterate to fixpoint: simulate each executable block, then meet
    // exit states at successor entries to propagate lattice values.
    let mut block_reg: Vec<Vec<Lattice>> = vec![vec![]; cfg.blocks.len()];
    let mut changed = true;
    while changed {
        changed = false;
        let mut pending: Vec<usize> = std::mem::take(&mut worklist);
        block_reg = vec![vec![]; cfg.blocks.len()];

        while let Some(bid) = pending.pop() {
            if !executable[bid] {
                continue;
            }
            let b = &cfg.blocks[bid];
            // Start with the global lattice state at block entry
            let mut local_reg = reg.clone();

            // Simulate each instruction in the block
            for i in b.start..b.end {
                let instr = &code[i];
                let op = instr.opcode();

                match op {
                    O::Ldi => {
                        let rd = instr.rd() as usize;
                        let val = instr.imm_signed() as i64;
                        local_reg[rd] = Lattice::Int(val);
                    }
                    O::Ldi64 => {
                        let rd = instr.rd() as usize;
                        let val = instr.imm40();
                        local_reg[rd] = Lattice::Int(val);
                    }
                    O::LdcF64 => {
                        let rd = instr.rd() as usize;
                        let idx = instr.imm() as usize;
                        if idx < prog.f64_consts.len() {
                            local_reg[rd] = Lattice::Flt(prog.f64_consts[idx]);
                        }
                    }
                    // Int RRR: fold when both operands are known constants
                    O::Add
                    | O::Sub
                    | O::Mul
                    | O::Div
                    | O::Mod
                    | O::BitAnd
                    | O::BitOr
                    | O::BitXor
                    | O::Shl
                    | O::Shr
                    | O::Eq
                    | O::Ne
                    | O::Lt
                    | O::Gt
                    | O::Le
                    | O::Ge => {
                        let rd = instr.rd() as usize;
                        let rs1 = instr.rs1() as usize;
                        let rs2 = instr.rs2() as usize;
                        local_reg[rd] = fold_int_lattice(local_reg[rs1], local_reg[rs2], op);
                    }
                    // Float RRR: fold when both operands are known constants
                    O::FAdd
                    | O::FSub
                    | O::FMul
                    | O::FDiv
                    | O::FEq
                    | O::FNe
                    | O::FLt
                    | O::FGt
                    | O::FLe
                    | O::FGe => {
                        let rd = instr.rd() as usize;
                        let rs1 = instr.rs1() as usize;
                        let rs2 = instr.rs2() as usize;
                        local_reg[rd] = fold_float_lattice(local_reg[rs1], local_reg[rs2], op);
                    }
                    // Int unary: Neg remains Top if src is Top, folds to wrapped neg if Int
                    O::Neg => {
                        let rd = instr.rd() as usize;
                        let rs = instr.rs1() as usize;
                        local_reg[rd] = match local_reg[rs] {
                            Lattice::Int(v) => Lattice::Int(v.wrapping_neg()),
                            Lattice::Top => Lattice::Top,
                            _ => Lattice::Bottom,
                        };
                    }
                    // Float unary: FNeg mirrors float sign
                    O::FNeg => {
                        let rd = instr.rd() as usize;
                        let rs = instr.rs1() as usize;
                        local_reg[rd] = match local_reg[rs] {
                            Lattice::Flt(v) => Lattice::Flt(-v),
                            Lattice::Top => Lattice::Top,
                            _ => Lattice::Bottom,
                        };
                    }
                    // Int immediate arithmetic: fold when rs1 is known
                    O::AddI => {
                        fold_int_imm_lattice(&mut local_reg, instr, |a, b| a.wrapping_add(b as i64))
                    }
                    O::SubI => {
                        fold_int_imm_lattice(&mut local_reg, instr, |a, b| a.wrapping_sub(b as i64))
                    }
                    O::MulI => {
                        fold_int_imm_lattice(&mut local_reg, instr, |a, b| a.wrapping_mul(b as i64))
                    }
                    O::DivI => fold_int_imm_lattice(&mut local_reg, instr, |a, b| {
                        if b == 0 {
                            0
                        } else {
                            a / b as i64
                        }
                    }),
                    // Conversions between int and float
                    O::I2F => {
                        let rd = instr.rd() as usize;
                        let rs = instr.rs1() as usize;
                        local_reg[rd] = match local_reg[rs] {
                            Lattice::Int(v) => Lattice::Flt(v as f64),
                            Lattice::Top => Lattice::Top,
                            _ => Lattice::Bottom,
                        };
                    }
                    O::F2I => {
                        let rd = instr.rd() as usize;
                        let rs = instr.rs1() as usize;
                        local_reg[rd] = match local_reg[rs] {
                            Lattice::Flt(v) => Lattice::Int(v as i64),
                            Lattice::Top => Lattice::Top,
                            _ => Lattice::Bottom,
                        };
                    }
                    // Mov: propagate lattice state from src to dest
                    O::Mov => {
                        let rd = instr.rd() as usize;
                        let rs = instr.rs1() as usize;
                        local_reg[rd] = local_reg[rs];
                    }
                    // Control flow: Jmp marks the successor executable
                    O::Jmp => {
                        if let Some(target_pc) = compute_target(code, i) {
                            let target_bid = cfg
                                .blocks
                                .iter()
                                .position(|b2| target_pc >= b2.start && target_pc < b2.end);
                            if let Some(tid) = target_bid {
                                if !executable[tid] {
                                    executable[tid] = true;
                                    pending.push(tid);
                                    changed = true;
                                }
                            }
                        }
                    }
                    O::Jz | O::Jnz => {
                        let rs = instr.rs1() as usize;
                        let cond = local_reg[rs];
                        // Evaluate taken target and fall-through
                        if let Some(target_pc) = compute_target(code, i) {
                            let target_bid = cfg
                                .blocks
                                .iter()
                                .position(|b2| target_pc >= b2.start && target_pc < b2.end);
                            if let Some(tid) = target_bid {
                                let take_branch = match (op, cond) {
                                    (O::Jz, Lattice::Int(0)) => true,   // Jz: cond==0 → taken
                                    (O::Jz, Lattice::Int(_)) => false,  // Jz: cond!=0 → fall
                                    (O::Jnz, Lattice::Int(0)) => false, // Jnz: cond==0 → fall
                                    (O::Jnz, Lattice::Int(_)) => true,  // Jnz: cond!=0 → taken
                                    _ => false, // Not constant → both paths are possible
                                };
                                let fallthrough_bid = if bid + 1 < cfg.blocks.len() {
                                    Some(bid + 1)
                                } else {
                                    None
                                };
                                if let Lattice::Int(_) = cond {
                                    // Constant condition: only one successor is reachable
                                    if take_branch {
                                        if !executable[tid] {
                                            executable[tid] = true;
                                            pending.push(tid);
                                            changed = true;
                                        }
                                    } else if let Some(fid) = fallthrough_bid {
                                        if !executable[fid] {
                                            executable[fid] = true;
                                            pending.push(fid);
                                            changed = true;
                                        }
                                    }
                                } else {
                                    // Non-constant condition: both successors are reachable
                                    if !executable[tid] {
                                        executable[tid] = true;
                                        pending.push(tid);
                                        changed = true;
                                    }
                                    if let Some(fid) = fallthrough_bid {
                                        if !executable[fid] {
                                            executable[fid] = true;
                                            pending.push(fid);
                                            changed = true;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    O::Call => {
                        // Call target is reachable
                        if let Some(target_pc) = compute_target(code, i) {
                            let target_bid = cfg
                                .blocks
                                .iter()
                                .position(|b2| target_pc >= b2.start && target_pc < b2.end);
                            if let Some(tid) = target_bid {
                                if !executable[tid] {
                                    executable[tid] = true;
                                    pending.push(tid);
                                    changed = true;
                                }
                            }
                        }
                        // Fall-through after return is also reachable
                        if bid + 1 < cfg.blocks.len() {
                            let fid = bid + 1;
                            if !executable[fid] {
                                executable[fid] = true;
                                pending.push(fid);
                                changed = true;
                            }
                        }
                    }
                    O::Ret | O::Halt => {
                        // Execution terminates here; no successors
                    }
                    // Side-effect / stateful ops: mark dest as Bottom (unknown)
                    _ => {
                        let rd = instr.rd() as usize;
                        local_reg[rd] = Lattice::Bottom;
                    }
                }
            }
            block_reg[bid] = local_reg;
        }

        // Meet lattice values at block boundaries: at each successor's entry,
        // the global register state is the meet of all predecessor exit states.
    #[allow(clippy::needless_range_loop)]
    for bid in 0..cfg.blocks.len() {
        if !executable[bid] {
                continue;
            }
            if block_reg[bid].is_empty() {
                continue;
            }
            let exit_state = &block_reg[bid];
            let b = &cfg.blocks[bid];

            for &s in &b.succ {
                if !executable[s] {
                    continue;
                }
                let entry_reg = &mut reg;
                for r in 0..256 {
                    let new_val = entry_reg[r].meet(exit_state[r]);
                    if new_val != entry_reg[r] {
                        entry_reg[r] = new_val;
                        changed = true;
                        if !pending.contains(&s) {
                            pending.push(s);
                        }
                    }
                }
            }
        }
        worklist = pending;
    }

    // After convergence, merge exit states of all executable blocks into reg
    for bid in 0..cfg.blocks.len() {
        if !executable[bid] {
            continue;
        }
        if block_reg[bid].is_empty() {
            continue;
        }
        let exit_state = &block_reg[bid];
        for r in 0..256 {
            reg[r] = reg[r].meet(exit_state[r]);
        }
    }

    // ── Emit optimized code ──
    // Use the converged lattice to replace known-constant results with Ldi/Ldi64,
    // and fold constant-condition branches into unconditional Jmp or fall-through.
    let mut old_to_new: Vec<Option<usize>> = vec![None; n];
    let mut new_code: Vec<Instruction> = Vec::new();

    // Re-check reachability: mark non-executable blocks that were gated by a
    // constant-condition branch that ended up folding to the non-taken side.
    for bid in 0..cfg.blocks.len() {
        if !executable[bid] {
            continue;
        }
        let b = &cfg.blocks[bid];

        if b.end > b.start {
            let term_pc = b.end - 1;
            let op = code[term_pc].opcode();
            if matches!(op, O::Jz | O::Jnz) {
                let rs = code[term_pc].rs1() as usize;
                if let Lattice::Int(cond_val) = reg[rs] {
                    let take_branch = match (op, cond_val) {
                        (O::Jz, 0) => true,
                        (O::Jnz, 0) => false,
                        _ => op == O::Jnz,
                    };
                    if !take_branch {
                        // Branch not taken: mark the jump target as unreachable,
                        // but only if ALL its predecessors are now non-executable.
                        if let Some(target_pc) = compute_target(code, term_pc) {
                            let target_bid = cfg
                                .blocks
                                .iter()
                                .position(|b2| target_pc >= b2.start && target_pc < b2.end);
                            if let Some(tid) = target_bid {
                                let any_pred_exec = cfg.blocks[tid].pred
                                    .iter()
                                    .any(|&pred| executable[pred]);
                                if !any_pred_exec {
                                    executable[tid] = false;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Emit instructions from each executable block, folding constants where possible
    #[allow(clippy::needless_range_loop)]
    for bid in 0..cfg.blocks.len() {
        if !executable[bid] {
            continue;
        }
        let b = &cfg.blocks[bid];

        for i in b.start..b.end {
            let instr = &code[i];
            let op = instr.opcode();
            let rd = instr.rd() as usize;

            // Skip non-foldable ops (side effects, state access, control flow)
            let can_fold = !matches!(
                op,
                O::Jmp
                    | O::Jz
                    | O::Jnz
                    | O::Call
                    | O::Ret
                    | O::Halt
                    | O::SendOrder
                    | O::Sentinel
                    | O::Log
                    | O::PersistGet
                    | O::PersistSet
                    | O::GetInd
                    | O::GetPrice
                    | O::GetPos
                    | O::GetBal
                    | O::WindowPush
                    | O::WindowMean
                    | O::WindowStddev
                    | O::WindowMin
                    | O::WindowMax
                    | O::WindowSum
            );

            if can_fold {
                if let Lattice::Int(val) = reg[rd] {
                    if val >= i32::MIN as i64 && val <= i32::MAX as i64 {
                        old_to_new[i] = Some(new_code.len());
                        new_code.push(Instruction::rri(O::Ldi, rd as u8, 0, val as u32));
                        continue;
                    } else if (-(1i64 << 39)..(1i64 << 39)).contains(&val) {
                        old_to_new[i] = Some(new_code.len());
                        new_code.push(Instruction::ri40(O::Ldi64, rd as u8, val));
                        continue;
                    }
                    // Value too large for immediate encoding → keep original
                }
                // Float constants keep original instruction to avoid const_pool management
            }

            // Fold constant-condition branches into Jmp or fall-through
            if matches!(op, O::Jz | O::Jnz) {
                let rs = instr.rs1() as usize;
                if let Lattice::Int(cond_val) = reg[rs] {
                    let take_branch = match (op, cond_val) {
                        (O::Jz, 0) => true,
                        (O::Jnz, 0) => false,
                        _ => op == O::Jnz,
                    };
                    if take_branch {
                        // Replace conditional branch with unconditional Jmp
                        let orig_imm = code[i].imm_signed();
                        old_to_new[i] = Some(new_code.len());
                        new_code.push(Instruction::rri(O::Jmp, 0, 0, orig_imm as u32));
                    } else {
                        // Branch not taken → omit entirely (fall through to next block)
                        continue;
                    }
                    continue;
                }
            }

            old_to_new[i] = Some(new_code.len());
            new_code.push(code[i]);
        }
    }

    // Pass 2: recalculate jump offsets in the new instruction stream
    let mut new_to_old: Vec<Option<usize>> = vec![None; new_code.len()];
    for (old, new) in old_to_new.iter().enumerate() {
        if let Some(n) = new {
            new_to_old[*n] = Some(old);
        }
    }

    for ni in 0..new_code.len() {
        let old_i = match new_to_old[ni] {
            Some(v) => v,
            None => continue,
        };
        let op = new_code[ni].opcode();
        if !is_terminator(op) {
            continue;
        }

        match op {
            O::Jmp => {
                if let Some(old_target) = compute_target(code, old_i) {
                    if let Some(new_target) = old_to_new[old_target] {
                        let offset = new_target as i64 - ni as i64 - 1;
                        new_code[ni] = Instruction::rri(O::Jmp, 0, 0, offset as u32);
                    }
                }
            }
            O::Jz | O::Jnz => {
                if let Some(old_target) = compute_target(code, old_i) {
                    if let Some(new_target) = old_to_new[old_target] {
                        let offset = new_target as i64 - ni as i64 - 1;
                        let cond_reg = code[old_i].rs1();
                        new_code[ni] = Instruction::rri(op, 0, cond_reg, offset as u32);
                    }
                }
            }
            _ => {}
        }
    }

    // Update entry point offsets after code changes
    let mut new_entries = prog.entries.clone();
    for entry in &mut new_entries {
        let old_off = entry.code_offset as usize;
        if let Some(new_off) = old_to_new.get(old_off).copied().flatten() {
            entry.code_offset = new_off as u32;
        } else {
            entry.code_offset = 0;
        }
    }

    let mut out = QfrProgram::new();
    out.entries = new_entries;
    out.const_pool = prog.const_pool.clone();
    out.const_map = prog.const_map.clone();
    out.ema_alphas = prog.ema_alphas.clone();
    out.f64_consts = prog.f64_consts.clone();
    out.i64_consts = prog.i64_consts.clone();
    out.string_consts = prog.string_consts.clone();
    out.code = new_code;
    out
}

// Evaluates an integer RRR op on two lattice values.
// Wrapping arithmetic for all integer operations. Division/mod by zero returns 0.
// Returns Top if either operand is Top, Bottom on type mismatch.
fn fold_int_lattice(a: Lattice, b: Lattice, op: O) -> Lattice {
    match (a, b) {
        (Lattice::Int(a), Lattice::Int(b)) => {
            let result = match op {
                O::Add => a.wrapping_add(b),
                O::Sub => a.wrapping_sub(b),
                O::Mul => a.wrapping_mul(b),
                O::Div => {
                    if b == 0 {
                        0
                    } else {
                        a.wrapping_div(b)
                    }
                }
                O::Mod => {
                    if b == 0 {
                        0
                    } else {
                        a.wrapping_rem(b)
                    }
                }
                O::BitAnd => a & b,
                O::BitOr => a | b,
                O::BitXor => a ^ b,
                O::Shl => a.wrapping_shl(b as u32),
                O::Shr => a.wrapping_shr(b as u32),
                O::Eq => {
                    if a == b {
                        1
                    } else {
                        0
                    }
                }
                O::Ne => {
                    if a != b {
                        1
                    } else {
                        0
                    }
                }
                O::Lt => {
                    if a < b {
                        1
                    } else {
                        0
                    }
                }
                O::Gt => {
                    if a > b {
                        1
                    } else {
                        0
                    }
                }
                O::Le => {
                    if a <= b {
                        1
                    } else {
                        0
                    }
                }
                O::Ge => {
                    if a >= b {
                        1
                    } else {
                        0
                    }
                }
                _ => return Lattice::Bottom,
            };
            Lattice::Int(result)
        }
        (Lattice::Top, _) | (_, Lattice::Top) => Lattice::Top,
        _ => Lattice::Bottom,
    }
}

// Evaluates a float RRR op on two lattice values.
// If either operand is Top, returns Top (unknown). If both are Flt, computes the result.
// Mismatched types or incompatible values fall to Bottom (overdefined).
fn fold_float_lattice(a: Lattice, b: Lattice, op: O) -> Lattice {
    match (a, b) {
        (Lattice::Flt(a), Lattice::Flt(b)) => {
            let result = match op {
                O::FAdd => a + b,
                O::FSub => a - b,
                O::FMul => a * b,
                O::FDiv => {
                    if b == 0.0 {
                        0.0
                    } else {
                        a / b
                    }
                }
                O::FEq => {
                    if (a - b).abs() < f64::EPSILON {
                        1.0
                    } else {
                        0.0
                    }
                }
                O::FNe => {
                    if (a - b).abs() >= f64::EPSILON {
                        1.0
                    } else {
                        0.0
                    }
                }
                O::FLt => {
                    if a < b {
                        1.0
                    } else {
                        0.0
                    }
                }
                O::FGt => {
                    if a > b {
                        1.0
                    } else {
                        0.0
                    }
                }
                O::FLe => {
                    if a <= b {
                        1.0
                    } else {
                        0.0
                    }
                }
                O::FGe => {
                    if a >= b {
                        1.0
                    } else {
                        0.0
                    }
                }
                _ => return Lattice::Bottom,
            };
            Lattice::Flt(result)
        }
        (Lattice::Top, _) | (_, Lattice::Top) => Lattice::Top,
        _ => Lattice::Bottom,
    }
}

// Applies an RRI fold function to an instruction using the current lattice state.
// If rs1 is lattice Top, the result stays Top; if Bottom or mismatch, stays Bottom.
fn fold_int_imm_lattice(reg: &mut [Lattice], instr: &Instruction, f: fn(i64, u32) -> i64) {
    let rd = instr.rd() as usize;
    let rs = instr.rs1() as usize;
    let imm = instr.imm();
    match reg[rs] {
        Lattice::Int(a) => reg[rd] = Lattice::Int(f(a, imm)),
        Lattice::Top => reg[rd] = Lattice::Top,
        _ => reg[rd] = Lattice::Bottom,
    }
}

// --- Section: Persist Coalesce (persist_coalesce) ---

/// PersistGet/Set coalescing optimization.
///
/// Removes redundant PersistGet when the same slot is already cached in a
/// register, and removes redundant PersistSet when the register value hasn't
/// changed since the last PersistGet of the same slot.
pub fn persist_coalesce(prog: &QfrProgram) -> QfrProgram {
    use std::collections::{HashMap, HashSet};
    let mut out = QfrProgram {
        entries: prog.entries.clone(),
        const_pool: prog.const_pool.clone(),
        code: Vec::with_capacity(prog.code.len()),
        const_map: prog.const_map.clone(),
        ema_alphas: prog.ema_alphas.clone(),
        f64_consts: prog.f64_consts.clone(),
        i64_consts: prog.i64_consts.clone(),
        string_consts: prog.string_consts.clone(),
    };

    // Track which persist slot maps to which register, and which regs have been modified
    let entry_offsets: HashSet<u32> = prog.entries.iter().map(|e| e.code_offset).collect();
    let mut slot_reg: HashMap<u32, u8> = HashMap::new();
    let mut dirty: HashSet<u8> = HashSet::new();

    for (i, instr) in prog.code.iter().enumerate() {
        let off = i as u32;

        // Entry points reset the shadow state (fresh persist context)
        if entry_offsets.contains(&off) {
            slot_reg.clear();
            dirty.clear();
        }

        let op = instr.opcode();
        let rd = instr.rd();

        // Determine if this instruction writes to its destination register
        let writes_rd = !matches!(
            op,
            O::Jmp | O::Jz | O::Jnz | O::Ret | O::Halt | O::SendOrder | O::Log
        );

        match op {
            O::PersistGet => {
                let r = rd;
                let slot = instr.imm();
                // Slot already in a register and not dirty → replace with Mov
                if let Some(&cached_r) = slot_reg.get(&slot) {
                    if !dirty.contains(&cached_r) {
                        if cached_r == r {
                            continue; // same register, redundant
                        }
                        out.code.push(Instruction::rr(O::Mov, r, cached_r));
                        slot_reg.insert(slot, r);
                        dirty.remove(&r);
                        continue;
                    }
                }
                out.code.push(*instr);
                slot_reg.insert(slot, r);
                dirty.remove(&r);
            }
            O::PersistSet => {
                let r = rd;
                let slot = instr.imm();
                // Writing same value back to the same slot → redundant
                if let Some(&cached_r) = slot_reg.get(&slot) {
                    if cached_r == r && !dirty.contains(&r) {
                        continue;
                    }
                }
                out.code.push(*instr);
                slot_reg.insert(slot, r);
                dirty.remove(&r);
            }
            _ => {
                if writes_rd {
                    dirty.insert(rd); // register value changed
                }
                out.code.push(*instr);
                if is_terminator(op) {
                    // Basic block boundary: reset shadow state
                    slot_reg.clear();
                    dirty.clear();
                }
            }
        }
    }
    out
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
        let p = make_prog(vec![I::rri(O::Ldi, 0, 0, 42), I::rr(O::Neg, 1, 0)]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].opcode(), O::Ldi);
        assert_eq!(opt.code[1].imm_signed(), -42);
    }

    #[test]
    fn fold_mov_propagates_lit_to_int() {
        let p = make_prog(vec![I::rri(O::Ldi, 0, 0, 100), I::rr(O::Mov, 1, 0)]);
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
            I::rrr(O::Add, 2, 0, 1), // 2+3=5
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
            I::rri(O::LdcF64, 192, 0, idx1),
            I::rri(O::LdcF64, 193, 0, idx2),
            I::rrr(O::FAdd, 194, 192, 193),
        ];
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].opcode(), O::LdcF64);

        // Check the folded constant pool entry
        let f_idx = opt.code[2].imm() as usize;
        if f_idx < opt.f64_consts.len() {
            let val = &opt.f64_consts[f_idx];
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
            I::rri(O::LdcF64, 192, 0, idx1),
            I::rri(O::LdcF64, 193, 0, idx2),
            I::rrr(O::FSub, 194, 192, 193),
        ];
        let opt = constant_fold(&p);
        let f_idx = opt.code[2].imm() as usize;
        if f_idx < opt.f64_consts.len() {
            let val = &opt.f64_consts[f_idx];
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
            I::rri(O::LdcF64, 192, 0, idx1),
            I::rri(O::LdcF64, 193, 0, idx2),
            I::rrr(O::FMul, 194, 192, 193),
        ];
        let opt = constant_fold(&p);
        let f_idx = opt.code[2].imm() as usize;
        if f_idx < opt.f64_consts.len() {
            let val = &opt.f64_consts[f_idx];
            assert!((*val - 4.5).abs() < 0.0001);
        } else {
            panic!("expected F64");
        }
    }

    #[test]
    fn fold_float_neg() {
        let mut p = QfrProgram::new();
        let idx = p.intern_f64(3.14);
        p.code = vec![I::rri(O::LdcF64, 192, 0, idx), I::rr(O::FNeg, 193, 192)];
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].opcode(), O::LdcF64);
        let f_idx = opt.code[1].imm() as usize;
        if f_idx < opt.f64_consts.len() {
            let val = &opt.f64_consts[f_idx];
            assert!((*val - -3.14).abs() < 0.0001);
        } else {
            panic!("expected F64");
        }
    }

    // ── Conversion folding ──

    #[test]
    fn fold_i2f() {
        let p = make_prog(vec![I::rri(O::Ldi, 0, 0, 42), I::rr(O::I2F, 192, 0)]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].opcode(), O::LdcF64);
        let f_idx = opt.code[1].imm() as usize;
        if f_idx < opt.f64_consts.len() {
            let val = &opt.f64_consts[f_idx];
            assert!((*val - 42.0).abs() < 0.0001);
        } else {
            panic!("expected F64");
        }
    }

    #[test]
    fn fold_f2i() {
        let mut p = QfrProgram::new();
        let idx = p.intern_f64(3.99);
        p.code = vec![I::rri(O::LdcF64, 192, 0, idx), I::rr(O::F2I, 1, 192)];
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
            I::ri40(O::Ldi64, 0, 400_000_000_000i64),
            I::ri40(O::Ldi64, 1, 200_000_000_000i64),
            I::rrr(O::Add, 2, 0, 1), // 400_000_000_000 + 200_000_000_000 = 600_000_000_000
        ];
        let opt = constant_fold(&p);
        // Result doesn't fit in 32-bit or 40-bit → uses Ldc
        assert_eq!(opt.code[2].opcode(), O::LdcF64);
        let f_idx = opt.code[2].imm() as usize;
        if f_idx < opt.f64_consts.len() {
            let val = &opt.f64_consts[f_idx];
            assert!((*val - 600_000_000_000.0).abs() < 0.5);
        } else {
            panic!("expected F64 const entry");
        }
    }

    // ── Float comparison folding ──

    #[test]
    fn fold_float_cmp_eq() {
        let mut p = QfrProgram::new();
        let idx = p.intern_f64(3.14);
        p.code = vec![
            I::rri(O::LdcF64, 192, 0, idx),
            I::rri(O::LdcF64, 193, 0, idx),
            I::rrr(O::FEq, 2, 192, 193),
        ];
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].opcode(), O::LdcF64);
    }

    #[test]
    fn fold_float_cmp_lt() {
        let mut p = QfrProgram::new();
        let i1 = p.intern_f64(1.0);
        let i2 = p.intern_f64(2.0);
        p.code = vec![
            I::rri(O::LdcF64, 192, 0, i1),
            I::rri(O::LdcF64, 193, 0, i2),
            I::rrr(O::FLt, 2, 192, 193),
        ];
        let opt = constant_fold(&p);
        let f_idx = opt.code[2].imm() as usize;
        if f_idx < opt.f64_consts.len() {
            let val = &opt.f64_consts[f_idx];
            assert!(
                (*val - 1.0).abs() < 0.0001,
                "1.0 < 2.0 should be true (1.0)"
            );
        } else {
            panic!("expected F64");
        }
    }

    // ── Optimize pipeline ──

    #[test]
    fn optimize_runs_without_panicking() {
        let mut p = QfrProgram::new();
        p.entries.push(crate::ir::EntryPoint {
            name: "main".into(),
            code_offset: 0,
        });
        p.code = vec![I::single(O::Ret)];
        optimize(&mut p);
        assert_eq!(p.code.len(), 1);
        assert_eq!(p.code[0].opcode(), O::Ret);
    }

    #[test]
    fn optimize_with_no_entries_removes_all_code() {
        let mut p = make_prog(vec![I::single(O::Ret)]);
        optimize(&mut p);
        assert!(p.code.is_empty());
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
            I::rri(O::LdcF64, 192, 0, ema_idx),
            I::rri(O::LdcF64, 193, 0, mult_idx),
            I::rrr(O::FMul, 194, 192, 193), // 50000 * 0.002 = 100
            I::rrr(O::FMul, 195, 192, 194), // 50000 * 100 = 5_000_000
        ];
        let opt = constant_fold(&p);
        assert_eq!(opt.code[2].opcode(), O::LdcF64);
        assert_eq!(opt.code[3].opcode(), O::LdcF64);
    }

    #[test]
    fn control_flow_jmp_clears_state() {
        let p = make_prog(vec![
            I::rri(O::Ldi, 0, 0, 5),
            I::ri(O::Jmp, 0, 3),      // unconditional jump
            I::rri(O::Ldi, 1, 0, 10), // after jmp (would be unreachable)
            I::rrr(O::Add, 2, 0, 1),  // after jmp target
        ]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code.len(), 4);
    }

    // ── Immediate op folding ──

    #[test]
    fn fold_addi() {
        let p = make_prog(vec![I::rri(O::Ldi, 0, 0, 10), I::rri(O::AddI, 1, 0, 5)]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].opcode(), O::Ldi);
        assert_eq!(opt.code[1].imm_signed(), 15);
    }

    #[test]
    fn fold_subi() {
        let p = make_prog(vec![I::rri(O::Ldi, 0, 0, 20), I::rri(O::SubI, 1, 0, 7)]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].imm_signed(), 13);
    }

    #[test]
    fn fold_muli() {
        let p = make_prog(vec![I::rri(O::Ldi, 0, 0, 6), I::rri(O::MulI, 1, 0, 7)]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].imm_signed(), 42);
    }

    #[test]
    fn fold_divi() {
        let p = make_prog(vec![I::rri(O::Ldi, 0, 0, 100), I::rri(O::DivI, 1, 0, 4)]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].imm_signed(), 25);
    }

    #[test]
    fn fold_eqi() {
        let p = make_prog(vec![I::rri(O::Ldi, 0, 0, 42), I::rri(O::EqI, 1, 0, 42)]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].imm_signed(), 1);
    }

    #[test]
    fn fold_lti() {
        let p = make_prog(vec![I::rri(O::Ldi, 0, 0, 3), I::rri(O::LtI, 1, 0, 10)]);
        let opt = constant_fold(&p);
        assert_eq!(opt.code[1].imm_signed(), 1);
    }

    #[test]
    fn fold_gti() {
        let p = make_prog(vec![I::rri(O::Ldi, 0, 0, 15), I::rri(O::GtI, 1, 0, 10)]);
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
            I::rri(O::LdcF64, 192, 0, i1),
            I::rri(O::LdcF64, 193, 0, i2),
            I::rrr(O::FDiv, 194, 192, 193),
        ];
        let opt = constant_fold(&p);
        let f_idx = opt.code[2].imm() as usize;
        if f_idx < opt.f64_consts.len() {
            let val = &opt.f64_consts[f_idx];
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
        assert_eq!(opt.string_consts.len(), 1);
        assert_eq!(opt.string_consts[s_idx as usize], "test");
    }

    #[test]
    fn dce_preserves_const_pool() {
        let mut p = QfrProgram::new();
        p.entries.push(crate::ir::EntryPoint {
            name: "main".into(),
            code_offset: 0,
        });
        let s_idx = p.intern_string("hello");
        p.code = vec![I::rri(O::LdcF64, 0, 0, s_idx), I::single(O::Ret)];
        let opt = dead_code_eliminate(&p);
        assert_eq!(opt.const_pool.len(), 1);
        assert!(opt.const_map.contains_key("hello"));
    }

    #[test]
    fn dce_preserves_entry_points() {
        let mut p = QfrProgram::new();
        p.entries.push(crate::ir::EntryPoint {
            name: "on_trade".into(),
            code_offset: 0,
        });
        p.code = vec![I::single(O::Ret)];
        let opt = dead_code_eliminate(&p);
        assert_eq!(opt.entries.len(), 1);
        assert_eq!(opt.entries[0].name, "on_trade");
        assert_eq!(opt.entries[0].code_offset, 0);
    }

    #[test]
    fn optimize_includes_dce_in_pipeline() {
        let mut p = QfrProgram::new();
        p.entries.push(crate::ir::EntryPoint {
            name: "main".into(),
            code_offset: 0,
        });
        p.code = vec![
            I::rri(O::Ldi, 0, 0, 1),  // [0]
            I::ri(O::Jmp, 0, 2),      // [1] jump to [4]
            I::rri(O::Ldi, 1, 0, 99), // [2] dead
            I::rri(O::Ldi, 2, 0, 99), // [3] dead
            I::rri(O::Ldi, 3, 0, 42), // [4] target
            I::single(O::Ret),        // [5]
        ];
        // Reachable: [0], [1], [4], [5]
        optimize(&mut p);
        assert_eq!(p.code.len(), 4);
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
        let p = make_prog(vec![I::rrr(O::BitAnd, 2, 0, 1), I::rrr(O::BitAnd, 3, 0, 1)]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code.len(), 2);
        assert_eq!(opt.code[1].opcode(), O::Mov);
    }

    #[test]
    fn cse_comparison_eliminated() {
        let p = make_prog(vec![I::rrr(O::Gt, 2, 0, 1), I::rrr(O::Gt, 3, 0, 1)]);
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
            I::rrr(O::Add, 2, 0, 1),  // cache (Add, r0, r1) → r2
            I::rri(O::Ldi, 1, 0, 10), // r1 overwritten → invalidates
            I::rrr(O::Add, 3, 0, 1),  // no match → full Add
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code[2].opcode(), O::Add);
    }

    #[test]
    fn cse_invalidated_when_cached_rd_overwritten() {
        let p = make_prog(vec![
            I::rrr(O::Add, 2, 0, 1),  // cache (Add, r0, r1) → r2
            I::rri(O::Ldi, 2, 0, 99), // r2 overwritten → cache entry invalid
            I::rrr(O::Add, 3, 0, 1),  // no match → full Add (r2's value lost)
        ]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code[2].opcode(), O::Add);
    }

    #[test]
    fn cse_preserves_entry_points_and_const_pool() {
        let mut p = QfrProgram::new();
        p.entries.push(crate::ir::EntryPoint {
            name: "main".into(),
            code_offset: 0,
        });
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
        let p = make_prog(vec![I::rr(O::FNeg, 193, 192), I::rr(O::FNeg, 194, 192)]);
        let opt = common_subexpr_elim(&p);
        assert_eq!(opt.code[1].opcode(), O::Mov);
    }

    #[test]
    fn cse_eq_eliminated() {
        let p = make_prog(vec![I::rrr(O::Eq, 2, 0, 1), I::rrr(O::Eq, 3, 0, 1)]);
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
        let p = make_prog(vec![I::rrr(O::Shl, 2, 0, 1), I::rrr(O::Shl, 3, 0, 1)]);
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
        let p = make_prog(vec![I::rri(O::Ldi, 0, 0, 1), I::single(O::Ret)]);
        let opt = dead_code_eliminate(&p);
        assert!(opt.code.is_empty());
    }

    // ── CFG Simplification ──

    fn make_prog_entry(code: Vec<I>, entry_offsets: &[u32]) -> QfrProgram {
        let mut p = QfrProgram::new();
        for &off in entry_offsets {
            p.entries.push(crate::ir::EntryPoint {
                name: "test".into(),
                code_offset: off,
            });
        }
        p.code = code;
        p
    }

    #[test]
    fn cfg_simplify_empty_code() {
        let p = QfrProgram::new();
        let opt = cfg_simplify(&p);
        assert!(opt.code.is_empty());
    }

    #[test]
    fn cfg_simplify_straight_line() {
        let p = make_prog_entry(
            vec![
                I::rri(O::Ldi, 0, 0, 1),
                I::rri(O::Ldi, 1, 0, 2),
                I::rrr(O::Add, 2, 0, 1),
                I::single(O::Ret),
            ],
            &[0],
        );
        let opt = cfg_simplify(&p);
        // Single block, all instructions kept
        assert_eq!(opt.code.len(), 4);
        // Entry should still point to instruction 0
        assert_eq!(opt.entries[0].code_offset, 0);
    }

    #[test]
    fn cfg_simplify_removes_jmp_to_next() {
        // Jmp 0 = fall-through, should be removed
        let p = make_prog_entry(
            vec![
                I::rri(O::Ldi, 0, 0, 10), // 0
                I::rri(O::Ldi, 1, 0, 20), // 1
                I::rri(O::Jmp, 0, 0, 0),  // 2: Jmp to 3 (next instruction)
                I::rrr(O::Add, 2, 0, 1),  // 3
                I::single(O::Ret),        // 4
            ],
            &[0],
        );
        let opt = cfg_simplify(&p);
        assert_eq!(opt.code.len(), 4); // Jmp removed
        assert_eq!(opt.code[0].opcode(), O::Ldi);
        assert_eq!(opt.code[2].opcode(), O::Add);
        assert_eq!(opt.code[3].opcode(), O::Ret);
    }

    #[test]
    fn cfg_simplify_merges_blocks() {
        // Two blocks: [0-2) Jmp→next, [2-4). After merge: single block.
        let p = make_prog_entry(
            vec![
                I::rri(O::Ldi, 0, 0, 5), // 0
                I::rri(O::Jmp, 0, 0, 0), // 1: Jmp to 2 (next)
                I::rri(O::Ldi, 1, 0, 3), // 2
                I::single(O::Ret),       // 3
            ],
            &[0],
        );
        let opt = cfg_simplify(&p);
        assert_eq!(opt.code.len(), 3); // Jmp removed
        assert_eq!(opt.code[0].opcode(), O::Ldi);
        assert_eq!(opt.code[1].opcode(), O::Ldi);
        assert_eq!(opt.code[2].opcode(), O::Ret);
    }

    #[test]
    fn cfg_simplify_if_else_keeps_structure() {
        // if/else: [Ldi, Ldi, Jz→else, Add(then), Jmp→end, Ldi(else), Ret(end)]
        let p = make_prog_entry(
            vec![
                I::rri(O::Ldi, 0, 0, 5),  // 0
                I::rri(O::Ldi, 1, 0, 10), // 1
                I::rri(O::Jz, 0, 0, 2),   // 2: if r0==0, jump to 5 (else)
                I::rrr(O::Add, 2, 0, 1),  // 3: then block
                I::rri(O::Jmp, 0, 0, 1),  // 4: Jmp to 6 (end)
                I::rri(O::Ldi, 2, 0, 99), // 5: else block
                I::single(O::Ret),        // 6: end
            ],
            &[0],
        );
        let opt = cfg_simplify(&p);
        // Blocks: [0-3), [3-5), [5-6), [6-7)
        // Block 1 (then): Jmp at 4 → target 6 (block 3). Not adjacent (block 2 in between). Jmp stays.
        // Block 2 (else): falls through to block 3. Jmp at 4 stays.
        assert_eq!(opt.code.len(), 7); // no Jmps removed
                                       // Jz to else (position 5): offset = 5-2-1 = 2
        assert_eq!(opt.code[2].imm_signed(), 2);
        // Jmp to end (position 6): offset = 6-4-1 = 1
        assert_eq!(opt.code[4].imm_signed(), 1);
    }

    #[test]
    fn cfg_simplify_if_without_else() {}

    #[test]
    fn cfg_simplify_removes_unreachable_block() {
        let p = make_prog_entry(
            vec![
                I::rri(O::Ldi, 0, 0, 1), // 0
                I::rri(O::Jmp, 0, 0, 3), // 1: Jmp to 5
                I::rri(O::Ldi, 1, 0, 2), // 2: never reached
                I::single(O::Ret),       // 3: never reached
                I::rri(O::Ldi, 2, 0, 3), // 4
                I::single(O::Ret),       // 5
            ],
            &[0],
        );
        let opt = cfg_simplify(&p);
        // Jmp at 1→5. Code at 2,3 is unreachable. After CFG: block [0-2) → block [4-6)
        // Wait, leaders: 0(entry), 2(fallthrough from Jmp), 5(target of Jmp)
        // Actually Jmp at 1: target=1+1+3=5. Fallthrough at 2.
        // Leaders: 0, 2, 5. Blocks: [0-2), [2-5), [5-6)
        // Block 0: Jmp → block 2. Block 1: [2-5) never reached.
        // Block 2: [5-6) Ret.
        // Remove unreachable: only block 0 and 2 remain.
        // Block 0 Jmp to block 2: need to recalc offset
        // Block 0: start=0, end=2 → instr 0-1 (Jmp)
        // Block 2: start=5, end=6 → instr 5 (Ret)
        // After emission: code = [Ldi, Jmp→?, Ret]
        // Jmp now at position 1, target Ret at position 2
        // offset = 2 - 1 - 1 = 0
        assert_eq!(opt.code.len(), 3);
        assert_eq!(opt.code[0].opcode(), O::Ldi);
        assert_eq!(opt.code[1].opcode(), O::Jmp);
        assert_eq!(opt.code[1].imm_signed(), 0); // Jmp to next instruction
        assert_eq!(opt.code[2].opcode(), O::Ret);
    }

    #[test]
    fn cfg_simplify_jump_chain() {
        // A → B → C where A has Jmp to B, B has Jmp to C
        // A's Jmp should redirect to C directly
        let p = make_prog_entry(
            vec![
                I::rri(O::Ldi, 0, 0, 1), // 0
                I::rri(O::Jmp, 0, 0, 2), // 1: Jmp to 4
                I::rri(O::Ldi, 1, 0, 2), // 2: intermediate
                I::rri(O::Jmp, 0, 0, 0), // 3: Jmp to 4 (next)
                I::rri(O::Ldi, 2, 0, 3), // 4
                I::single(O::Ret),       // 5
            ],
            &[0],
        );
        let opt = cfg_simplify(&p);
        // After merge: Jmp at 3 to next removed (adjacent)
        // After chain simplification: Jmp at 1 redirects to block C (if B was removed)
        // B gets merged with C if single succ/pred
        // Actually B has Jmp to C, and C follows B, so merge.
        // Then A has Jmp to B, B is gone, so... by pred/succ remapping, A should point to C now.
        // But this depends on whether the chain simplification runs
        assert_eq!(opt.code.len(), 4);
    }

    #[test]
    fn cfg_simplify_preserves_multiple_entries() {
        use crate::ir::EntryPoint;
        let mut p = QfrProgram::new();
        p.entries = vec![
            EntryPoint {
                name: "main".into(),
                code_offset: 0,
            },
            EntryPoint {
                name: "on_trade".into(),
                code_offset: 3,
            },
        ];
        p.code = vec![
            I::rri(O::Ldi, 0, 0, 1), // 0: main entry
            I::rri(O::Jmp, 0, 0, 2), // 1: Jmp to 4
            I::single(O::Ret),       // 2: never reached
            I::rri(O::Ldi, 1, 0, 2), // 3: on_trade entry
            I::rri(O::Ldi, 2, 0, 3), // 4
            I::single(O::Ret),       // 5
        ];
        let opt = cfg_simplify(&p);
        // on_trade entry at 3 kept, main redirects
        assert_eq!(opt.entries.len(), 2);
        assert_eq!(opt.entries[1].name, "on_trade");
        // on_trade entry should still point to a valid instruction
        assert!(opt.entries[1].code_offset < opt.code.len() as u32);
    }

    // ── Sparse Conditional Constant Propagation (SCCP) ──

    #[test]
    fn sccp_constant_jnz_always_taken() {
        // Jnz with r0=1 → always taken, then-block removed
        let p = make_prog_entry(
            vec![
                I::rri(O::Ldi, 0, 0, 1),  // 0
                I::rri(O::Jnz, 0, 0, 2),  // 1: r0!=0 → jump to 4
                I::rri(O::Ldi, 1, 0, 10), // 2: then (dead)
                I::single(O::Ret),        // 3
                I::rri(O::Ldi, 1, 0, 99), // 4: else
                I::single(O::Ret),        // 5
            ],
            &[0],
        );
        let opt = sccp(&p);
        // Jnz→Jmp, then-block removed
        assert_eq!(opt.code.len(), 4);
        assert_eq!(opt.code[1].opcode(), O::Jmp);
        assert_eq!(opt.code[2].opcode(), O::Ldi);
        assert_eq!(opt.code[2].imm_signed(), 99);
    }

    #[test]
    fn sccp_constant_jz_fallthrough() {
        // Jz with r0=0 → always jumps to target, then-block removed
        let p = make_prog_entry(
            vec![
                I::rri(O::Ldi, 0, 0, 0),  // 0
                I::rri(O::Jz, 0, 0, 2),   // 1: r0==0 → jump to 4
                I::rri(O::Ldi, 1, 0, 10), // 2: then (dead)
                I::single(O::Ret),        // 3
                I::rri(O::Ldi, 1, 0, 99), // 4: else
                I::single(O::Ret),        // 5
            ],
            &[0],
        );
        let opt = sccp(&p);
        // Jz→Jmp, then-block removed
        assert_eq!(opt.code.len(), 4);
        assert_eq!(opt.code[1].opcode(), O::Jmp);
        assert_eq!(opt.code[2].imm_signed(), 99);
    }

    #[test]
    fn sccp_propagates_across_blocks() {
        // Block A: r0 = 10, Jmp B
        // Block B: r1 = r0 + 5  → folds to Ldi r1, 15
        let p = make_prog_entry(
            vec![
                I::rri(O::Ldi, 0, 0, 10), // 0
                I::rri(O::Jmp, 0, 0, 1),  // 1: Jmp to 3
                I::single(O::Ret),        // 2: never reached
                I::rri(O::AddI, 1, 0, 5), // 3: r1 = r0 + 5 → 15
                I::single(O::Ret),        // 4
            ],
            &[0],
        );
        let opt = sccp(&p);
        // After const_fold + cfg + sccp:
        // AddI folds to Ldi r1,15
        assert_eq!(opt.code[2].opcode(), O::Ldi);
        assert_eq!(opt.code[2].imm_signed(), 15);
    }

    #[test]
    fn sccp_pipeline_folds_known_branches() {
        // Full pipeline: if(true) { r2 = r1 + r1 } else { r2 = 0 }
        // SCCP sees r0=1 → Jz not taken → else block eliminated → r2=84 folded
        let mut p = make_prog_entry(
            vec![
                I::rri(O::Ldi, 0, 0, 1),  // 0: r0 = 1
                I::rri(O::Ldi, 1, 0, 42), // 1: r1 = 42
                I::rri(O::Jz, 0, 0, 2),   // 2: if r0 == 0 → jump to 5 (else)
                I::rrr(O::Add, 2, 1, 1),  // 3: r2 = r1 + r1 = 84
                I::rri(O::Jmp, 0, 0, 1),  // 4: Jmp to 6 (end)
                I::rri(O::Ldi, 2, 0, 0),  // 5: else: r2 = 0
                I::single(O::Ret),        // 6: end
            ],
            &[0],
        );
        optimize(&mut p);
        // After all passes: Ldi r0=1, Ldi r1=42, Ldi r2=84, Jmp(0), Ret
        // Jmp(0) is residual from if/else → end jump (no-op after else removed)
        assert_eq!(p.code.len(), 5);
        assert_eq!(p.code[2].imm_signed(), 84);
        assert_eq!(p.code[4].opcode(), O::Ret);
    }
}
