use crate::ir::{ConstEntry, QfrProgram};
use crate::opcodes::{Instruction, Opcode};
use std::collections::HashMap;

const INT_REGS: usize = 192;
const FLOAT_REGS: usize = 64;
const PERSIST_SLOTS: usize = 64;

fn sanitize_f(val: f64) -> f64 {
    if val.is_nan() || val.is_infinite() { 0.0 } else { val }
}

#[derive(Debug, Clone, Copy)]
pub struct PersistSlot {
    pub tag: u8,  // 0=i64, 1=f64
    pub int_val: i64,
    pub float_val: f64,
}

impl Default for PersistSlot {
    fn default() -> Self {
        PersistSlot { tag: 0, int_val: 0, float_val: 0.0 }
    }
}

#[derive(Debug)]
pub struct Vm {
    pub program: QfrProgram,

    // Register files
    pub int_regs: [i64; INT_REGS],
    pub float_regs: [f64; FLOAT_REGS],

    // Persistent state (survives hot-reload)
    pub persist: [PersistSlot; PERSIST_SLOTS],

    // Execution state
    pub pc: usize,
    pub call_stack: Vec<usize>,
    pub running: bool,

    // Latest indicator values (pushed before entry calls)
    pub indicators: std::collections::HashMap<String, f64>,

    // Trade entry convention registers (pre-loaded before on_trade)
    #[allow(dead_code)]
    trade_price: f64,
    #[allow(dead_code)]
    trade_qty: f64,
    #[allow(dead_code)]
    trade_side: i64,
    #[allow(dead_code)]
    trade_id: i64,
    #[allow(dead_code)]
    trade_time: i64,

    pub last_price: f64,
    pub position_size: f64,
    pub balances: HashMap<String, f64>,

    // Depth data (for on_depth callback)
    pub depth_bids: Vec<(f64, f64)>,
    pub depth_asks: Vec<(f64, f64)>,

    // Rolling windows (lazily initialized, max 64)
    pub windows: Vec<Option<crate::window::RollingWindow>>,

    // Optional profiler
    pub profiler: Option<crate::profiler::Profiler>,
}

/// A complete snapshot of VM state for replay and hot-reload.
#[derive(Debug, Clone)]
pub struct VmSnapshot {
    pub int_regs: [i64; INT_REGS],
    pub float_regs: [f64; FLOAT_REGS],
    pub persist: [PersistSlot; PERSIST_SLOTS],
    pub pc: usize,
    /// Captured window contents as Vec<f64>.
    pub windows: Vec<Option<Vec<f64>>>,
    pub indicators: std::collections::HashMap<String, f64>,
}

impl Vm {
    pub fn new(program: QfrProgram) -> Self {
        Vm {
            program,
            int_regs: [0i64; INT_REGS],
            float_regs: [0.0f64; FLOAT_REGS],
            persist: [PersistSlot::default(); PERSIST_SLOTS],
            pc: 0,
            call_stack: Vec::new(),
            running: false,
            indicators: std::collections::HashMap::new(),
            trade_price: 0.0,
            trade_qty: 0.0,
            trade_side: 0,
            trade_id: 0,
            trade_time: 0,
            last_price: 0.0,
            position_size: 0.0,
            balances: HashMap::new(),
            depth_bids: Vec::new(),
            depth_asks: Vec::new(),
            windows: (0..64).map(|_| None).collect(),
            profiler: None,
        }
    }

    pub fn set_balance(&mut self, asset: &str, val: f64) {
        self.balances.insert(asset.to_string(), val);
    }

    pub fn set_last_price(&mut self, price: f64) {
        self.last_price = price;
    }

    pub fn set_position_size(&mut self, size: f64) {
        self.position_size = size;
    }

    pub fn set_depth_bids(&mut self, bids: Vec<(f64, f64)>) {
        self.depth_bids = bids;
    }

    pub fn set_depth_asks(&mut self, asks: Vec<(f64, f64)>) {
        self.depth_asks = asks;
    }

    pub fn set_indicator(&mut self, name: &str, val: f64) {
        self.indicators.insert(name.to_string(), val);
    }

    /// Execute an entry point function (e.g. "on_trade", "on_eval")
    pub fn call(&mut self, entry_name: &str) {
        let offset = match self.program.entry_offset(entry_name) {
            Some(o) => o as usize,
            None => return, // entry point not defined
        };
        if let Some(ref mut p) = self.profiler {
            p.start_handler(entry_name);
        }
        self.pc = offset;
        self.running = true;
        self.call_stack.clear();
        self.execute();
        if let Some(ref mut p) = self.profiler {
            p.end_handler();
        }
    }

    fn execute(&mut self) {
        while self.running && self.pc < self.program.code.len() {
            let instr = self.program.code[self.pc];
            self.pc += 1;
            self.dispatch(instr);
        }
    }

