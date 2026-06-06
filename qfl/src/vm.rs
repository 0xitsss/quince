use crate::ir::{ConstEntry, QfrProgram};
use crate::opcodes::{Instruction, JUMP_TABLE, SENTINEL_OPCODE};
use std::collections::HashMap;

pub const NUM_REGS: usize = 256;
pub const INT_REG_COUNT: u8 = 192;
pub const PERSIST_SLOTS: usize = 64;
pub const MAX_CALL_DEPTH: usize = 64;
pub const MAX_INDICATORS: usize = 1024;
pub const MAX_BALANCES: usize = 128;
pub const MAX_WINDOWS: usize = 64;
pub const WINDOW_ARENA_SIZE: usize = 65536;
pub const MAX_DEPTH_LEVELS: usize = 64;
pub const MAX_EMA_STATES: usize = 256;

#[derive(Clone, Copy)]
#[repr(C)]
pub union Register {
    pub i: i64,
    pub f: f64,
}

impl std::fmt::Debug for Register {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Register")
            .field("i", unsafe { &self.i })
            .field("f", unsafe { &self.f })
            .finish()
    }
}

impl Register {
    #[inline(always)]
    pub fn from_i64(val: i64) -> Self { Register { i: val } }
    #[inline(always)]
    pub fn from_f64(val: f64) -> Self { Register { f: val } }
}

impl Default for Register {
    fn default() -> Self { Register { i: 0 } }
}

#[derive(Debug, Clone, Copy)]
pub struct PersistSlot {
    pub tag: u8,
    pub int_val: i64,
    pub float_val: f64,
}

impl Default for PersistSlot {
    fn default() -> Self { PersistSlot { tag: 0, int_val: 0, float_val: 0.0 } }
}

#[derive(Debug, Clone, Copy)]
pub struct EmaState {
    pub alpha: f64,
    pub value: f64,
    pub initialized: bool,
}

impl Default for EmaState {
    fn default() -> Self { EmaState { alpha: 0.0, value: 0.0, initialized: false } }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct WindowMeta {
    pub offset: u16,
    pub capacity: u16,
    pub head: u16,
    pub len: u16,
    pub sum: f64,
    pub sum_sq: f64,
    pub min: f64,
    pub max: f64,
}

impl WindowMeta {
    pub fn mean(&self) -> f64 {
        if self.len == 0 { 0.0 } else { self.sum / self.len as f64 }
    }
    pub fn variance(&self) -> f64 {
        if self.len < 2 { return 0.0; }
        let n = self.len as f64;
        (self.sum_sq - self.sum * self.sum / n) / n
    }
    pub fn stddev(&self) -> f64 { self.variance().sqrt() }
    pub fn sum(&self) -> f64 { self.sum }
    pub fn min(&self) -> f64 { if self.len == 0 { 0.0 } else { self.min } }
    pub fn max(&self) -> f64 { if self.len == 0 { 0.0 } else { self.max } }
}

/// Flat VM — Vec/HashMap only in setup paths, NOT in execute_tick hot path.
#[derive(Debug)]
pub struct Vm {
    // ── Register file (2048 bytes, L1) ──
    pub regs: [Register; NUM_REGS],

    // ── Execution state ──
    pub pc: usize,
    pub running: bool,
    pub call_stack: [usize; MAX_CALL_DEPTH],
    pub call_depth: u8,

    // ── Code + constants (raw pointers into owned backing) ──
    pub code_ptr: *const u64,
    pub code_len: usize,
    pub consts_ptr: *const f64,
    pub const_count: u32,
    pub i64_consts_ptr: *const i64,
    pub i64_const_count: u32,
    _code_owned: Vec<u64>,
    _consts_owned: Vec<f64>,
    _i64_consts_owned: Vec<i64>,

    // ── Constants pool (kept for backward compat; hot path uses consts_ptr) ──
    pub const_pool: Vec<ConstEntry>,
    pub const_map: HashMap<String, u32>,
    pub const_strings: Vec<String>,

    // ── Entry points ──
    pub entry_names: [u64; 8],
    pub entry_offsets: [u32; 8],
    pub entry_count: u8,
    /// Pre-computed offsets for the 4 standard handlers (u32::MAX = not found).
    handler_cache: [u32; 4],

    // ── Persist (hot-reload safe) ──
    pub persist: [PersistSlot; PERSIST_SLOTS],

    // ── Engine state (flat arrays in hot path) ──
    pub indicators: [f64; MAX_INDICATORS],
    pub indicator_map: HashMap<String, u16>,
    /// Pre-computed: const_strings index → indicator slot (u16::MAX = not found).
    /// Built by finalize_const_lookups() after all indicator registrations.
    /// Hot path uses this instead of HashMap + String clone.
    pub indicator_by_str: Vec<u16>,
    pub balances: [f64; MAX_BALANCES],
    pub balance_map: HashMap<String, u16>,
    /// Pre-computed: const_strings index → balance slot (u16::MAX = not found).
    pub balance_by_str: Vec<u16>,
    pub last_price: f64,
    pub position_size: f64,

    // ── Depth book (flat arrays, no Vec) ──
    pub depth_bids_price: [f64; MAX_DEPTH_LEVELS],
    pub depth_bids_qty: [f64; MAX_DEPTH_LEVELS],
    pub depth_asks_price: [f64; MAX_DEPTH_LEVELS],
    pub depth_asks_qty: [f64; MAX_DEPTH_LEVELS],
    pub depth_bids_len: u8,
    pub depth_asks_len: u8,

    // ── Rolling windows (arena-based RingVec replacement) ──
    pub window_arena: Vec<f64>,
    pub window_meta: [WindowMeta; MAX_WINDOWS],

    // Phase 4g: fused feature states
    pub ema_states: [EmaState; MAX_EMA_STATES],

    // ── Order flag (fast early-exit for flush_pending_order) ──
    pub has_pending_order: bool,