    fn dispatch(&mut self, instr: Instruction) {
        let op = instr.opcode();
        if let Some(ref mut p) = self.profiler {
            p.record_opcode(op);
        }
        let rd = instr.rd();
        let rs1 = instr.rs1();
        let rs2 = instr.rs2();
        let imm = instr.imm_signed();

        match op {
            // Int arithmetic
            Opcode::Add => self.set_int(rd, self.int(rs1).wrapping_add(self.int(rs2))),
            Opcode::Sub => self.set_int(rd, self.int(rs1).wrapping_sub(self.int(rs2))),
            Opcode::Mul => self.set_int(rd, self.int(rs1).wrapping_mul(self.int(rs2))),
            Opcode::Div => {
                let divisor = self.int(rs2);
                if divisor == 0 { self.set_int(rd, 0); }
                else { self.set_int(rd, self.int(rs1) / divisor); }
            }
            Opcode::Mod => {
                let divisor = self.int(rs2);
                if divisor == 0 { self.set_int(rd, 0); }
                else { self.set_int(rd, self.int(rs1) % divisor); }
            }
            Opcode::Neg => self.set_int(rd, self.int(rs1).wrapping_neg()),

            Opcode::AddI => self.set_int(rd, self.int(rs1).wrapping_add(imm as i64)),
            Opcode::SubI => self.set_int(rd, self.int(rs1).wrapping_sub(imm as i64)),
            Opcode::MulI => self.set_int(rd, self.int(rs1).wrapping_mul(imm as i64)),
            Opcode::DivI => {
                let divisor = imm as i64;
                if divisor == 0 { self.set_int(rd, 0); }
                else { self.set_int(rd, self.int(rs1) / divisor); }
            }

            // Float arithmetic (with NaN/Inf sanitization)
            Opcode::FAdd => self.set_float(rd, sanitize_f(self.float(rs1) + self.float(rs2))),
            Opcode::FSub => self.set_float(rd, sanitize_f(self.float(rs1) - self.float(rs2))),
            Opcode::FMul => self.set_float(rd, sanitize_f(self.float(rs1) * self.float(rs2))),
            Opcode::FDiv => {
                let divisor = self.float(rs2);
                if divisor == 0.0 { self.set_float(rd, 0.0); }
                else { self.set_float(rd, sanitize_f(self.float(rs1) / divisor)); }
            }
            Opcode::FNeg => self.set_float(rd, sanitize_f(-self.float(rs1))),

            // Int comparison
            Opcode::Eq => self.set_int(rd, if self.int(rs1) == self.int(rs2) { 1 } else { 0 }),
            Opcode::Ne => self.set_int(rd, if self.int(rs1) != self.int(rs2) { 1 } else { 0 }),
            Opcode::Lt => self.set_int(rd, if self.int(rs1) < self.int(rs2) { 1 } else { 0 }),
            Opcode::Gt => self.set_int(rd, if self.int(rs1) > self.int(rs2) { 1 } else { 0 }),
            Opcode::Le => self.set_int(rd, if self.int(rs1) <= self.int(rs2) { 1 } else { 0 }),
            Opcode::Ge => self.set_int(rd, if self.int(rs1) >= self.int(rs2) { 1 } else { 0 }),

            // Float comparison
            Opcode::FEq => self.set_int(rd, if self.float(rs1) == self.float(rs2) { 1 } else { 0 }),
            Opcode::FNe => self.set_int(rd, if self.float(rs1) != self.float(rs2) { 1 } else { 0 }),
            Opcode::FLt => self.set_int(rd, if self.float(rs1) < self.float(rs2) { 1 } else { 0 }),
            Opcode::FGt => self.set_int(rd, if self.float(rs1) > self.float(rs2) { 1 } else { 0 }),
            Opcode::FLe => self.set_int(rd, if self.float(rs1) <= self.float(rs2) { 1 } else { 0 }),
            Opcode::FGe => self.set_int(rd, if self.float(rs1) >= self.float(rs2) { 1 } else { 0 }),

            // Immediate comparisons
            Opcode::EqI => self.set_int(rd, if self.int(rs1) == imm as i64 { 1 } else { 0 }),
            Opcode::LtI => self.set_int(rd, if self.int(rs1) < imm as i64 { 1 } else { 0 }),
            Opcode::GtI => self.set_int(rd, if self.int(rs1) > imm as i64 { 1 } else { 0 }),

            // Bitwise
            Opcode::BitAnd => self.set_int(rd, self.int(rs1) & self.int(rs2)),
            Opcode::BitOr => self.set_int(rd, self.int(rs1) | self.int(rs2)),
            Opcode::BitXor => self.set_int(rd, self.int(rs1) ^ self.int(rs2)),
            Opcode::BitNot => self.set_int(rd, !self.int(rs1)),
            Opcode::Shl => self.set_int(rd, self.int(rs1).wrapping_shl(self.int(rs2) as u32)),
            Opcode::Shr => self.set_int(rd, (self.int(rs1) as u64).wrapping_shr(self.int(rs2) as u32) as i64),

            // Control flow (use imm_signed — 32-bit signed offset)
            Opcode::Jmp => {
                let target = ((self.pc as i64) + imm as i64) as usize;
                self.pc = target;
            }
            Opcode::Jz => {
                if self.int(rs1) == 0 {
                    let target = ((self.pc as i64) + imm as i64) as usize;
                    self.pc = target;
                }
            }
            Opcode::Jnz => {
                if self.int(rs1) != 0 {
                    let target = ((self.pc as i64) + imm as i64) as usize;
                    self.pc = target;
                }
            }
            Opcode::Call => {
                self.call_stack.push(self.pc);
                let target = ((self.pc as i64) + imm as i64) as usize;
                self.pc = target;
            }
            Opcode::Ret => {
                match self.call_stack.pop() {
                    Some(ret_pc) => self.pc = ret_pc,
                    None => self.running = false,
                }
            }

            // Data movement
            Opcode::Mov => {
                if (rd as usize) >= INT_REGS && (rs1 as usize) >= INT_REGS {
                    self.float_regs[(rd - INT_REGS as u8) as usize] =
                        self.float_regs[(rs1 - INT_REGS as u8) as usize];
                } else if (rd as usize) < INT_REGS && (rs1 as usize) < INT_REGS {
                    self.int_regs[rd as usize] = self.int_regs[rs1 as usize];
                } else if (rd as usize) >= INT_REGS {
                    self.float_regs[(rd - INT_REGS as u8) as usize] = self.int(rs1) as f64;
                } else {
                    self.int_regs[rd as usize] = self.float(rs1) as i64;
                }
            }
            Opcode::Ldi => self.set_int(rd, imm as i64),
            Opcode::Ldi64 => {
                let val = instr.imm40();
                if rd >= INT_REGS as u8 {
                    self.float_regs[(rd - INT_REGS as u8) as usize] = val as f64;
                } else {
                    self.int_regs[rd as usize] = val;
                }
            }
            Opcode::Ldc => {
                let idx = imm as usize;
                if idx < self.program.const_pool.len() {
                    match &self.program.const_pool[idx] {
                        ConstEntry::I64(v) => self.set_int(rd, *v),
                        ConstEntry::F64(v) => self.set_float(rd, *v),
                        ConstEntry::String(_) => {
                            self.int_regs[rd as usize] = idx as i64;
                        }
                    }
                }
            }

            // Type conversion
            Opcode::I2F => {
                let val = self.int(rs1) as f64;
                self.set_float(rd, val);
            }
            Opcode::F2I => {
                let val = self.float(rs1) as i64;
                self.set_int(rd, val);
            }

            // Engine builtins
            Opcode::GetInd => {
                let name_idx = self.int(rs1) as usize;
                let name = match self.program.const_pool.get(name_idx) {
                    Some(ConstEntry::String(s)) => s.clone(),
                    _ => String::new(),
                };
                let val = self.indicators.get(&name).copied().unwrap_or(0.0);
                self.set_float(rd, val);
            }
            Opcode::GetPrice => self.set_float(rd, self.last_price),
            Opcode::GetPos => self.set_float(rd, self.position_size),
            Opcode::GetBal => {
                let name_idx = self.int(rs1) as usize;
                let name = match self.program.const_pool.get(name_idx) {
                    Some(ConstEntry::String(s)) => s.clone(),
                    _ => String::new(),
                };
                let bal = self.balances.get(&name).copied().unwrap_or(0.0);
                self.set_float(rd, bal);
            }
            Opcode::GetDepthBid => {
                let level = self.int(rs1) as usize;
                if level < self.depth_bids.len() {
                    self.set_float(rd, self.depth_bids[level].1);
                }
            }
            Opcode::GetDepthAsk => {
                let level = self.int(rs1) as usize;
                if level < self.depth_asks.len() {
                    self.set_float(rd, self.depth_asks[level].1);
                }
            }
            Opcode::SendOrder => {
                let _side = self.int(250);
                let _qty = self.float(192);
                let _price = self.float(193);
                tracing::info!("QFL: SEND_ORDER side={} qty={} price={} type={} reduce={}",
                    _side, _qty, _price,
                    self.int(253),
                    self.int(254),
                );
            }
            Opcode::PersistGet => {
                let slot = imm as usize;
                if slot < PERSIST_SLOTS {
                    let ps = &self.persist[slot];
                    if ps.tag == 0 {
                        self.set_int(rd, ps.int_val);
                    } else {
                        self.set_float(rd, ps.float_val);
                    }
                }
            }
            Opcode::PersistSet => {
                let slot = imm as usize;
                if slot < PERSIST_SLOTS {
                    let ps = &mut self.persist[slot];
                    if rd >= INT_REGS as u8 {
                        ps.tag = 1;
                        ps.float_val = self.float_regs[(rd - INT_REGS as u8) as usize];
                    } else {
                        ps.tag = 0;
                        ps.int_val = self.int_regs[rd as usize];
                    }
                }
            }
            Opcode::Log => {
                let idx = self.int(rs1) as usize;
                let msg = match self.program.const_pool.get(idx) {
                    Some(ConstEntry::String(s)) => s.as_str(),
                    _ => "",
                };
                tracing::info!("QFL: {}", msg);
            }
            Opcode::WindowPush => {
                let wid = imm as usize;
                if wid < self.windows.len() {
                    let val = self.float(rs1);
                    if self.windows[wid].is_none() {
                        self.windows[wid] = Some(crate::window::RollingWindow::new(64));
                    }
                    if let Some(ref mut w) = self.windows[wid] {
                        w.push(val);
                        self.set_float(rd, val);
                    }
                }
            }
            Opcode::WindowMean => {
                let wid = imm as usize;
                if wid < self.windows.len() {
                    if let Some(ref w) = self.windows[wid] {
                        self.set_float(rd, w.mean());
                    }
                }
            }
            Opcode::WindowStddev => {
                let wid = imm as usize;
                if wid < self.windows.len() {
                    if let Some(ref w) = self.windows[wid] {
                        self.set_float(rd, w.stddev());
                    }
                }
            }
            Opcode::WindowMin => {
                let wid = imm as usize;
                if wid < self.windows.len() {
                    if let Some(ref w) = self.windows[wid] {
                        self.set_float(rd, w.min());
                    }
                }
            }
            Opcode::WindowMax => {
                let wid = imm as usize;
                if wid < self.windows.len() {
                    if let Some(ref w) = self.windows[wid] {
                        self.set_float(rd, w.max());
                    }
                }
            }
            Opcode::WindowSum => {
                let wid = imm as usize;
                if wid < self.windows.len() {
                    if let Some(ref w) = self.windows[wid] {
                        self.set_float(rd, w.sum());
                    }
                }
            }
            Opcode::Halt => {
                self.running = false;
            }
            Opcode::MaxOpcode => unreachable!(),
        }
    }

    // ── Snapshot / Replay ──

    /// Save full VM state for replay or hot-reload.
    pub fn snapshot(&self) -> VmSnapshot {
        let windows: Vec<Option<Vec<f64>>> = self.windows.iter().map(|w| {
            w.as_ref().map(|win| {
                let mut vals = Vec::with_capacity(win.len());
                for i in 0..win.len() {
                    vals.push(win.get(i).unwrap_or(0.0));
                }
                vals
            })
        }).collect();

        VmSnapshot {
            int_regs: self.int_regs,
            float_regs: self.float_regs,
            persist: self.persist,
            pc: self.pc,
            windows,
            indicators: self.indicators.clone(),
        }
    }

    /// Restore VM state from a snapshot.
    pub fn restore(&mut self, snap: &VmSnapshot) {
        self.int_regs = snap.int_regs;
        self.float_regs = snap.float_regs;
        self.persist = snap.persist;
        self.pc = snap.pc;
        self.indicators = snap.indicators.clone();
        for (i, win_data) in snap.windows.iter().enumerate() {
            if let Some(vals) = win_data {
                let mut w = crate::window::RollingWindow::new(vals.len());
                for &v in vals {
                    w.push(v);
                }
                self.windows[i] = Some(w);
            } else {
                self.windows[i] = None;
            }
        }
    }

    // Helper: read int register (rd < 192) or return 0
    pub fn int(&self, reg: u8) -> i64 {
        if (reg as usize) < INT_REGS {
            self.int_regs[reg as usize]
        } else {
            self.float_regs[(reg - INT_REGS as u8) as usize] as i64
        }
    }

    // Helper: read float register (rd >= 192) or convert from int
    pub fn float(&self, reg: u8) -> f64 {
        if (reg as usize) >= INT_REGS {
            self.float_regs[(reg - INT_REGS as u8) as usize]
        } else {
            self.int_regs[reg as usize] as f64
        }
    }

    // Helper: set int register
    fn set_int(&mut self, reg: u8, val: i64) {
        if (reg as usize) < INT_REGS {
            self.int_regs[reg as usize] = val;
        } else {
            self.float_regs[(reg - INT_REGS as u8) as usize] = val as f64;
        }
    }

    // Helper: set float register
    fn set_float(&mut self, reg: u8, val: f64) {
        if (reg as usize) >= INT_REGS {
            self.float_regs[(reg - INT_REGS as u8) as usize] = val;
        } else {
            self.int_regs[reg as usize] = val as i64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{EntryPoint, QfrProgram};
    use crate::opcodes::Instruction;

    fn make_prog(code: Vec<Instruction>) -> QfrProgram {
        let mut prog = QfrProgram::new();
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = code;
        prog
    }

    // ── Int arithmetic ──

    macro_rules! test_int_arith {
        ($name:ident, $op:expr, $a:expr, $b:expr, $expected:expr) => {
            #[test]
            fn $name() {
                let mut vm = Vm::new(make_prog(vec![
                    Instruction::rri(Opcode::Ldi, 0, 0, $a as u32),
                    Instruction::rri(Opcode::Ldi, 1, 0, $b as u32),
                    Instruction::rrr($op, 2, 0, 1),
                    Instruction::single(Opcode::Halt),
                ]));
                vm.call("main");
                assert_eq!(vm.int_regs[2], $expected);
            }
        };
    }

    test_int_arith!(adds_two_positive_ints, Opcode::Add, 42, 58, 100);
    test_int_arith!(subtracts_smaller_from_larger_int, Opcode::Sub, 100, 30, 70);
    test_int_arith!(subtracts_larger_from_smaller_int_returns_negative, Opcode::Sub, 10, 50, -40);
    test_int_arith!(multiplies_two_positive_ints, Opcode::Mul, 7, 8, 56);
    #[test]
    fn multiplies_positive_by_negative_int() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, (-3i32) as u32),
            Instruction::rri(Opcode::Ldi, 1, 0, 4),
            Instruction::rrr(Opcode::Mul, 2, 0, 1),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[2], -12);
    }

    #[test]
    fn jnz_loop_counts_down_from_three_to_zero() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 3),       // [0] r0 = 3
            Instruction::rri(Opcode::AddI, 0, 0, (-1i32) as u32), // [1] r0 -= 1
            Instruction::rri(Opcode::Jnz, 0, 0, (-2i32) as u32), // [2] if r0 != 0 jump back to [1]
            Instruction::single(Opcode::Halt),             // [3] halt
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[0], 0);
    }

    #[test]
    fn mod_with_negative_dividend_returns_negative_remainder() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, (-100i32) as u32),
            Instruction::rri(Opcode::Ldi, 1, 0, 3),
            Instruction::rrr(Opcode::Mod, 2, 0, 1),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[2], -1);
    }

    macro_rules! test_int_neg {
        ($name:ident, $val:expr, $expected:expr) => {
            #[test]
            fn $name() {
                let mut vm = Vm::new(make_prog(vec![
                    Instruction::rri(Opcode::Ldi, 0, 0, $val as u32),
                    Instruction::rr(Opcode::Neg, 1, 0),
                    Instruction::single(Opcode::Halt),
                ]));
                vm.call("main");
                assert_eq!(vm.int_regs[1], $expected);
            }
        };
    }

    test_int_neg!(negates_positive_int_to_negative, 42, -42);
    #[test]
    fn negates_negative_int_to_positive() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, (-42i32) as u32),
            Instruction::rr(Opcode::Neg, 1, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[1], 42);
    }
    test_int_neg!(negating_zero_returns_zero, 0, 0);

    macro_rules! test_int_imm {
        ($name:ident, $op:expr, $a:expr, $imm:expr, $expected:expr) => {
            #[test]
            fn $name() {
                let mut vm = Vm::new(make_prog(vec![
                    Instruction::rri(Opcode::Ldi, 0, 0, $a as u32),
                    Instruction::rri($op, 1, 0, $imm as u32),
                    Instruction::single(Opcode::Halt),
                ]));
                vm.call("main");
                assert_eq!(vm.int_regs[1], $expected);
            }
        };
    }

    test_int_imm!(addi_adds_immediate_to_int, Opcode::AddI, 10, 5, 15);
    test_int_imm!(subi_subtracts_immediate_from_int, Opcode::SubI, 10, 5, 5);
    test_int_imm!(muli_multiplies_int_by_immediate, Opcode::MulI, 10, 5, 50);
    test_int_imm!(divi_divides_int_by_immediate_truncates, Opcode::DivI, 100, 3, 33);

    // ── Float arithmetic ──

    macro_rules! test_float_arith {
        ($name:ident, $op:expr, $a:expr, $b:expr, $expected:expr) => {
            #[test]
            fn $name() {
                let mut prog = QfrProgram::new();
                let fa = prog.intern_f64($a);
                let fb = prog.intern_f64($b);
                prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
                prog.code = vec![
                    Instruction::rri(Opcode::Ldc, 192, 0, fa),
                    Instruction::rri(Opcode::Ldc, 193, 0, fb),
                    Instruction::rrr($op, 194, 192, 193),
                    Instruction::single(Opcode::Halt),
                ];
                let mut vm = Vm::new(prog);
                vm.call("main");
                assert!((vm.float_regs[2] - $expected).abs() < 0.001);
            }
        };
    }

    test_float_arith!(fadd_adds_two_floats, Opcode::FAdd, 10.5, 20.5, 31.0);
    test_float_arith!(fsub_subtracts_float_from_float, Opcode::FSub, 100.0, 30.5, 69.5);
    test_float_arith!(fmul_multiplies_two_floats, Opcode::FMul, 3.0, 4.5, 13.5);
    test_float_arith!(fdiv_divides_float_by_float, Opcode::FDiv, 10.0, 3.0, 3.333_333);

    macro_rules! test_fneg {
        ($name:ident, $val:expr, $expected:expr) => {
            #[test]
            fn $name() {
                let mut prog = QfrProgram::new();
                let fv = prog.intern_f64($val);
                prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
                prog.code = vec![
                    Instruction::rri(Opcode::Ldc, 192, 0, fv),
                    Instruction::rr(Opcode::FNeg, 193, 192),
                    Instruction::single(Opcode::Halt),
                ];
                let mut vm = Vm::new(prog);
                vm.call("main");
                assert!((vm.float_regs[1] - $expected).abs() < 0.001);
            }
        };
    }

    test_fneg!(fneg_negates_positive_float, 10.0, -10.0);
    test_fneg!(fneg_negates_negative_float, -10.0, 10.0);

    // ── Int comparison ──

    macro_rules! test_int_cmp {
        ($name:ident, $op:expr, $a:expr, $b:expr, $expected:expr) => {
            #[test]
            fn $name() {
                let mut vm = Vm::new(make_prog(vec![
                    Instruction::rri(Opcode::Ldi, 0, 0, $a as u32),
                    Instruction::rri(Opcode::Ldi, 1, 0, $b as u32),
                    Instruction::rrr($op, 2, 0, 1),
                    Instruction::single(Opcode::Halt),
                ]));
                vm.call("main");
                assert_eq!(vm.int_regs[2], $expected);
            }
        };
    }

    test_int_cmp!(eq_returns_one_when_ints_equal, Opcode::Eq, 5, 5, 1);
    test_int_cmp!(eq_returns_zero_when_ints_not_equal, Opcode::Eq, 5, 6, 0);
    test_int_cmp!(ne_returns_one_when_ints_not_equal, Opcode::Ne, 5, 6, 1);
    test_int_cmp!(ne_returns_zero_when_ints_equal, Opcode::Ne, 5, 5, 0);
    test_int_cmp!(lt_returns_one_when_first_less_than_second, Opcode::Lt, 3, 7, 1);
    test_int_cmp!(lt_returns_zero_when_first_greater_than_second, Opcode::Lt, 7, 3, 0);
    test_int_cmp!(gt_returns_one_when_first_greater_than_second, Opcode::Gt, 7, 3, 1);
    test_int_cmp!(gt_returns_zero_when_first_less_than_second, Opcode::Gt, 3, 7, 0);
    test_int_cmp!(le_returns_one_when_equal, Opcode::Le, 5, 5, 1);
    test_int_cmp!(le_returns_zero_when_greater, Opcode::Le, 6, 5, 0);
    test_int_cmp!(ge_returns_one_when_equal, Opcode::Ge, 5, 5, 1);
    test_int_cmp!(ge_returns_zero_when_less, Opcode::Ge, 4, 5, 0);

    // ── Float comparison ──

    macro_rules! test_float_cmp {
        ($name:ident, $op:expr, $a:expr, $b:expr, $expected:expr) => {
            #[test]
            fn $name() {
                let mut prog = QfrProgram::new();
                let fa = prog.intern_f64($a);
                let fb = prog.intern_f64($b);
                prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
                prog.code = vec![
                    Instruction::rri(Opcode::Ldc, 192, 0, fa),
                    Instruction::rri(Opcode::Ldc, 193, 0, fb),
                    Instruction::rrr($op, 2, 192, 193),
                    Instruction::single(Opcode::Halt),
                ];
                let mut vm = Vm::new(prog);
                vm.call("main");
                assert_eq!(vm.int_regs[2], $expected);
            }
        };
    }

    test_float_cmp!(feq_returns_one_when_floats_equal, Opcode::FEq, 5.0, 5.0, 1);
    test_float_cmp!(feq_returns_zero_when_floats_not_equal, Opcode::FEq, 5.0, 6.0, 0);
    test_float_cmp!(fne_returns_one_when_floats_not_equal, Opcode::FNe, 5.0, 6.0, 1);
    test_float_cmp!(fne_returns_zero_when_floats_equal, Opcode::FNe, 5.0, 5.0, 0);
    test_float_cmp!(flt_returns_one_when_first_float_less, Opcode::FLt, 3.0, 7.0, 1);
    test_float_cmp!(flt_returns_zero_when_first_float_greater, Opcode::FLt, 7.0, 3.0, 0);
    test_float_cmp!(fgt_returns_one_when_first_float_greater, Opcode::FGt, 7.0, 3.0, 1);
    test_float_cmp!(fgt_returns_zero_when_first_float_less, Opcode::FGt, 3.0, 7.0, 0);
    test_float_cmp!(fle_returns_one_when_first_float_less_or_equal, Opcode::FLe, 5.0, 5.0, 1);
    test_float_cmp!(fle_returns_zero_when_first_float_greater, Opcode::FLe, 6.0, 5.0, 0);
    test_float_cmp!(fge_returns_one_when_first_float_greater_or_equal, Opcode::FGe, 5.0, 5.0, 1);
    test_float_cmp!(fge_returns_zero_when_first_float_less, Opcode::FGe, 4.0, 5.0, 0);

    // ── Immediate comparison ──

    macro_rules! test_imm_cmp {
        ($name:ident, $op:expr, $a:expr, $imm:expr, $expected:expr) => {
            #[test]
            fn $name() {
                let mut vm = Vm::new(make_prog(vec![
                    Instruction::rri(Opcode::Ldi, 0, 0, $a as u32),
                    Instruction::rri($op, 1, 0, $imm as u32),
                    Instruction::single(Opcode::Halt),
                ]));
                vm.call("main");
                assert_eq!(vm.int_regs[1], $expected);
            }
        };
    }

    test_imm_cmp!(eqi_returns_one_when_int_equals_immediate, Opcode::EqI, 5, 5, 1);
    test_imm_cmp!(eqi_returns_zero_when_int_not_equals_immediate, Opcode::EqI, 5, 6, 0);
    test_imm_cmp!(lti_returns_one_when_int_less_than_immediate, Opcode::LtI, 3, 7, 1);
    test_imm_cmp!(lti_returns_zero_when_int_greater_than_immediate, Opcode::LtI, 7, 3, 0);
    test_imm_cmp!(gti_returns_one_when_int_greater_than_immediate, Opcode::GtI, 7, 3, 1);
    test_imm_cmp!(gti_returns_zero_when_int_less_than_immediate, Opcode::GtI, 3, 7, 0);

    // ── Bitwise ──

    macro_rules! test_bitwise {
        ($name:ident, $op:expr, $a:expr, $b:expr, $expected:expr) => {
            #[test]
            fn $name() {
                let mut vm = Vm::new(make_prog(vec![
                    Instruction::rri(Opcode::Ldi, 0, 0, $a as u32),
                    Instruction::rri(Opcode::Ldi, 1, 0, $b as u32),
                    Instruction::rrr($op, 2, 0, 1),
                    Instruction::single(Opcode::Halt),
                ]));
                vm.call("main");
                assert_eq!(vm.int_regs[2], $expected);
            }
        };
    }

    test_bitwise!(bitand_and_two_bitmasks, Opcode::BitAnd, 0b1100, 0b1010, 0b1000);
    test_bitwise!(bitor_or_two_bitmasks, Opcode::BitOr, 0b1100, 0b1010, 0b1110);
    test_bitwise!(bitxor_xor_two_bitmasks, Opcode::BitXor, 0b1100, 0b1010, 0b0110);

    #[test]
    fn bitnot_inverts_all_bits() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 0x0f0f_0f0f),
            Instruction::rr(Opcode::BitNot, 1, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[1], !0x0f0f_0f0f);
    }

    test_bitwise!(shl_shifts_left_by_eight, Opcode::Shl, 1, 8, 256);
    test_bitwise!(shr_shifts_right_by_eight, Opcode::Shr, 256, 8, 1);

    // ── Control flow ──

    #[test]
    fn call_fallthrough_without_ret_executes_callee_then_halt() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 1),     // [0] r0 = 1
            Instruction::rri(Opcode::Call, 0, 0, 1),    // [1] call fn -> target=2+1=3
            Instruction::single(Opcode::Halt),           // [2] halt
            Instruction::rri(Opcode::Ldi, 1, 0, 42),    // [3] fn body
            Instruction::single(Opcode::Halt),           // [4] halt
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[1], 42);
    }

    #[test]
    fn jmp_backward_with_jnz_loops_from_zero_to_ten() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 0),     // [0] r0 = 0
            Instruction::rri(Opcode::AddI, 0, 0, 1),    // [1] r0 += 1
            Instruction::rri(Opcode::Ldi, 1, 0, 10),    // [2] r1 = 10
            Instruction::rrr(Opcode::Eq, 2, 0, 1),      // [3] r2 = (r0 == r1)
            Instruction::rri(Opcode::Jnz, 0, 2, 1),     // [4] if r2 != 0, skip 1 → Halt
            Instruction::rri(Opcode::Jmp, 0, 0, (-5i32) as u32), // [5] back to [1]
            Instruction::single(Opcode::Halt),           // [6]
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[0], 10);
    }

    macro_rules! test_jz {
        ($name:ident, $val:expr, $r1_expected:expr) => {
            #[test]
            fn $name() {
                let mut vm = Vm::new(make_prog(vec![
                    Instruction::rri(Opcode::Ldi, 0, 0, $val as u32),
                    Instruction::rri(Opcode::Jz, 0, 0, 2),
                    Instruction::rri(Opcode::Ldi, 1, 0, 99),
                    Instruction::single(Opcode::Halt),
                ]));
                vm.call("main");
                assert_eq!(vm.int_regs[1], $r1_expected);
            }
        };
    }

    test_jz!(jz_not_taken_when_register_nonzero, 1, 99);
    test_jz!(jz_taken_when_register_is_zero, 0, 0);

    macro_rules! test_jnz {
        ($name:ident, $val:expr, $r1_expected:expr) => {
            #[test]
            fn $name() {
                let mut vm = Vm::new(make_prog(vec![
                    Instruction::rri(Opcode::Ldi, 0, 0, $val as u32),
                    Instruction::rri(Opcode::Jnz, 0, 0, 2),
                    Instruction::rri(Opcode::Ldi, 1, 0, 99),
                    Instruction::single(Opcode::Halt),
                ]));
                vm.call("main");
                assert_eq!(vm.int_regs[1], $r1_expected);
            }
        };
    }

    test_jnz!(jnz_taken_when_register_nonzero, 1, 0);
    test_jnz!(jnz_not_taken_when_register_zero, 0, 99);

    #[test]
    fn call_ret_preserves_caller_registers_and_resumes_after_call() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 42),     // [0]
            Instruction::rri(Opcode::Call, 0, 0, 2),      // [1] → target = 2+2=4
            Instruction::rri(Opcode::Ldi, 1, 0, 99),      // [2]
            Instruction::single(Opcode::Halt),             // [3]
            Instruction::rri(Opcode::Ldi, 2, 0, 7),       // [4] fn body
            Instruction::single(Opcode::Ret),              // [5] → pops pc=2
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[0], 42);
        assert_eq!(vm.int_regs[1], 99);
        assert_eq!(vm.int_regs[2], 7);
    }

    #[test]
    fn nested_call_adds_one_then_multiplies_by_two() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 1),     // [0]
            Instruction::rri(Opcode::Call, 0, 0, 1),    // [1] → target=2+1=3 (fn_a)
            Instruction::single(Opcode::Halt),           // [2]
            Instruction::rri(Opcode::AddI, 0, 0, 1),    // [3] fn_a: r0 += 1
            Instruction::rri(Opcode::Call, 0, 0, 1),    // [4] → target=5+1=6 (fn_b)
            Instruction::single(Opcode::Ret),            // [5] ret from fn_a → pop pc=2
            Instruction::rri(Opcode::MulI, 0, 0, 2),    // [6] fn_b: r0 *= 2
            Instruction::single(Opcode::Ret),            // [7] ret from fn_b → pop pc=5
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[0], 4); // (1+1)*2 = 4
    }

    // ── Data movement ──

    macro_rules! test_ldi {
        ($name:ident, $val:expr, $expected:expr) => {
            #[test]
            fn $name() {
                let mut vm = Vm::new(make_prog(vec![
                    Instruction::rri(Opcode::Ldi, 0, 0, $val as u32),
                    Instruction::single(Opcode::Halt),
                ]));
                vm.call("main");
                assert_eq!(vm.int_regs[0], $expected);
            }
        };
    }

    test_ldi!(ldi_loads_zero, 0, 0);
    test_ldi!(ldi_loads_positive_int, 42, 42);
    test_ldi!(ldi_loads_max_32bit_signed, 0x7fff_ffff, 0x7fff_ffff);

    #[test]
    fn ldi_loads_negative_int_via_two_complement() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, (-1i32) as u32),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[0], -1i64);
    }

    #[test]
    fn ldi64_loads_40bit_positive_value() {
        let big: i64 = 0x7f_1234_5678; // fits in 40-bit signed
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri40(Opcode::Ldi64, 0, big),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[0], big);
    }

    #[test]
    fn ldi64_loads_negative_one_as_40bit_signed() {
        let big: i64 = -1;
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri40(Opcode::Ldi64, 0, big),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[0], big);
    }

    #[test]
    fn ldi64_loads_small_negative_40bit_value() {
        let big: i64 = -42;
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri40(Opcode::Ldi64, 0, big),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[0], big);
    }

    macro_rules! test_mov {
        ($name:ident, $from:expr, $to:expr, $val:expr, $expected:expr) => {
            #[test]
            fn $name() {
                let mut vm = Vm::new(make_prog(vec![
                    Instruction::rri(Opcode::Ldi, $from, 0, $val as u32),
                    Instruction::rr(Opcode::Mov, $to, $from),
                    Instruction::single(Opcode::Halt),
                ]));
                vm.call("main");
                assert_eq!(vm.int_regs[$to as usize], $expected);
            }
        };
    }

    test_mov!(mov_copies_int_from_one_register_to_another, 0, 1, 42, 42);
    test_mov!(mov_copies_zero_between_registers, 0, 1, 0, 0);

    #[test]
    fn mov_copies_int_value_to_float_register() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 42),
            Instruction::rr(Opcode::Mov, 192, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert!((vm.float_regs[0] - 42.0).abs() < 0.001);
    }

    // Ldc tests
    macro_rules! test_ldc {
        ($name:ident, $ctor:ident, $val:expr) => {
            #[test]
            fn $name() {
                let mut prog = QfrProgram::new();
                let idx = prog.$ctor($val);
                prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
                prog.code = vec![
                    Instruction::rri(Opcode::Ldc, 0, 0, idx),
                    Instruction::single(Opcode::Halt),
                ];
                let mut vm = Vm::new(prog);
                vm.call("main");
            }
        };
    }

    test_ldc!(ldc_loads_i64_constant, intern_i64, 42i64);
    test_ldc!(ldc_loads_f64_constant, intern_f64, 3.14f64);
    test_ldc!(ldc_loads_string_constant, intern_string, "hello");

    // ── Type conversion ──

    #[test]
    fn i2f_converts_positive_int_to_float() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 42),
            Instruction::rr(Opcode::I2F, 192, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert!((vm.float_regs[0] - 42.0).abs() < 0.001);
    }

    #[test]
    fn f2i_truncates_float_to_int() {
        let mut prog = QfrProgram::new();
        let fv = prog.intern_f64(42.7);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 192, 0, fv),
            Instruction::rr(Opcode::F2I, 0, 192),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert_eq!(vm.int_regs[0], 42);
    }

    // ── Builtins ──

    #[test]
    fn getind_returns_indicator_value_when_name_exists() {
        let mut prog = QfrProgram::new();
        let name_idx = prog.intern_string("ema");
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 0, 0, name_idx),
            Instruction::rr(Opcode::GetInd, 192, 0),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.set_indicator("ema", 123.456);
        vm.call("main");
        assert!((vm.float_regs[0] - 123.456).abs() < 0.001);
    }

    #[test]
    fn getind_returns_zero_when_indicator_name_not_found() {
        let mut prog = QfrProgram::new();
        let name_idx = prog.intern_string("nonexistent");
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 0, 0, name_idx),
            Instruction::rr(Opcode::GetInd, 192, 0),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert!((vm.float_regs[0] - 0.0).abs() < 0.001);
    }

    #[test]
    fn getprice_returns_last_price() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri(Opcode::GetPrice, 192, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.set_last_price(50000.0);
        vm.call("main");
        assert!((vm.float_regs[0] - 50000.0).abs() < 0.001);
    }

    #[test]
    fn getpos_returns_current_position_size() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri(Opcode::GetPos, 192, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.set_position_size(1.5);
        vm.call("main");
        assert!((vm.float_regs[0] - 1.5).abs() < 0.001);
    }

    #[test]
    fn getbal_returns_balance_when_asset_exists() {
        let mut prog = QfrProgram::new();
        let name_idx = prog.intern_string("USDT");
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 0, 0, name_idx),
            Instruction::rr(Opcode::GetBal, 192, 0),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.set_balance("USDT", 10000.0);
        vm.call("main");
        assert!((vm.float_regs[0] - 10000.0).abs() < 0.001);
    }

    #[test]
    fn getbal_returns_zero_when_asset_not_found() {
        let mut prog = QfrProgram::new();
        let name_idx = prog.intern_string("NONE");
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 0, 0, name_idx),
            Instruction::rr(Opcode::GetBal, 192, 0),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert!((vm.float_regs[0] - 0.0).abs() < 0.001);
    }

    #[test]
    fn getdepthbid_returns_volume_at_bid_level() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 0),
            Instruction::rrr(Opcode::GetDepthBid, 192, 0, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.set_depth_bids(vec![(100.0, 1.5)]);
        vm.call("main");
        assert!((vm.float_regs[0] - 1.5).abs() < 0.001);
    }

    #[test]
    fn getdepthask_returns_volume_at_ask_level() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 0),
            Instruction::rrr(Opcode::GetDepthAsk, 192, 0, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.set_depth_asks(vec![(101.0, 2.0)]);
        vm.call("main");
        assert!((vm.float_regs[0] - 2.0).abs() < 0.001);
    }

    #[test]
    fn log_does_not_crash_with_valid_string() {
        let mut prog = QfrProgram::new();
        let msg_idx = prog.intern_string("test log message");
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 0, 0, msg_idx),
            Instruction::rr(Opcode::Log, 1, 0),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        // Just verify it doesn't crash
    }

    #[test]
    fn halt_stops_execution_immediately_skipping_further_instructions() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 42),
            Instruction::single(Opcode::Halt),
            Instruction::rri(Opcode::Ldi, 1, 0, 99),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[0], 42);
        assert_eq!(vm.int_regs[1], 0);
    }

    #[test]
    fn sendorder_does_not_crash_with_buy_side_set() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 250, 0, 0),
            Instruction::single(Opcode::SendOrder),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
    }

    #[test]
    fn persist_set_then_get_returns_same_int_value() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 42),
            Instruction::rri(Opcode::PersistSet, 0, 0, 0),
            Instruction::rri(Opcode::Ldi, 0, 0, 0),
            Instruction::rri(Opcode::PersistGet, 1, 0, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[1], 42);
    }

    #[test]
    fn persist_stores_and_retrieves_float_value() {
        let mut prog = QfrProgram::new();
        let fv = prog.intern_f64(3.14);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 192, 0, fv),
            Instruction::rri(Opcode::PersistSet, 192, 0, 0),
            Instruction::rri(Opcode::PersistGet, 193, 0, 0),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert!((vm.float_regs[1] - 3.14).abs() < 0.001);
    }

    #[test]
    fn persist_int_value_survives_vm_reload() {
        let code1 = vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 100),
            Instruction::rri(Opcode::PersistSet, 0, 0, 0),
            Instruction::single(Opcode::Halt),
        ];
        let mut prog1 = QfrProgram::new();
        prog1.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog1.code = code1;
        let mut vm = Vm::new(prog1);
        vm.call("main");

        let code2 = vec![
            Instruction::rri(Opcode::PersistGet, 1, 0, 0),
            Instruction::single(Opcode::Halt),
        ];
        let mut prog2 = QfrProgram::new();
        prog2.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog2.code = code2;
        let mut vm2 = Vm::new(prog2);
        vm2.persist.copy_from_slice(&vm.persist);
        vm2.call("main");
        assert_eq!(vm2.int_regs[1], 100);
    }

    #[test]
    fn persist_all_64_slots_store_and_read_back_last_slot() {
        let mut code = Vec::new();
        for i in 0..64 {
            code.push(Instruction::rri(Opcode::Ldi, 0, 0, i as u32));
            code.push(Instruction::rri(Opcode::PersistSet, 0, 0, i as u32));
        }
        for i in 0..64 {
            code.push(Instruction::rri(Opcode::PersistGet, 1, 0, i as u32));
        }
        code.push(Instruction::single(Opcode::Halt));

        // Read back after setting all
        let mut vm = Vm::new(make_prog(code.clone()));
        vm.call("main");

        // Now verify in a new VM
        let mut code2: Vec<Instruction> = (0..64)
            .map(|i| Instruction::rri(Opcode::PersistGet, 0, 0, i as u32))
            .collect();
        code2.push(Instruction::single(Opcode::Halt));
        let mut vm2 = Vm::new(make_prog(code2));
        vm2.persist.copy_from_slice(&vm.persist);
        vm2.call("main");
        assert_eq!(vm2.int_regs[0], 63);
    }

    #[test]
    fn call_to_nonexistent_entry_does_not_crash() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("nonexistent");
    }

    #[test]
    fn entry_point_past_code_end_does_not_execute_anything() {
        let mut prog = QfrProgram::new();
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 5 });
        prog.code = vec![
            Instruction::single(Opcode::Halt),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main"); // offset 5 is past code.len(), should just not run
    }

    #[test]
    fn multiplication_of_large_ints_returns_correct_product() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 100000),
            Instruction::rri(Opcode::Ldi, 1, 0, 100000),
            Instruction::rrr(Opcode::Mul, 2, 0, 1),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[2], 10_000_000_000i64);
    }

    #[test]
    fn add_overflow_wraps_around_i64_max() {
        let big = (1i64 << 39) - 1; // max 40-bit signed
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri40(Opcode::Ldi64, 0, big),
            Instruction::rri(Opcode::Ldi, 1, 0, 1),
            Instruction::rrr(Opcode::Add, 2, 0, 1),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[2], big.wrapping_add(1));
    }

    #[test]
    fn hundred_ldi_instructions_last_value_wins() {
        let mut code = Vec::new();
        for i in 0..100 {
            code.push(Instruction::rri(Opcode::Ldi, 0, 0, i));
        }
        code.push(Instruction::single(Opcode::Halt));
        let mut vm = Vm::new(make_prog(code));
        vm.call("main");
        assert_eq!(vm.int_regs[0], 99);
    }

    #[test]
    fn multiple_registers_hold_independent_values() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 10),
            Instruction::rri(Opcode::Ldi, 1, 0, 20),
            Instruction::rri(Opcode::Ldi, 2, 0, 30),
            Instruction::rrr(Opcode::Add, 3, 0, 1),
            Instruction::rrr(Opcode::Add, 4, 1, 2),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[3], 30);
        assert_eq!(vm.int_regs[4], 50);
        assert_eq!(vm.int_regs[0], 10);
    }

    // ── Deep / recursive calls ──

    #[test]
    fn recursive_call_self_fifty_times_returns_zero() {
        // Self-call 50 times using a counter
        let mut prog = QfrProgram::new();
        let _offset = 0i32;
        // [0] ldi r0, 50   (counter)
        // [2] if r0 == 0 -> ret (halt)
        // [4] addi r0, -1
        // [6] call -> target = current + (-6) = back to [2]
        // [7] ret
        // [8] halt
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 8 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 50),                 // [0] r0 = 50
            Instruction::rri(Opcode::EqI, 1, 0, 0),                  // [1] r1 = (r0 == 0)
            Instruction::rri(Opcode::Jnz, 0, 1, 6),                  // [2] if r1 != 0 skip to [8] halt
            Instruction::rri(Opcode::AddI, 0, 0, (-1i32) as u32),    // [3] r0 -= 1
            Instruction::rri(Opcode::Call, 0, 0, (-5i32) as u32),    // [4] call back to [1]
            Instruction::single(Opcode::Ret),                         // [5] ret
            Instruction::single(Opcode::Halt),                        // [6]
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert_eq!(vm.int_regs[0], 0);
    }

    // ── Loop edge cases ──

    #[test]
    fn while_loop_decrements_counter_from_ten_to_zero() {
        // while loop: r0 counts down to 0
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 10),     // [0] r0 = 10
            Instruction::rri(Opcode::Ldi, 1, 0, 0),      // [1] r1 = 0
            Instruction::rrr(Opcode::Eq, 2, 0, 1),       // [2] r2 = (r0 == 0)
            Instruction::rri(Opcode::Jnz, 0, 2, 3),      // [3] if r2 goto halt
            Instruction::rri(Opcode::AddI, 0, 0, (-1i32) as u32), // [4] r0 -= 1
            Instruction::rri(Opcode::Jmp, 0, 0, (-5i32) as u32),  // [5] back to [2]
            Instruction::single(Opcode::Halt),           // [6]
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[0], 0);
    }

    #[test]
    fn repeat_loop_counts_up_to_five_and_stops() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 1, 0, 1),       // [0] r1 = 1 (increment)
            Instruction::rri(Opcode::Ldi, 0, 0, 0),       // [1] r0 = 0
            Instruction::rri(Opcode::AddI, 0, 0, 1),      // [2] r0 += 1
            Instruction::rri(Opcode::EqI, 2, 0, 5),       // [3] r2 = (r0 == 5)
            Instruction::rri(Opcode::Jz, 0, 2, (-3i32) as u32), // [4] if r2 == 0 back to [2]
            Instruction::single(Opcode::Halt),             // [5]
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[0], 5);
    }

    // ── Empty step / empty program ──

    #[test]
    fn empty_program_with_entry_does_not_crash() {
        let mut prog = QfrProgram::new();
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        let mut vm = Vm::new(prog);
        vm.call("main"); // should not crash, no code to execute
    }

    #[test]
    fn entry_point_at_code_boundary_does_not_execute() {
        let mut prog = QfrProgram::new();
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 5 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 10),
            Instruction::rri(Opcode::Ldi, 1, 0, 20),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main"); // offset 5 is past code.len(), should just not run
    }

    #[test]
    fn entry_point_in_middle_of_code_skips_earlier_instructions() {
        let mut prog = QfrProgram::new();
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 2 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 10),  // [0] not reached
            Instruction::rri(Opcode::Ldi, 1, 0, 99),  // [1] skipped
            Instruction::rri(Opcode::Ldi, 2, 0, 42),  // [2] entry point
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert_eq!(vm.int_regs[0], 0);
        assert_eq!(vm.int_regs[1], 0);
        assert_eq!(vm.int_regs[2], 42);
    }

    #[test]
    fn halt_as_only_instruction_does_not_crash() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        // Just verify no crash
    }

    #[test]
    fn first_halt_stops_execution_second_halt_not_reached() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::single(Opcode::Halt),
            Instruction::rri(Opcode::Ldi, 0, 0, 99),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[0], 0); // should stop at first Halt
    }

    // ── Persist edge cases ──

    #[test]
    fn persist_tag_switches_from_int_to_float_when_float_stored() {
        let mut prog = QfrProgram::new();
        let fv = prog.intern_f64(3.14);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 42),
            Instruction::rri(Opcode::PersistSet, 0, 0, 0),  // persist[0] = 42 (int)
            Instruction::rri(Opcode::Ldc, 192, 0, fv),
            Instruction::rri(Opcode::PersistSet, 192, 0, 0), // persist[0] = 3.14 (float)
            Instruction::rri(Opcode::PersistGet, 193, 0, 0), // read back
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert!((vm.float_regs[1] - 3.14).abs() < 0.001);
        assert_eq!(vm.persist[0].tag, 1);
    }

    #[test]
    fn persist_tag_switches_from_float_to_int_when_int_stored() {
        let mut prog = QfrProgram::new();
        let fv = prog.intern_f64(2.71);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 192, 0, fv),
            Instruction::rri(Opcode::PersistSet, 192, 0, 0), // persist[0] = 2.71 (float)
            Instruction::rri(Opcode::Ldi, 0, 0, 99),
            Instruction::rri(Opcode::PersistSet, 0, 0, 0),   // persist[0] = 99 (int)
            Instruction::rri(Opcode::PersistGet, 1, 0, 0),   // read back
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert_eq!(vm.int_regs[1], 99);
        assert_eq!(vm.persist[0].tag, 0);
    }

    #[test]
    fn persist_slot_63_last_slot_stores_and_retrieves_value() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 77),
            Instruction::rri(Opcode::PersistSet, 0, 0, 63),
            Instruction::rri(Opcode::PersistGet, 1, 0, 63),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[1], 77);
    }

    #[test]
    fn persist_slot_zero_initialized_to_zero() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::PersistGet, 1, 0, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[1], 0);
    }

    #[test]
    fn all_persist_slots_start_as_zero_on_fresh_vm() {
        let vm = Vm::new(make_prog(vec![]));
        for slot in vm.persist.iter() {
            assert_eq!(slot.int_val, 0);
            assert_eq!(slot.float_val, 0.0);
            assert_eq!(slot.tag, 0);
        }
    }

    // ── Register file edge cases ──

    #[test]
    fn last_int_register_r191_stores_and_retrieves_value() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 191, 0, 0x7f_ff_ff_ff),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[191], 0x7f_ff_ff_ff);
    }

    #[test]
    fn last_float_register_r255_stores_and_retrieves_value() {
        let mut prog = QfrProgram::new();
        let fv = prog.intern_f64(3.14);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 255, 0, fv),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert!((vm.float_regs[63] - 3.14).abs() < 0.001);
    }

    #[test]
    fn uninitialized_int_register_reads_as_zero() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rrr(Opcode::Add, 2, 50, 51), // uninitialized regs
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[2], 0);
    }

    #[test]
    fn uninitialized_float_register_reads_as_zero() {
        let mut prog = QfrProgram::new();
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rrr(Opcode::FAdd, 220, 200, 201), // uninitialized float regs
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert_eq!(vm.float_regs[28], 0.0);
    }

    // ── Type conversion edge cases ──

    #[test]
    fn f2i_truncates_positive_float_by_dropping_fraction() {
        let mut prog = QfrProgram::new();
        let fv = prog.intern_f64(42.999);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 192, 0, fv),
            Instruction::rr(Opcode::F2I, 0, 192),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert_eq!(vm.int_regs[0], 42);
    }

    #[test]
    fn f2i_truncates_negative_float_toward_zero() {
        let mut prog = QfrProgram::new();
        let fv = prog.intern_f64(-42.7);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 192, 0, fv),
            Instruction::rr(Opcode::F2I, 0, 192),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert_eq!(vm.int_regs[0], -42);
    }

    #[test]
    fn i2f_converts_negative_int_to_float() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, (-1i32) as u32),
            Instruction::rr(Opcode::I2F, 192, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert!((vm.float_regs[0] + 1.0).abs() < 0.001);
    }

    #[test]
    fn i2f_converts_large_int_exceeding_53bit_precision() {
        let mut prog = QfrProgram::new();
        let large = 1i64 << 50;
        let idx = prog.intern_i64(large);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 0, 0, idx),
            Instruction::rr(Opcode::I2F, 192, 0),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert!((vm.float_regs[0] - (large as f64)).abs() < 1.0);
    }

    // ── Ldi64 edge cases ──

    #[test]
    fn ldi64_loads_max_40bit_positive_value() {
        let max40 = (1i64 << 39) - 1;
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri40(Opcode::Ldi64, 0, max40),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[0], max40);
    }

    #[test]
    fn ldi64_loads_min_40bit_negative_value() {
        let min40 = -(1i64 << 39);
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri40(Opcode::Ldi64, 0, min40),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[0], min40);
    }

    #[test]
    fn ldi64_loads_into_float_register() {
        let val: i64 = 12345;
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri40(Opcode::Ldi64, 192, val),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert!((vm.float_regs[0] - 12345.0).abs() < 0.001);
    }

    // ── Bitwise edge cases ──

    #[test]
    fn bitnot_of_negative_one_returns_zero() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri40(Opcode::Ldi64, 0, -1i64),
            Instruction::rr(Opcode::BitNot, 1, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[1], 0);
    }

    #[test]
    fn shl_by_zero_returns_original_value() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 42),
            Instruction::rri(Opcode::Ldi, 1, 0, 0),
            Instruction::rrr(Opcode::Shl, 2, 0, 1),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[2], 42);
    }

    #[test]
    fn shr_by_zero_returns_original_value() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 42),
            Instruction::rri(Opcode::Ldi, 1, 0, 0),
            Instruction::rrr(Opcode::Shr, 2, 0, 1),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[2], 42);
    }

    // ── Builtin edge cases ──

    #[test]
    fn getprice_returns_zero_when_not_set() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri(Opcode::GetPrice, 192, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert!((vm.float_regs[0] - 0.0).abs() < 0.001);
    }

    #[test]
    fn getpos_returns_zero_when_not_set() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri(Opcode::GetPos, 192, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert!((vm.float_regs[0] - 0.0).abs() < 0.001);
    }

    #[test]
    fn getdepthbid_returns_zero_when_level_out_of_range() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 99), // level 99 out of range
            Instruction::rrr(Opcode::GetDepthBid, 192, 0, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.set_depth_bids(vec![(100.0, 1.5)]);
        vm.call("main");
        assert!((vm.float_regs[0] - 0.0).abs() < 0.001);
    }

    #[test]
    fn getdepthask_returns_zero_when_level_out_of_range() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 99),
            Instruction::rrr(Opcode::GetDepthAsk, 192, 0, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.set_depth_asks(vec![(101.0, 2.0)]);
        vm.call("main");
        assert!((vm.float_regs[0] - 0.0).abs() < 0.001);
    }

    #[test]
    fn sendorder_does_not_crash_with_zero_qty() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 250, 0, 0),   // side = buy
            Instruction::single(Opcode::SendOrder),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        // Just verify no crash with zero qty
    }

    #[test]
    fn sendorder_does_not_crash_with_limit_type_and_reduce_only_set() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 250, 0, 0),
            Instruction::rri(Opcode::Ldi, 253, 0, 1),   // type = limit
            Instruction::rri(Opcode::Ldi, 254, 0, 0),   // reduce_only = false
            Instruction::single(Opcode::SendOrder),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
    }

    // ── Mov edge cases ──

    #[test]
    fn mov_to_self_leaves_value_unchanged() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 5, 0, 42),
            Instruction::rr(Opcode::Mov, 5, 5), // mov to self
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[5], 42);
    }

    #[test]
    fn mov_from_float_to_int_truncates_value() {
        let mut prog = QfrProgram::new();
        let fv = prog.intern_f64(99.7);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 192, 0, fv),
            Instruction::rr(Opcode::Mov, 0, 192), // float reg -> int reg (truncates)
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert_eq!(vm.int_regs[0], 99);
    }

    // ── Large constants ──

    #[test]
    fn ldc_handles_1000_character_string_without_crashing() {
        let mut prog = QfrProgram::new();
        let long = "x".repeat(1000);
        let idx = prog.intern_string(&long);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 0, 0, idx),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert_eq!(vm.int_regs[0], idx as i64);
    }

    #[test]
    fn ldc_with_invalid_const_pool_index_does_not_crash() {
        let mut prog = QfrProgram::new();
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 0, 0, 9999), // beyond const pool
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main"); // should not crash
    }

    // ── Many instructions stress test ──

    #[test]
    fn twenty_iterations_of_mul_add_produces_2_pow_20_minus_1() {
        let mut code = Vec::new();
        // r0 = 0; loop 100 times: r0 = r0 * 2 + 1
        code.push(Instruction::rri(Opcode::Ldi, 0, 0, 0));
        for _ in 0..20 {
            code.push(Instruction::rri(Opcode::MulI, 0, 0, 2));
            code.push(Instruction::rri(Opcode::AddI, 0, 0, 1));
        }
        code.push(Instruction::single(Opcode::Halt));
        let mut vm = Vm::new(make_prog(code));
        vm.call("main");
        assert_eq!(vm.int_regs[0], (1i64 << 20) - 1);
    }

    #[test]
    fn all_191_int_registers_hold_values_independently() {
        let mut code = Vec::new();
        for i in 0..191 {
            code.push(Instruction::rri(Opcode::Ldi, i as u8, 0, i));
        }
        code.push(Instruction::single(Opcode::Halt));
        let mut vm = Vm::new(make_prog(code));
        vm.call("main");
        for i in 0..191 {
            assert_eq!(vm.int_regs[i], i as i64);
        }
    }

    #[test]
    fn transitive_lt_chain_works_correctly() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 10),
            Instruction::rri(Opcode::Ldi, 1, 0, 20),
            Instruction::rri(Opcode::Ldi, 2, 0, 30),
            Instruction::rrr(Opcode::Lt, 3, 0, 1),   // 10 < 20 = 1
            Instruction::rrr(Opcode::Lt, 4, 1, 2),   // 20 < 30 = 1
            Instruction::rrr(Opcode::Eq, 5, 3, 4),   // 1 == 1 = 1 (transitive)
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[3], 1);
        assert_eq!(vm.int_regs[4], 1);
        assert_eq!(vm.int_regs[5], 1);
    }

    // ── Div/mod by zero ──

    #[test]
    fn int_div_by_zero_returns_zero() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 42),
            Instruction::rri(Opcode::Ldi, 1, 0, 0),
            Instruction::rrr(Opcode::Div, 2, 0, 1),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[2], 0);
    }

    #[test]
    fn divi_by_zero_immediate_returns_zero() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 42),
            Instruction::rri(Opcode::DivI, 1, 0, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[1], 0);
    }

    #[test]
    fn mod_by_zero_returns_zero() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 42),
            Instruction::rri(Opcode::Ldi, 1, 0, 0),
            Instruction::rrr(Opcode::Mod, 2, 0, 1),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[2], 0);
    }

    #[test]
    fn fdiv_by_zero_returns_zero() {
        let mut prog = QfrProgram::new();
        let fa = prog.intern_f64(42.0);
        let fb = prog.intern_f64(0.0);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 192, 0, fa),
            Instruction::rri(Opcode::Ldc, 193, 0, fb),
            Instruction::rrr(Opcode::FDiv, 194, 192, 193),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert_eq!(vm.float_regs[2], 0.0);
    }

    // ── NaN / Inf sanitization ──

    #[test]
    fn fadd_with_nan_operand_returns_zero() {
        let mut prog = QfrProgram::new();
        let nan = prog.intern_f64(f64::NAN);
        let five = prog.intern_f64(5.0);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 192, 0, nan),
            Instruction::rri(Opcode::Ldc, 193, 0, five),
            Instruction::rrr(Opcode::FAdd, 194, 192, 193),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert_eq!(vm.float_regs[2], 0.0);
    }

    #[test]
    fn fmul_with_infinity_operand_returns_zero() {
        let mut prog = QfrProgram::new();
        let inf = prog.intern_f64(f64::INFINITY);
        let two = prog.intern_f64(2.0);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 192, 0, inf),
            Instruction::rri(Opcode::Ldc, 193, 0, two),
            Instruction::rrr(Opcode::FMul, 194, 192, 193),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert_eq!(vm.float_regs[2], 0.0);
    }

    // ── Recursion / loop protection ──

    #[test]
    fn recursion_depth_of_1000_does_not_overflow_stack() {
        // Recursive calls without Ret: call->call->call, depth counter in r0.
        // When r0 hits 0, jump to Halt instead of calling again.
        let mut code = Vec::new();
        code.push(Instruction::rri(Opcode::Ldi, 0, 0, 1000));           // [0] r0 = 1000
        code.push(Instruction::rri(Opcode::EqI, 1, 0, 0));              // [1] r1 = (r0 == 0)
        code.push(Instruction::rri(Opcode::Jnz, 0, 1, 3));              // [2] if r1 != 0 → [5] Halt
        code.push(Instruction::rri(Opcode::AddI, 0, 0, (-1i32) as u32)); // [3] r0 -= 1
        code.push(Instruction::rri(Opcode::Call, 0, 0, (-4i32) as u32)); // [4] call back to [1]
        code.push(Instruction::single(Opcode::Halt));                    // [5] Halt
        let mut vm = Vm::new(make_prog(code));
        vm.call("main");
        assert_eq!(vm.int_regs[0], 0);
        assert!(vm.call_stack.len() >= 999);
    }

    #[test]
    fn loop_counts_from_zero_to_one_thousand_and_halts() {
        // Count from 0 to 1000 then halt — no infinite loop
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 0),        // [0] r0 = 0
            Instruction::rri(Opcode::Ldi, 1, 0, 1000),     // [1] r1 = 1000
            Instruction::rri(Opcode::AddI, 0, 0, 1),       // [2] r0 += 1
            Instruction::rrr(Opcode::Eq, 2, 0, 1),          // [3] r2 = (r0 == 1000)
            Instruction::rri(Opcode::Jz, 0, 2, (-3i32) as u32), // [4] if r2==0 → [2]
            Instruction::single(Opcode::Halt),             // [5]
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[0], 1000);
    }

    // ── Empty / immediate halt ──

    #[test]
    fn halt_stops_before_executing_following_instructions() {
        // Entry with Halt as the only instruction
        let mut vm = Vm::new(make_prog(vec![
            Instruction::single(Opcode::Halt),
            Instruction::rri(Opcode::Ldi, 0, 0, 99),
        ]));
        vm.call("main");
        assert_eq!(vm.int_regs[0], 0);
    }

    // ── Integer overflow ──

    #[test]
    fn i64_add_overflow_wraps_from_max_to_min() {
        let mut prog = QfrProgram::new();
        let max_idx = prog.intern_i64(i64::MAX);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 0, 0, max_idx),
            Instruction::rri(Opcode::Ldi, 1, 0, 1),
            Instruction::rrr(Opcode::Add, 2, 0, 1),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert_eq!(vm.int_regs[2], i64::MAX.wrapping_add(1));
    }

    #[test]
    fn i64_sub_overflow_wraps_from_min_to_max() {
        let mut prog = QfrProgram::new();
        let min_idx = prog.intern_i64(i64::MIN);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 0, 0, min_idx),
            Instruction::rri(Opcode::Ldi, 1, 0, 1),
            Instruction::rrr(Opcode::Sub, 2, 0, 1),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert_eq!(vm.int_regs[2], i64::MIN.wrapping_sub(1));
    }

    #[test]
    fn i64_mul_overflow_wraps_for_large_values() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri40(Opcode::Ldi64, 0, 2_500_000_000i64),
            Instruction::ri40(Opcode::Ldi64, 1, 4_000_000_000i64),
            Instruction::rrr(Opcode::Mul, 2, 0, 1),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        let expected = 2_500_000_000i64.wrapping_mul(4_000_000_000i64);
        assert_eq!(vm.int_regs[2], expected);
    }

    #[test]
    fn shl_by_100_on_one_wraps_to_shift_by_36() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 1),
            Instruction::rri(Opcode::Ldi, 1, 0, 100),
            Instruction::rrr(Opcode::Shl, 2, 0, 1),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        // wrapping_shl: 1 << (100 % 64) = 1 << 36 = 68719476736
        assert_eq!(vm.int_regs[2], 1i64.wrapping_shl(100));
    }

    // ── NaN / Inf via register values ──

    #[test]
    fn fadd_of_nan_and_valid_float_returns_zero() {
        let mut prog = QfrProgram::new();
        let nan = prog.intern_f64(f64::NAN);
        let val = prog.intern_f64(5.0);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 192, 0, nan),
            Instruction::rri(Opcode::Ldc, 193, 0, val),
            Instruction::rrr(Opcode::FAdd, 194, 192, 193),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert_eq!(vm.float_regs[2], 0.0);
    }

    #[test]
    fn fmul_of_infinity_and_valid_float_returns_zero() {
        let mut prog = QfrProgram::new();
        let inf = prog.intern_f64(f64::INFINITY);
        let two = prog.intern_f64(2.0);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 192, 0, inf),
            Instruction::rri(Opcode::Ldc, 193, 0, two),
            Instruction::rrr(Opcode::FMul, 194, 192, 193),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert_eq!(vm.float_regs[2], 0.0);
    }

    // ── Rolling Window opcodes ──

    #[test]
    fn window_push_and_mean() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri40(Opcode::Ldi64, 0, 10),        // [0] r0 = 10 (int)
            Instruction::rr(Opcode::I2F, 192, 0),            // [1] f0 = 10.0
            Instruction::rri(Opcode::WindowPush, 192, 192, 0), // [2] window[0].push(10.0)
            Instruction::ri(Opcode::WindowMean, 193, 0),     // [3] f1 = window[0].mean()
            Instruction::single(Opcode::Halt),                // [4]
        ]));
        vm.call("main");
        assert!((vm.float_regs[1] - 10.0).abs() < 1e-10);
    }

    #[test]
    fn window_multiple_values_correct_mean() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri40(Opcode::Ldi64, 0, 10),
            Instruction::rr(Opcode::I2F, 192, 0),
            Instruction::rri(Opcode::WindowPush, 192, 192, 0), // push 10
            Instruction::ri40(Opcode::Ldi64, 1, 20),
            Instruction::rr(Opcode::I2F, 193, 1),
            Instruction::rri(Opcode::WindowPush, 192, 193, 0), // push 20
            Instruction::ri40(Opcode::Ldi64, 2, 30),
            Instruction::rr(Opcode::I2F, 194, 2),
            Instruction::rri(Opcode::WindowPush, 192, 194, 0), // push 30
            Instruction::ri(Opcode::WindowMean, 195, 0),       // mean
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert!((vm.float_regs[3] - 20.0).abs() < 1e-10);
    }

    #[test]
    fn window_mean_of_single_value() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri40(Opcode::Ldi64, 0, 42),
            Instruction::rr(Opcode::I2F, 192, 0),
            Instruction::rri(Opcode::WindowPush, 192, 192, 0),
            Instruction::ri(Opcode::WindowMean, 193, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert!((vm.float_regs[1] - 42.0).abs() < 1e-10);
    }

    #[test]
    fn window_min_and_max() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri40(Opcode::Ldi64, 0, 5),
            Instruction::rr(Opcode::I2F, 192, 0),
            Instruction::rri(Opcode::WindowPush, 192, 192, 0), // push 5
            Instruction::ri40(Opcode::Ldi64, 1, 3),
            Instruction::rr(Opcode::I2F, 193, 1),
            Instruction::rri(Opcode::WindowPush, 192, 193, 0), // push 3
            Instruction::ri40(Opcode::Ldi64, 2, 8),
            Instruction::rr(Opcode::I2F, 194, 2),
            Instruction::rri(Opcode::WindowPush, 192, 194, 0), // push 8
            Instruction::ri(Opcode::WindowMin, 195, 0),
            Instruction::ri(Opcode::WindowMax, 196, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert!((vm.float_regs[3] - 3.0).abs() < 1e-10, "min should be 3.0");
        assert!((vm.float_regs[4] - 8.0).abs() < 1e-10, "max should be 8.0");
    }

    #[test]
    fn window_stddev_of_constant_values() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri40(Opcode::Ldi64, 0, 100),
            Instruction::rr(Opcode::I2F, 192, 0),
            Instruction::rri(Opcode::WindowPush, 192, 192, 0),
            Instruction::rri(Opcode::WindowPush, 192, 192, 0),
            Instruction::rri(Opcode::WindowPush, 192, 192, 0),
            Instruction::ri(Opcode::WindowStddev, 193, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert!((vm.float_regs[1] - 0.0).abs() < 1e-10);
    }

    #[test]
    fn window_sum_of_multiple_values() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri40(Opcode::Ldi64, 0, 1),
            Instruction::rr(Opcode::I2F, 192, 0),
            Instruction::rri(Opcode::WindowPush, 192, 192, 0), // push 1
            Instruction::ri40(Opcode::Ldi64, 1, 2),
            Instruction::rr(Opcode::I2F, 193, 1),
            Instruction::rri(Opcode::WindowPush, 192, 193, 0), // push 2
            Instruction::ri40(Opcode::Ldi64, 2, 3),
            Instruction::rr(Opcode::I2F, 194, 2),
            Instruction::rri(Opcode::WindowPush, 192, 194, 0), // push 3
            Instruction::ri(Opcode::WindowSum, 195, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert!((vm.float_regs[3] - 6.0).abs() < 1e-10);
    }

    #[test]
    fn window_multiple_ids_independent() {
        let mut vm = Vm::new(make_prog(vec![
            // Push 10 to window 0
            Instruction::ri40(Opcode::Ldi64, 0, 10),
            Instruction::rr(Opcode::I2F, 192, 0),
            Instruction::rri(Opcode::WindowPush, 192, 192, 0),
            // Push 20 to window 1
            Instruction::ri40(Opcode::Ldi64, 1, 20),
            Instruction::rr(Opcode::I2F, 193, 1),
            Instruction::rri(Opcode::WindowPush, 192, 193, 1),
            // Read means
            Instruction::ri(Opcode::WindowMean, 194, 0),
            Instruction::ri(Opcode::WindowMean, 195, 1),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert!((vm.float_regs[2] - 10.0).abs() < 1e-10, "window 0 mean should be 10");
        assert!((vm.float_regs[3] - 20.0).abs() < 1e-10, "window 1 mean should be 20");
    }

    #[test]
    fn window_empty_returns_zero() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri(Opcode::WindowMean, 192, 0),
            Instruction::ri(Opcode::WindowStddev, 193, 0),
            Instruction::ri(Opcode::WindowMin, 194, 0),
            Instruction::ri(Opcode::WindowMax, 195, 0),
            Instruction::ri(Opcode::WindowSum, 196, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.float_regs[0], 0.0);
        assert_eq!(vm.float_regs[1], 0.0);
        assert_eq!(vm.float_regs[2], 0.0);
        assert_eq!(vm.float_regs[3], 0.0);
        assert_eq!(vm.float_regs[4], 0.0);
    }

    // ── Snapshot / Replay ──

    #[test]
    fn snapshot_captures_registers() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 42),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        let snap = vm.snapshot();
        assert_eq!(snap.int_regs[0], 42);
    }

    #[test]
    fn snapshot_captures_persist() {
        let mut prog = QfrProgram::new();
        prog.entries.push(crate::ir::EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 42),
            Instruction::rri(Opcode::PersistSet, 0, 0, 5),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        let snap = vm.snapshot();
        assert_eq!(snap.persist[5].int_val, 42);
    }

    #[test]
    fn restore_recovers_int_register() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 99),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        let snap = vm.snapshot();

        // New VM with fresh state
        let mut vm2 = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm2.restore(&snap);
        assert_eq!(vm2.int_regs[0], 99);
    }

    #[test]
    fn snapshot_window_push_then_restore() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri40(Opcode::Ldi64, 0, 42),
            Instruction::rr(Opcode::I2F, 192, 0),
            Instruction::rri(Opcode::WindowPush, 192, 192, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        let snap = vm.snapshot();

        let mut vm2 = Vm::new(make_prog(vec![
            Instruction::single(Opcode::Halt),
        ]));
        vm2.restore(&snap);
        assert!(vm2.windows[0].is_some());
        if let Some(ref w) = vm2.windows[0] {
            assert!((w.mean() - 42.0).abs() < 1e-10);
        }
    }

    #[test]
    fn snapshot_and_restore_indicators() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::single(Opcode::Halt),
        ]));
        vm.indicators.insert("ema".into(), 1.5);
        let snap = vm.snapshot();

        let mut vm2 = Vm::new(make_prog(vec![
            Instruction::single(Opcode::Halt),
        ]));
        vm2.restore(&snap);
        assert!((vm2.indicators.get("ema").unwrap() - 1.5).abs() < 1e-10);
    }
}