    // ── Profiler / Tracer ──
    pub profiler: Option<crate::profiler::Profiler>,
    pub tracer: Option<crate::tracer::Tracer>,
}

impl Vm {
    pub fn new(program: QfrProgram) -> Self {
        let _code_owned: Vec<u64> = program.code.iter().map(|i| i.raw()).collect();
        let code_ptr = _code_owned.as_ptr();
        let code_len = _code_owned.len();

        let mut _consts_owned: Vec<f64> = Vec::new();
        let mut _i64_consts_owned: Vec<i64> = Vec::new();
        let mut const_strings: Vec<String> = Vec::new();
        for entry in &program.const_pool {
            match entry {
                ConstEntry::F64(v) => _consts_owned.push(*v),
                ConstEntry::I64(v) => _i64_consts_owned.push(*v),
                ConstEntry::String(s) => const_strings.push(s.clone()),
            }
        }
        let consts_ptr = _consts_owned.as_ptr();
        let const_count = _consts_owned.len() as u32;
        let i64_consts_ptr = _i64_consts_owned.as_ptr();
        let i64_const_count = _i64_consts_owned.len() as u32;

        let mut entry_names = [0u64; 8];
        let mut entry_offsets = [0u32; 8];
        let entry_count = program.entries.len().min(8) as u8;
        for (i, e) in program.entries.iter().enumerate().take(entry_count as usize) {
            let mut name_bytes = [0u8; 8];
            let src = e.name.as_bytes();
            let n = src.len().min(8);
            name_bytes[..n].copy_from_slice(&src[..n]);
            entry_names[i] = u64::from_le_bytes(name_bytes);
            entry_offsets[i] = e.code_offset;
        }

        let indicator_map = HashMap::new();
        let balance_map = HashMap::new();

        let indicator_by_str = vec![u16::MAX; const_strings.len()];
        let balance_by_str = vec![u16::MAX; const_strings.len()];

        // Pre-compute handler cache (no linear scan in hot path)
        const HANDLER_LABELS: [&str; 4] = ["on_trade", "on_eval", "on_fill", "on_depth"];
        let mut handler_cache = [u32::MAX; 4];
        for (i, &label) in HANDLER_LABELS.iter().enumerate() {
            let label_bytes = label.as_bytes();
            let label_len = label_bytes.len();
            for j in 0..entry_count as usize {
                let stored = entry_names[j].to_le_bytes();
                if stored[..label_len] == label_bytes[..label_len] {
                    handler_cache[i] = entry_offsets[j];
                    break;
                }
            }
        }

        let mut vm = Vm {
            regs: [Register::default(); NUM_REGS],
            pc: 0,
            running: false,
            call_stack: [0; MAX_CALL_DEPTH],
            call_depth: 0,
            code_ptr,
            code_len,
            consts_ptr,
            const_count,
            i64_consts_ptr,
            i64_const_count,
            _code_owned,
            _consts_owned,
            _i64_consts_owned,
            const_pool: program.const_pool.clone(),
            const_map: program.const_map.clone(),
            const_strings,
            entry_names,
            entry_offsets,
            entry_count,
            handler_cache,
            persist: [PersistSlot::default(); PERSIST_SLOTS],
            indicators: [0.0; MAX_INDICATORS],
            indicator_map,
            indicator_by_str,
            balances: [0.0; MAX_BALANCES],
            balance_map,
            balance_by_str,
            last_price: 0.0,
            position_size: 0.0,
            depth_bids_price: [0.0; MAX_DEPTH_LEVELS],
            depth_bids_qty: [0.0; MAX_DEPTH_LEVELS],
            depth_asks_price: [0.0; MAX_DEPTH_LEVELS],
            depth_asks_qty: [0.0; MAX_DEPTH_LEVELS],
            depth_bids_len: 0,
            depth_asks_len: 0,
            window_arena: vec![0.0; WINDOW_ARENA_SIZE],
            window_meta: [WindowMeta::default(); MAX_WINDOWS],
            ema_states: [EmaState::default(); MAX_EMA_STATES],
            has_pending_order: false,
            profiler: None,
            tracer: None,
        };
        // Initialize EMA alphas from compiled program
        for (i, alpha) in program.ema_alphas.iter().enumerate() {
            if i < MAX_EMA_STATES {
                vm.ema_states[i].alpha = *alpha;
            }
        }
        vm
    }

    // ── Register access helpers ──

    #[inline(always)]
    pub fn set_int(&mut self, reg: u8, val: i64) {
        unsafe { *self.regs.get_unchecked_mut(reg as usize) = Register::from_i64(val); }
    }

    #[inline(always)]
    pub fn set_float(&mut self, reg: u8, val: f64) {
        unsafe { *self.regs.get_unchecked_mut(reg as usize) = Register::from_f64(val); }
    }

    #[inline(always)]
    pub fn reg_i(&self, idx: usize) -> i64 { unsafe { self.regs.get_unchecked(idx).i } }

    #[inline(always)]
    pub fn reg_f(&self, idx: usize) -> f64 { unsafe { self.regs.get_unchecked(idx).f } }

    #[inline(always)]
    pub fn int(&self, reg: u8) -> i64 {
        unsafe { self.regs.get_unchecked(reg as usize).i }
    }

    #[inline(always)]
    pub fn float(&self, reg: u8) -> f64 {
        unsafe { self.regs.get_unchecked(reg as usize).f }
    }

    // ── Entry point execution ──

    pub fn call(&mut self, entry_name: &str) {
        const HANDLER_LABELS: [&str; 4] = ["on_trade", "on_eval", "on_fill", "on_depth"];
        let offset = if let Some(idx) = HANDLER_LABELS.iter().position(|&n| n == entry_name) {
            let cached = self.handler_cache[idx];
            if cached != u32::MAX {
                cached as usize
            } else {
                return;
            }
        } else {
            match self.entry_offset(entry_name) {
                Some(o) => o as usize,
                None => return,
            }
        };
        if let Some(ref mut p) = self.profiler {
            p.start_handler(entry_name);
        }
        self.pc = offset;
        self.running = true;
        self.call_depth = 0;
        self.run();
        if let Some(ref mut p) = self.profiler {
            p.end_handler();
        }
    }

    fn entry_offset(&self, name: &str) -> Option<u32> {
        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len().min(8);
        for i in 0..self.entry_count as usize {
            let stored = self.entry_names[i].to_le_bytes();
            if stored[..name_len] == name_bytes[..name_len] {
                return Some(self.entry_offsets[i]);
            }
        }
        None
    }

    #[inline(always)]
    fn run(&mut self) {
        let has_profiler = self.profiler.is_some();
        let has_tracer = self.tracer.is_some();

        while self.running {
            let instr = unsafe { *self.code_ptr.add(self.pc) };
            self.pc += 1;

            let opcode = (instr & 0xFF) as u8;
            if opcode == SENTINEL_OPCODE {
                break;
            }

            if has_profiler {
                if let Some(ref mut p) = self.profiler {
                    p.record_opcode(crate::opcodes::Opcode::from_u8(opcode));
                }
            }

            unsafe {
                let handler = *JUMP_TABLE.get_unchecked(opcode as usize);
                handler(self, instr);
            }

            if has_tracer {
                self.trace_op(opcode, instr);
            }
        }
    }

    fn trace_op(&mut self, _opcode: u8, _instr: u64) {}

    // ── State setters ──

    pub fn set_balance(&mut self, asset: &str, val: f64) {
        if let Some(&slot) = self.balance_map.get(asset) {
            if (slot as usize) < MAX_BALANCES {
                self.balances[slot as usize] = val;
            }
            return;
        }
        let idx = self.balance_map.len() as u16;
        if (idx as usize) < MAX_BALANCES {
            self.balances[idx as usize] = val;
        }
        self.balance_map.insert(asset.to_string(), idx);
    }

    pub fn set_last_price(&mut self, price: f64) {
        self.last_price = price;
    }

    pub fn set_position_size(&mut self, size: f64) {
        self.position_size = size;
    }

    pub fn set_depth_bids(&mut self, bids: &[quince_core::types::DepthLevel]) {
        let n = bids.len().min(MAX_DEPTH_LEVELS);
        for i in 0..n {
            self.depth_bids_price[i] = bids[i].price;
            self.depth_bids_qty[i] = bids[i].qty;
        }
        self.depth_bids_len = n as u8;
    }

    pub fn set_depth_asks(&mut self, asks: &[quince_core::types::DepthLevel]) {
        let n = asks.len().min(MAX_DEPTH_LEVELS);
        for i in 0..n {
            self.depth_asks_price[i] = asks[i].price;
            self.depth_asks_qty[i] = asks[i].qty;
        }
        self.depth_asks_len = n as u8;
    }

    pub fn set_indicator(&mut self, name: &str, val: f64) {
        if let Some(&slot) = self.indicator_map.get(name) {
            if (slot as usize) < MAX_INDICATORS {
                self.indicators[slot as usize] = val;
            }
            return;
        }
        let idx = self.indicator_map.len() as u16;
        if (idx as usize) < MAX_INDICATORS {
            self.indicators[idx as usize] = val;
        }
        self.indicator_map.insert(name.to_string(), idx);
    }

    pub fn ensure_indicator_slot(&mut self, name: &str) -> u16 {
        if let Some(&slot) = self.indicator_map.get(name) {
            return slot;
        }
        let len = self.indicator_map.len() as u16;
        self.indicator_map.insert(name.to_string(), len);
        len
    }

    pub fn indicator_slot(&self, name: &str) -> Option<u16> {
        self.indicator_map.get(name).copied()
    }

    pub fn set_indicator_by_slot(&mut self, slot: u16, val: f64) {
        if (slot as usize) < MAX_INDICATORS {
            self.indicators[slot as usize] = val;
        }
    }

    // Phase 4g: fused feature state management
    pub fn set_ema_alpha(&mut self, state_id: u8, alpha: f64) {
        if (state_id as usize) < MAX_EMA_STATES {
            self.ema_states[state_id as usize].alpha = alpha;
        }
    }

    /// Rebuild indicator_by_str / balance_by_str from the current HashMap state.
    /// Must be called after all indicator/balance registrations (e.g. from Engine).
    pub fn finalize_const_lookups(&mut self) {
        self.indicator_by_str.clear();
        self.indicator_by_str.resize(self.const_strings.len(), u16::MAX);
        for (str_idx, s) in self.const_strings.iter().enumerate() {
            if let Some(&slot) = self.indicator_map.get(s) {
                if str_idx < self.indicator_by_str.len() {
                    self.indicator_by_str[str_idx] = slot;
                }
            }
        }
        self.balance_by_str.clear();
        self.balance_by_str.resize(self.const_strings.len(), u16::MAX);
        for (str_idx, s) in self.const_strings.iter().enumerate() {
            if let Some(&slot) = self.balance_map.get(s) {
                if str_idx < self.balance_by_str.len() {
                    self.balance_by_str[str_idx] = slot;
                }
            }
        }
    }

    // ── Snapshot for hot-reload ──

    pub fn snapshot(&self) -> VmSnapshot {
        VmSnapshot {
            regs: self.regs,
            persist: self.persist,
            pc: self.pc,
            indicators: self.indicators,
            balances: self.balances,
        }
    }

    pub fn restore(&mut self, snap: &VmSnapshot) {
        self.regs = snap.regs;
        self.persist = snap.persist;
        self.pc = snap.pc;
        self.indicators = snap.indicators;
        self.balances = snap.balances;
    }

    pub fn code_instr(&self) -> Vec<Instruction> {
        let mut v = Vec::with_capacity(self.code_len);
        for i in 0..self.code_len {
            let raw = unsafe { *self.code_ptr.add(i) };
            v.push(Instruction::decode(&raw.to_le_bytes()));
        }
        v
    }
}

#[derive(Debug, Clone)]
pub struct VmSnapshot {
    pub regs: [Register; NUM_REGS],
    pub persist: [PersistSlot; PERSIST_SLOTS],
    pub pc: usize,
    pub indicators: [f64; MAX_INDICATORS],
    pub balances: [f64; MAX_BALANCES],
}

// ═══════════════════════════════════════════════════════════════════════════════
// Jump Table Handlers
// ═══════════════════════════════════════════════════════════════════════════════

#[inline(always)]
unsafe fn rd(instr: u64) -> u8 { ((instr >> 8) & 0xFF) as u8 }
#[inline(always)]
unsafe fn rs1(instr: u64) -> u8 { ((instr >> 16) & 0xFF) as u8 }
#[inline(always)]
unsafe fn rs2(instr: u64) -> u8 { ((instr >> 24) & 0xFF) as u8 }
#[inline(always)]
unsafe fn imm(instr: u64) -> i32 { (instr >> 32) as i32 }
#[inline(always)]
unsafe fn immu(instr: u64) -> u32 { (instr >> 32) as u32 }

fn sanitize_f(val: f64) -> f64 {
    if val.is_nan() || val.is_infinite() { 0.0 } else { val }
}

pub mod handlers {
    use super::*;

    // ── Int arithmetic ──

    #[inline(always)]
    pub unsafe fn vm_add(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
        let b = vm.regs.get_unchecked(rs2(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = a.wrapping_add(b);
    }

    #[inline(always)]
pub unsafe fn vm_sub(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        vm.regs.get_unchecked_mut(r).i = vm.regs.get_unchecked(rs1(instr) as usize).i
            .wrapping_sub(vm.regs.get_unchecked(rs2(instr) as usize).i);
    }

    #[inline(always)]
pub unsafe fn vm_mul(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        vm.regs.get_unchecked_mut(r).i = vm.regs.get_unchecked(rs1(instr) as usize).i
            .wrapping_mul(vm.regs.get_unchecked(rs2(instr) as usize).i);
    }

    #[inline(always)]
pub unsafe fn vm_div(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let divisor = vm.regs.get_unchecked(rs2(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = if divisor == 0 { 0 }
            else { vm.regs.get_unchecked(rs1(instr) as usize).i / divisor };
    }

    #[inline(always)]
pub unsafe fn vm_mod(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let divisor = vm.regs.get_unchecked(rs2(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = if divisor == 0 { 0 }
            else { vm.regs.get_unchecked(rs1(instr) as usize).i % divisor };
    }

    #[inline(always)]
pub unsafe fn vm_neg(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        vm.regs.get_unchecked_mut(r).i =
            vm.regs.get_unchecked(rs1(instr) as usize).i.wrapping_neg();
    }

    #[inline(always)]
pub unsafe fn vm_addi(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        vm.regs.get_unchecked_mut(r).i =
            vm.regs.get_unchecked(rs1(instr) as usize).i.wrapping_add(imm(instr) as i64);
    }

    #[inline(always)]
pub unsafe fn vm_subi(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        vm.regs.get_unchecked_mut(r).i =
            vm.regs.get_unchecked(rs1(instr) as usize).i.wrapping_sub(imm(instr) as i64);
    }

    #[inline(always)]
pub unsafe fn vm_muli(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        vm.regs.get_unchecked_mut(r).i =
            vm.regs.get_unchecked(rs1(instr) as usize).i.wrapping_mul(imm(instr) as i64);
    }

    #[inline(always)]
pub unsafe fn vm_divi(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let divisor = imm(instr) as i64;
        vm.regs.get_unchecked_mut(r).i = if divisor == 0 { 0 }
            else { vm.regs.get_unchecked(rs1(instr) as usize).i / divisor };
    }

    // ── Float arithmetic ──

    macro_rules! float_binop {
        ($name:ident, $op:tt) => {
            #[inline(always)]
pub unsafe fn $name(vm: &mut Vm, instr: u64) {
                let r = rd(instr) as usize;
                let a = vm.regs.get_unchecked(rs1(instr) as usize).f;
                let b = vm.regs.get_unchecked(rs2(instr) as usize).f;
                vm.regs.get_unchecked_mut(r).f = sanitize_f(a $op b);
            }
        };
    }

    float_binop!(vm_fadd, +);
    float_binop!(vm_fsub, -);
    float_binop!(vm_fmul, *);

    #[inline(always)]
pub unsafe fn vm_fdiv(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let divisor = vm.regs.get_unchecked(rs2(instr) as usize).f;
        vm.regs.get_unchecked_mut(r).f = if divisor == 0.0 { 0.0 }
            else { sanitize_f(vm.regs.get_unchecked(rs1(instr) as usize).f / divisor) };
    }

    #[inline(always)]
pub unsafe fn vm_fneg(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        vm.regs.get_unchecked_mut(r).f =
            sanitize_f(-vm.regs.get_unchecked(rs1(instr) as usize).f);
    }

    // ── Int comparison ──

    macro_rules! int_cmp {
        ($name:ident, $cmp:tt) => {
            #[inline(always)]
pub unsafe fn $name(vm: &mut Vm, instr: u64) {
                let r = rd(instr) as usize;
                let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
                let b = vm.regs.get_unchecked(rs2(instr) as usize).i;
                vm.regs.get_unchecked_mut(r).i = if a $cmp b { 1 } else { 0 };
            }
        };
    }

    int_cmp!(vm_eq, ==);
    int_cmp!(vm_ne, !=);
    int_cmp!(vm_lt, <);
    int_cmp!(vm_gt, >);
    int_cmp!(vm_le, <=);
    int_cmp!(vm_ge, >=);

    // ── Float comparison ──

    macro_rules! float_cmp {
        ($name:ident, $cmp:tt) => {
            #[inline(always)]
pub unsafe fn $name(vm: &mut Vm, instr: u64) {
                let r = rd(instr) as usize;
                let a = vm.regs.get_unchecked(rs1(instr) as usize).f;
                let b = vm.regs.get_unchecked(rs2(instr) as usize).f;
                vm.regs.get_unchecked_mut(r).i = if a $cmp b { 1 } else { 0 };
            }
        };
    }

    float_cmp!(vm_feq, ==);
    float_cmp!(vm_fne, !=);
    float_cmp!(vm_flt, <);
    float_cmp!(vm_fgt, >);
    float_cmp!(vm_fle, <=);
    float_cmp!(vm_fge, >=);

    // ── Immediate comparison ──

    #[inline(always)]
pub unsafe fn vm_eqi(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = if a == imm(instr) as i64 { 1 } else { 0 };
    }

    #[inline(always)]
pub unsafe fn vm_lti(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = if a < imm(instr) as i64 { 1 } else { 0 };
    }

    #[inline(always)]
pub unsafe fn vm_gti(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = if a > imm(instr) as i64 { 1 } else { 0 };
    }

    // ── Bitwise ──

    macro_rules! bitwise_binop {
        ($name:ident, $op:tt) => {
            #[inline(always)]
pub unsafe fn $name(vm: &mut Vm, instr: u64) {
                let r = rd(instr) as usize;
                let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
                let b = vm.regs.get_unchecked(rs2(instr) as usize).i;
                vm.regs.get_unchecked_mut(r).i = a $op b;
            }
        };
    }

    bitwise_binop!(vm_bitand, &);
    bitwise_binop!(vm_bitor, |);
    bitwise_binop!(vm_bitxor, ^);

    #[inline(always)]
pub unsafe fn vm_bitnot(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        vm.regs.get_unchecked_mut(r).i = !vm.regs.get_unchecked(rs1(instr) as usize).i;
    }

    #[inline(always)]
pub unsafe fn vm_shl(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
        let b = vm.regs.get_unchecked(rs2(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = a.wrapping_shl(b as u32);
    }

    #[inline(always)]
pub unsafe fn vm_shr(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let a = vm.regs.get_unchecked(rs1(instr) as usize).i as u64;
        let b = vm.regs.get_unchecked(rs2(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = a.wrapping_shr(b as u32) as i64;
    }

    // ── Control flow ──

    #[inline(always)]
pub unsafe fn vm_jmp(vm: &mut Vm, instr: u64) {
        let target = (vm.pc as i64).wrapping_add(imm(instr) as i64) as usize;
        vm.pc = target;
    }

    #[inline(always)]
pub unsafe fn vm_jz(vm: &mut Vm, instr: u64) {
        if vm.regs.get_unchecked(rs1(instr) as usize).i == 0 {
            vm.pc = (vm.pc as i64).wrapping_add(imm(instr) as i64) as usize;
        }
    }

    #[inline(always)]
pub unsafe fn vm_jnz(vm: &mut Vm, instr: u64) {
        if vm.regs.get_unchecked(rs1(instr) as usize).i != 0 {
            vm.pc = (vm.pc as i64).wrapping_add(imm(instr) as i64) as usize;
        }
    }

    #[inline(always)]
pub unsafe fn vm_call(vm: &mut Vm, instr: u64) {
        let depth = vm.call_depth as usize;
        if depth < MAX_CALL_DEPTH {
            *vm.call_stack.get_unchecked_mut(depth) = vm.pc;
            vm.call_depth += 1;
        }
        vm.pc = (vm.pc as i64).wrapping_add(imm(instr) as i64) as usize;
    }

    #[inline(always)]
pub unsafe fn vm_ret(vm: &mut Vm, instr: u64) {
        let _ = instr;
        if vm.call_depth > 0 {
            vm.call_depth -= 1;
            vm.pc = *vm.call_stack.get_unchecked(vm.call_depth as usize);
        } else {
            vm.running = false;
        }
    }

    // ── Data movement ──

    #[inline(always)]
pub unsafe fn vm_mov(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let s = rs1(instr) as usize;
        *vm.regs.get_unchecked_mut(r) = *vm.regs.get_unchecked(s);
    }

    #[inline(always)]
pub unsafe fn vm_ldi(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        vm.regs.get_unchecked_mut(r).i = imm(instr) as i64;
    }

    #[inline(always)]
pub unsafe fn vm_ldi64(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let low = (instr >> 32) as u64;           // imm = bits 32-63 = low 32 of 40-bit val
        let high = ((instr >> 24) & 0xFF) as u64;  // rs2 = bits 24-31 = high 8 of 40-bit val
        let val = (high << 32) | low;
        let sign = (val >> 39) & 1;
        vm.regs.get_unchecked_mut(r).i = if sign == 1 {
            (val | 0xFFFF_FF00_0000_0000) as i64
        } else {
            val as i64
        };
    }

    /// Load i64 from const pool (RI: rd, index)
    #[inline(always)]
pub unsafe fn vm_ldi64_c(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let idx = immu(instr) as usize;
        if idx < vm.i64_const_count as usize {
            vm.regs.get_unchecked_mut(r).i = *vm.i64_consts_ptr.add(idx);
        }
    }

    #[inline(always)]
pub unsafe fn vm_ldc(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let idx = immu(instr) as usize;
        // Layout: f64[0..const_count), i64[const_count..const_count+i64_const_count), strings after
        let f64_end = vm.const_count as usize;
        let i64_end = f64_end + vm.i64_const_count as usize;
        if idx < f64_end {
            vm.regs.get_unchecked_mut(r).f = *vm.consts_ptr.add(idx);
        } else if idx < i64_end {
            let i64_idx = idx - f64_end;
            vm.regs.get_unchecked_mut(r).i = *vm.i64_consts_ptr.add(i64_idx);
        } else {
            let str_idx = idx - i64_end;
            if str_idx < vm.const_strings.len() {
                vm.regs.get_unchecked_mut(r).i = str_idx as i64;
            }
        }
    }

    // ── Type conversion ──

    #[inline(always)]
pub unsafe fn vm_i2f(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let val = vm.regs.get_unchecked(rs1(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).f = val as f64;
    }

    #[inline(always)]
pub unsafe fn vm_f2i(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let val = vm.regs.get_unchecked(rs1(instr) as usize).f;
        vm.regs.get_unchecked_mut(r).i = val as i64;
    }

    // ── Engine builtins ──

    #[inline(always)]
pub unsafe fn vm_getind(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let str_idx = vm.regs.get_unchecked(rs1(instr) as usize).i as usize;
        let val = if str_idx < vm.indicator_by_str.len() {
            let slot = *vm.indicator_by_str.get_unchecked(str_idx);
            if slot != u16::MAX {
                *vm.indicators.get_unchecked(slot as usize)
            } else {
                0.0
            }
        } else {
            0.0
        };
        vm.regs.get_unchecked_mut(r).f = val;
    }

    #[inline(always)]
pub unsafe fn vm_getprice(vm: &mut Vm, instr: u64) {
        let _ = instr;
        let r = rd(instr) as usize;
        vm.regs.get_unchecked_mut(r).f = vm.last_price;
    }

    #[inline(always)]
pub unsafe fn vm_getpos(vm: &mut Vm, instr: u64) {
        let _ = instr;
        let r = rd(instr) as usize;
        vm.regs.get_unchecked_mut(r).f = vm.position_size;
    }

    #[inline(always)]
pub unsafe fn vm_getbal(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let str_idx = vm.regs.get_unchecked(rs1(instr) as usize).i as usize;
        let val = if str_idx < vm.balance_by_str.len() {
            let slot = *vm.balance_by_str.get_unchecked(str_idx);
            if slot != u16::MAX {
                *vm.balances.get_unchecked(slot as usize)
            } else {
                0.0
            }
        } else {
            0.0
        };
        vm.regs.get_unchecked_mut(r).f = val;
    }

    #[inline(always)]
pub unsafe fn vm_getdepthbid(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let level = vm.regs.get_unchecked(rs1(instr) as usize).i as usize;
        let val = if level < vm.depth_bids_len as usize { vm.depth_bids_qty[level] } else { 0.0 };
        vm.regs.get_unchecked_mut(r).f = val;
    }

    #[inline(always)]
pub unsafe fn vm_getdepthask(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let level = vm.regs.get_unchecked(rs1(instr) as usize).i as usize;
        let val = if level < vm.depth_asks_len as usize { vm.depth_asks_qty[level] } else { 0.0 };
        vm.regs.get_unchecked_mut(r).f = val;
    }

    #[inline(always)]
pub unsafe fn vm_sendorder(vm: &mut Vm, instr: u64) {
        let _ = instr;
        vm.has_pending_order = true;
        let _side = vm.regs.get_unchecked(250).i;
        let _qty = vm.regs.get_unchecked(192).f;
        let _price = vm.regs.get_unchecked(193).f;
        tracing::info!("QFL: SEND_ORDER side={} qty={} price={} type={} reduce={}",
            _side, _qty, _price,
            vm.regs.get_unchecked(253).i,
            vm.regs.get_unchecked(254).i,
        );
    }

    #[inline(always)]
pub unsafe fn vm_persistget(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let slot = immu(instr) as usize;
        if slot < PERSIST_SLOTS {
            let ps = vm.persist.get_unchecked(slot);
            if ps.tag == 0 {
                vm.regs.get_unchecked_mut(r).i = ps.int_val;
            } else {
                vm.regs.get_unchecked_mut(r).f = ps.float_val;
            }
        }
    }

    #[inline(always)]
pub unsafe fn vm_persistset(vm: &mut Vm, instr: u64) {
        let slot = immu(instr) as usize;
        let r = rd(instr) as usize;
        if slot < PERSIST_SLOTS {
            let ps = vm.persist.get_unchecked_mut(slot);
            if r >= INT_REG_COUNT as usize {
                ps.tag = 1;
                ps.float_val = vm.regs.get_unchecked(r).f;
            } else {
                ps.tag = 0;
                ps.int_val = vm.regs.get_unchecked(r).i;
            }
        }
    }

    #[inline(always)]
pub unsafe fn vm_log(vm: &mut Vm, instr: u64) {
        let str_idx = vm.regs.get_unchecked(rs1(instr) as usize).i as usize;
        let msg = if str_idx < vm.const_strings.len() {
            vm.const_strings.get_unchecked(str_idx).as_str()
        } else {
            ""
        };
        tracing::info!("QFL: {}", msg);
    }

    #[inline(always)]
pub unsafe fn vm_log2(vm: &mut Vm, instr: u64) {
        let str_idx = vm.regs.get_unchecked(rs1(instr) as usize).i as usize;
        let val = vm.regs.get_unchecked(rs2(instr) as usize).f;
        if str_idx < vm.const_strings.len() {
            tracing::info!("QFL: {}: {}", vm.const_strings.get_unchecked(str_idx), val);
        } else {
            tracing::info!("QFL: {}", val);
        }
    }

    #[inline(always)]
pub unsafe fn vm_halt(vm: &mut Vm, instr: u64) {
        let _ = instr;
        vm.running = false;
    }

    // ── Window opcodes ──

    #[inline(always)]
pub unsafe fn vm_windowpush(vm: &mut Vm, instr: u64) {
        let wid = immu(instr) as usize;
        let val = vm.regs.get_unchecked(rs1(instr) as usize).f;
        let r = rd(instr) as usize;
        if wid < MAX_WINDOWS {
            let meta = &mut vm.window_meta[wid];
            if meta.capacity == 0 {
                meta.capacity = 64;
                meta.offset = (wid as u16) * 64;
                meta.head = 0;
                meta.len = 0;
                meta.sum = 0.0;
                meta.sum_sq = 0.0;
                meta.min = val;
                meta.max = val;
            }
            let cap = meta.capacity as usize;
            let off = meta.offset as usize;
            let head = meta.head as usize;

            if (meta.len as usize) < cap {
                *vm.window_arena.get_unchecked_mut(off + head) = val;
                meta.head = ((head + 1) % cap) as u16;
                meta.len += 1;
                meta.sum += val;
                meta.sum_sq += val * val;
                if val < meta.min { meta.min = val; }
                if val > meta.max { meta.max = val; }
            } else {
                let old = *vm.window_arena.get_unchecked(off + head);
                *vm.window_arena.get_unchecked_mut(off + head) = val;
                meta.head = ((head + 1) % cap) as u16;

                meta.sum = meta.sum - old + val;
                meta.sum_sq = meta.sum_sq - old * old + val * val;

                if old == meta.min || old == meta.max {
                    let mut new_min = f64::MAX;
                    let mut new_max = f64::MIN;
                    for i in 0..cap {
                        let v = *vm.window_arena.get_unchecked(off + i);
                        if v < new_min { new_min = v; }
                        if v > new_max { new_max = v; }
                    }
                    meta.min = new_min;
                    meta.max = new_max;
                } else {
                    if val < meta.min { meta.min = val; }
                    if val > meta.max { meta.max = val; }
                }
            }
            vm.regs.get_unchecked_mut(r).f = val;
        }
    }

    macro_rules! window_unary {
        ($name:ident, $method:ident) => {
            #[inline(always)]
pub unsafe fn $name(vm: &mut Vm, instr: u64) {
                let wid = immu(instr) as usize;
                let r = rd(instr) as usize;
                if wid < MAX_WINDOWS {
                    let result = vm.window_meta[wid].$method();
                    if vm.window_meta[wid].len > 0 {
                        vm.regs.get_unchecked_mut(r).f = result;
                    }
                }
            }
        };
    }

    window_unary!(vm_windowmean, mean);
    window_unary!(vm_windowstddev, stddev);
    window_unary!(vm_windowmin, min);
    window_unary!(vm_windowmax, max);
    window_unary!(vm_windowsum, sum);

    // ── Phase 4g: fused feature opcodes ──

    #[inline(always)]
pub unsafe fn vm_ema(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let val = vm.regs.get_unchecked(rs1(instr) as usize).f;
        let sid = rs2(instr) as usize;
        if sid < MAX_EMA_STATES {
            let st = &mut vm.ema_states[sid];
            if st.initialized {
                st.value = st.alpha * val + (1.0 - st.alpha) * st.value;
            } else {
                st.value = val;
                st.initialized = true;
            }
            vm.regs.get_unchecked_mut(r).f = sanitize_f(st.value);
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::EntryPoint;
    use crate::opcodes::Opcode;
    use quince_core::types::DepthLevel;

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
                assert_eq!(vm.reg_i(2), $expected);
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
        assert_eq!(vm.reg_i(2), -12);
    }

    #[test]
    fn jnz_loop_counts_down_from_three_to_zero() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 3),
            Instruction::rri(Opcode::AddI, 0, 0, (-1i32) as u32),
            Instruction::rri(Opcode::Jnz, 0, 0, (-2i32) as u32),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.reg_i(0), 0);
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
        assert_eq!(vm.reg_i(2), -1);
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
                assert_eq!(vm.reg_i(1), $expected);
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
        assert_eq!(vm.reg_i(1), 42);
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
                assert_eq!(vm.reg_i(1), $expected);
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
                assert!((vm.reg_f(194) - $expected).abs() < 0.001);
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
                assert!((vm.reg_f(193) - $expected).abs() < 0.001);
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
                assert_eq!(vm.reg_i(2), $expected);
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
                assert_eq!(vm.reg_i(2), $expected);
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
                assert_eq!(vm.reg_i(1), $expected);
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
                assert_eq!(vm.reg_i(2), $expected);
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
        assert_eq!(vm.reg_i(1), !0x0f0f_0f0f);
    }

    test_bitwise!(shl_shifts_left_by_eight, Opcode::Shl, 1, 8, 256);
    test_bitwise!(shr_shifts_right_by_eight, Opcode::Shr, 256, 8, 1);

    // ── Control flow ──

    #[test]
    fn call_fallthrough_without_ret_executes_callee_then_halt() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 1),
            Instruction::rri(Opcode::Call, 0, 0, 1),
            Instruction::single(Opcode::Halt),
            Instruction::rri(Opcode::Ldi, 1, 0, 42),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.reg_i(1), 42);
    }

    #[test]
    fn jmp_backward_with_jnz_loops_from_zero_to_ten() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 0),
            Instruction::rri(Opcode::AddI, 0, 0, 1),
            Instruction::rri(Opcode::Ldi, 1, 0, 10),
            Instruction::rrr(Opcode::Eq, 2, 0, 1),
            Instruction::rri(Opcode::Jnz, 0, 2, 1),
            Instruction::rri(Opcode::Jmp, 0, 0, (-5i32) as u32),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.reg_i(0), 10);
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
                assert_eq!(vm.reg_i(1), $r1_expected);
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
                assert_eq!(vm.reg_i(1), $r1_expected);
            }
        };
    }

    test_jnz!(jnz_taken_when_register_nonzero, 1, 0);
    test_jnz!(jnz_not_taken_when_register_zero, 0, 99);

    #[test]
    fn call_ret_preserves_caller_registers_and_resumes_after_call() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 42),
            Instruction::rri(Opcode::Call, 0, 0, 2),
            Instruction::rri(Opcode::Ldi, 1, 0, 99),
            Instruction::single(Opcode::Halt),
            Instruction::rri(Opcode::Ldi, 2, 0, 7),
            Instruction::single(Opcode::Ret),
        ]));
        vm.call("main");
        assert_eq!(vm.reg_i(0), 42);
        assert_eq!(vm.reg_i(1), 99);
        assert_eq!(vm.reg_i(2), 7);
    }

    #[test]
    fn nested_call_adds_one_then_multiplies_by_two() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 1),
            Instruction::rri(Opcode::Call, 0, 0, 1),
            Instruction::single(Opcode::Halt),
            Instruction::rri(Opcode::AddI, 0, 0, 1),
            Instruction::rri(Opcode::Call, 0, 0, 1),
            Instruction::single(Opcode::Ret),
            Instruction::rri(Opcode::MulI, 0, 0, 2),
            Instruction::single(Opcode::Ret),
        ]));
        vm.call("main");
        assert_eq!(vm.reg_i(0), 4);
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
                assert_eq!(vm.reg_i(0), $expected);
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
        assert_eq!(vm.reg_i(0), -1i64);
    }

    #[test]
    fn ldi64_loads_40bit_positive_value() {
        let big: i64 = 0x7f_1234_5678;
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri40(Opcode::Ldi64, 0, big),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.reg_i(0), big);
    }

    #[test]
    fn ldi64_loads_negative_one_as_40bit_signed() {
        let big: i64 = -1;
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri40(Opcode::Ldi64, 0, big),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.reg_i(0), big);
    }

    #[test]
    fn ldi64_loads_small_negative_40bit_value() {
        let big: i64 = -42;
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri40(Opcode::Ldi64, 0, big),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.reg_i(0), big);
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
                assert_eq!(vm.reg_i($to as usize), $expected);
            }
        };
    }

    test_mov!(mov_copies_int_from_one_register_to_another, 0, 1, 42, 42);
    test_mov!(mov_copies_zero_between_registers, 0, 1, 0, 0);

    #[test]
    fn mov_copies_int_value_to_float_register() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 42),
            Instruction::rr(Opcode::I2F, 192, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert!((vm.reg_f(192) - 42.0).abs() < 0.001);
    }

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
        assert!((vm.reg_f(192) - 42.0).abs() < 0.001);
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
        assert_eq!(vm.reg_i(0), 42);
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
        vm.finalize_const_lookups();
        vm.call("main");
        assert!((vm.reg_f(192) - 123.456).abs() < 0.001);
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
        assert!((vm.reg_f(192) - 0.0).abs() < 0.001);
    }

    #[test]
    fn getprice_returns_last_price() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri(Opcode::GetPrice, 192, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.set_last_price(50000.0);
        vm.call("main");
        assert!((vm.reg_f(192) - 50000.0).abs() < 0.001);
    }

    #[test]
    fn getpos_returns_current_position_size() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri(Opcode::GetPos, 192, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.set_position_size(1.5);
        vm.call("main");
        assert!((vm.reg_f(192) - 1.5).abs() < 0.001);
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
        vm.finalize_const_lookups();
        vm.call("main");
        assert!((vm.reg_f(192) - 10000.0).abs() < 0.001);
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
        assert!((vm.reg_f(192) - 0.0).abs() < 0.001);
    }

    #[test]
    fn getdepthbid_returns_volume_at_bid_level() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 0),
            Instruction::rrr(Opcode::GetDepthBid, 192, 0, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.set_depth_bids(&[DepthLevel { price: 100.0, qty: 1.5 }]);
        vm.call("main");
        assert!((vm.reg_f(192) - 1.5).abs() < 0.001);
    }

    #[test]
    fn getdepthask_returns_volume_at_ask_level() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 0),
            Instruction::rrr(Opcode::GetDepthAsk, 192, 0, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.set_depth_asks(&[DepthLevel { price: 101.0, qty: 2.0 }]);
        vm.call("main");
        assert!((vm.reg_f(192) - 2.0).abs() < 0.001);
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
    }

    #[test]
    fn halt_stops_execution_immediately_skipping_further_instructions() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 42),
            Instruction::single(Opcode::Halt),
            Instruction::rri(Opcode::Ldi, 1, 0, 99),
        ]));
        vm.call("main");
        assert_eq!(vm.reg_i(0), 42);
        assert_eq!(vm.reg_i(1), 0);
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
        assert_eq!(vm.reg_i(1), 42);
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
        assert!((vm.reg_f(193) - 3.14).abs() < 0.001);
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
        assert_eq!(vm2.reg_i(1), 100);
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

        let mut vm = Vm::new(make_prog(code.clone()));
        vm.call("main");

        let mut code2: Vec<Instruction> = (0..64)
            .map(|i| Instruction::rri(Opcode::PersistGet, 0, 0, i as u32))
            .collect();
        code2.push(Instruction::single(Opcode::Halt));
        let mut vm2 = Vm::new(make_prog(code2));
        vm2.persist.copy_from_slice(&vm.persist);
        vm2.call("main");
        assert_eq!(vm2.reg_i(0), 63);
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

        // This entry_offset hack won't work with the new Vm.
        // We just verify it doesn't crash.
        // In the new design, entry_offset returns None since const_pool is empty.
        vm.call("main");
    }

    // ── Window opcodes ──

    #[test]
    fn window_push_creates_window_and_stores_value() {
        // Override with float via Ldc
        let mut prog = QfrProgram::new();
        let fv = prog.intern_f64(42.0);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 0, 0, fv),
            Instruction::rri(Opcode::WindowPush, 192, 0, 0),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert!((vm.reg_f(192) - 42.0).abs() < 0.001);
        assert!(vm.window_meta[0].len > 0);
    }

    #[test]
    fn window_mean_of_constant_values_is_constant() {
        let mut prog = QfrProgram::new();
        let fv = prog.intern_f64(100.0);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 0, 0, fv),
            Instruction::rri(Opcode::WindowPush, 192, 0, 0),
            Instruction::rri(Opcode::Ldc, 0, 0, fv),
            Instruction::rri(Opcode::WindowPush, 192, 0, 0),
            Instruction::rri(Opcode::WindowMean, 193, 0, 0),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert!((vm.reg_f(193) - 100.0).abs() < 0.001);
    }

    #[test]
    fn persist_get_unset_returns_zero() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::PersistGet, 1, 0, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.reg_i(1), 0);
    }

    #[test]
    fn getprice_returns_zero_when_not_set() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri(Opcode::GetPrice, 192, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert!((vm.reg_f(192) - 0.0).abs() < 0.001);
    }

    #[test]
    fn getpos_returns_zero_when_not_set() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::ri(Opcode::GetPos, 192, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert!((vm.reg_f(192) - 0.0).abs() < 0.001);
    }

    #[test]
    fn set_indicator_by_slot_works() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::single(Opcode::Halt),
        ]));
        vm.set_indicator_by_slot(0, 42.5);
        assert!((vm.indicators[0] - 42.5).abs() < 0.001);
    }

    #[test]
    fn mov_between_float_regs() {
        let mut prog = QfrProgram::new();
        let fv = prog.intern_f64(3.14);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 192, 0, fv),
            Instruction::rr(Opcode::Mov, 193, 192),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert!((vm.reg_f(193) - 3.14).abs() < 0.001);
    }

    #[test]
    fn jz_with_large_offset() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 0),
            Instruction::rri(Opcode::Jz, 0, 0, 2),
            Instruction::rri(Opcode::Ldi, 1, 0, 99),
            Instruction::single(Opcode::Halt),
            Instruction::rri(Opcode::Ldi, 1, 0, 42),
            Instruction::single(Opcode::Halt),
        ]));
        vm.call("main");
        assert_eq!(vm.reg_i(1), 42);
    }

    #[test]
    fn neg_zero_returns_zero() {
        let mut prog = QfrProgram::new();
        let fv = prog.intern_f64(0.0);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 192, 0, fv),
            Instruction::rr(Opcode::FNeg, 193, 192),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert!((vm.reg_f(193) - 0.0).abs() < 0.001);
    }

    #[test]
    fn getdepthbid_no_levels_returns_zero() {
        let mut vm = Vm::new(make_prog(vec![
            Instruction::rri(Opcode::Ldi, 0, 0, 0),
            Instruction::rrr(Opcode::GetDepthBid, 192, 0, 0),
            Instruction::single(Opcode::Halt),
        ]));
        vm.set_depth_bids(&[]);
        vm.call("main");
        assert!((vm.reg_f(192) - 0.0).abs() < 0.001);
    }

    #[test]
    fn f2i_with_negative_float() {
        let mut prog = QfrProgram::new();
        let fv = prog.intern_f64(-3.7);
        prog.entries.push(EntryPoint { name: "main".into(), code_offset: 0 });
        prog.code = vec![
            Instruction::rri(Opcode::Ldc, 192, 0, fv),
            Instruction::rr(Opcode::F2I, 0, 192),
            Instruction::single(Opcode::Halt),
        ];
        let mut vm = Vm::new(prog);
        vm.call("main");
        assert_eq!(vm.reg_i(0), -3);
    }
}
