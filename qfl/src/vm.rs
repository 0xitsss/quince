// --- VM module: bytecode interpreter with jump-table dispatch ---
// Executes compiled QFL programs. Hot/cold split for cache efficiency.

use crate::ir::{ConstEntry, QfrProgram};
use crate::opcodes::{Instruction, JUMP_TABLE};
use std::collections::HashMap;
use std::io::Write;

// --- Architectural constants ---

pub const NUM_REGS: usize = 256;            // Total register file slots (2048 bytes)
pub const INT_REG_COUNT: u8 = 192;          // Regs 0-191 are integer-typed; 192-255 are float-typed
pub const PERSIST_SLOTS: usize = 64;        // Number of persist slots for hot-reload-safe state
pub const MAX_CALL_DEPTH: usize = 64;       // Max nested call/return depth
pub const MAX_INDICATORS: usize = 1024;     // Max number of named indicator slots
pub const MAX_BALANCES: usize = 128;       // Max number of named balance slots
pub const MAX_WINDOWS: usize = 64;          // Max number of rolling windows
pub const WINDOW_ARENA_SIZE: usize = 65536; // Total ring-buffer elements across all windows
pub const MAX_DEPTH_LEVELS: usize = 64;     // Max order-book depth levels per side
pub const MAX_EMA_STATES: usize = 256;      // Max number of EMA state slots

// --- SendOrder register convention ---
// When issuing a SEND_ORDER instruction, the VM reads these fixed registers:
pub const REG_SEND_SIDE: u8 = 250;   // Order side (i64: 0=buy, 1=sell)
pub const REG_SEND_QTY: u8 = 192;    // Order quantity (f64)
pub const REG_SEND_PRICE: u8 = 193;  // Order price (f64)
pub const REG_SEND_TYPE: u8 = 253;   // Order type (i64: e.g. market/limit)
pub const REG_SEND_REDUCE: u8 = 254; // Reduce-only flag (i64)

// --- Register File ---
// A 256-slot register file. Each slot stores either an i64 or f64 (union).
// Regs 0-191 are conventionally integer; regs 192-255 are conventionally float.

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
    pub fn from_i64(val: i64) -> Self {
        Register { i: val }
    }
    #[inline(always)]
    pub fn from_f64(val: f64) -> Self {
        Register { f: val }
    }
}

impl Default for Register {
    fn default() -> Self {
        Register { i: 0 }
    }
}

// --- Persist Slots (hot-reload safe state) ---
// Tag 0 = integer, tag 1 = float. Survives across hot-reloads via snapshot/restore.

#[derive(Debug, Clone, Copy)]
pub struct PersistSlot {
    pub tag: u8,          // 0=int, 1=float — determines which field the reader uses
    pub int_val: i64,
    pub float_val: f64,
}

impl Default for PersistSlot {
    fn default() -> Self {
        PersistSlot {
            tag: 0,
            int_val: 0,
            float_val: 0.0,
        }
    }
}

// --- EMA State (Exponential Moving Average) ---
// Used by the vm_ema opcode for fused feature computation.

#[derive(Debug, Clone, Copy)]
pub struct EmaState {
    pub alpha: f64,          // Smoothing factor (set at compile time from program.ema_alphas)
    pub value: f64,          // Current EMA value
    pub initialized: bool,   // False until first value pushed (seeds with raw value)
}

impl Default for EmaState {
    fn default() -> Self {
        EmaState {
            alpha: 0.0,
            value: 0.0,
            initialized: false,
        }
    }
}

// --- Rolling Window Metadata ---
// Each window is a ring buffer inside window_arena. Metadata tracks aggregates
// and O(1) min/max via monotonic deques (indices into the ring buffer).

#[derive(Debug, Clone, Copy)]
pub struct WindowMeta {
    pub offset: u16,     // Start index into window_arena for this window
    pub capacity: u16,   // Ring buffer capacity (typically 64)
    pub head: u16,       // Current write position (ring index)
    pub len: u16,        // Number of elements currently in window
    pub sum: f64,        // Running sum (for O(1) mean)
    pub sum_sq: f64,     // Running sum of squares (for O(1) variance/stddev)
    pub min: f64,        // Current minimum value in window
    pub max: f64,        // Current maximum value in window
    // O(1) sliding min/max via monotonic deque (indices into ring buffer)
    pub min_deque: [u8; 64],  // Fixed-size deque tracking indices of increasing minima
    pub max_deque: [u8; 64],  // Fixed-size deque tracking indices of decreasing maxima
    pub min_dq_front: u8,     // Front pointer for min deque (circular buffer in [0,64))
    pub min_dq_back: u8,      // Back pointer for min deque
    pub max_dq_front: u8,     // Front pointer for max deque
    pub max_dq_back: u8,      // Back pointer for max deque
}

impl Default for WindowMeta {
    fn default() -> Self {
        WindowMeta {
            offset: 0,
            capacity: 0,
            head: 0,
            len: 0,
            sum: 0.0,
            sum_sq: 0.0,
            min: 0.0,
            max: 0.0,
            min_deque: [0u8; 64],
            max_deque: [0u8; 64],
            min_dq_front: 0,
            min_dq_back: 0,
            max_dq_front: 0,
            max_dq_back: 0,
        }
    }
}

// --- WindowMeta Computed Properties ---

impl WindowMeta {
    // Returns the arithmetic mean of elements currently in the window.
    pub fn mean(&self) -> f64 {
        if self.len == 0 {
            0.0
        } else {
            self.sum / self.len as f64
        }
    }
    // Returns the population variance of elements currently in the window.
    pub fn variance(&self) -> f64 {
        if self.len < 2 {
            return 0.0;
        }
        let n = self.len as f64;
        (self.sum_sq - self.sum * self.sum / n) / n
    }
    // Returns the population standard deviation.
    pub fn stddev(&self) -> f64 {
        self.variance().sqrt()
    }
    // Returns the running sum.
    pub fn sum(&self) -> f64 {
        self.sum
    }
    // Returns the current minimum (0 if empty).
    pub fn min(&self) -> f64 {
        if self.len == 0 {
            0.0
        } else {
            self.min
        }
    }
    // Returns the current maximum (0 if empty).
    pub fn max(&self) -> f64 {
        if self.len == 0 {
            0.0
        } else {
            self.max
        }
    }
}

// --- Cold VM data ---
// Behind a `Box` pointer so hot fields stay cache-friendly.
// These large arrays (~30+ KB) live in L2/L3, not L1.

#[derive(Debug)]
#[repr(C)]
pub struct ColdVm {
    // Large flat arrays (pushed out of L1, behind Box)
    pub indicators: [f64; MAX_INDICATORS],           // Named indicator values by slot
    pub indicator_by_str: Vec<u16>,                   // String constant index → indicator slot
    pub balances: [f64; MAX_BALANCES],                // Named balance values by slot
    pub balance_by_str: Vec<u16>,                     // String constant index → balance slot

    // Depth book (order book snapshots)
    pub depth_bids_price: [f64; MAX_DEPTH_LEVELS],    // Bid prices per level
    pub depth_bids_qty: [f64; MAX_DEPTH_LEVELS],      // Bid quantities per level
    pub depth_asks_price: [f64; MAX_DEPTH_LEVELS],    // Ask prices per level
    pub depth_asks_qty: [f64; MAX_DEPTH_LEVELS],      // Ask quantities per level
    pub depth_bids_len: u8,                           // Number of valid bid levels
    pub depth_asks_len: u8,                           // Number of valid ask levels

    // Persist (hot-reload safe state, snapshot/restore)
    pub persist: [PersistSlot; PERSIST_SLOTS],

    // Rolling windows (ring-buffer arena + per-window metadata)
    pub window_arena: Vec<f64>,                       // Flat ring-buffer storage for all windows
    pub window_meta: [WindowMeta; MAX_WINDOWS],       // Metadata for each window

    // Fused feature states
    pub ema_states: [EmaState; MAX_EMA_STATES],       // EMA state slots

    // Ownership holders — backing allocations for raw pointers held in hot Vm.
    // These Vecs must NOT be modified after Vm construction (raw pointers into them).
    pub _code_owned: Vec<u64>,
    pub _consts_owned: Vec<f64>,
    pub _i64_consts_owned: Vec<i64>,

    // Constants pool (backward compat — string constants used by log, getind, getbal)
    pub const_pool: Vec<ConstEntry>,
    pub const_strings: Vec<String>,

    // Name→slot mapping (hash-based, O(1) lookup instead of O(n) linear scan)
    pub indicator_map: HashMap<String, u16>,
    pub balance_map: HashMap<String, u16>,

    // Profiler / Tracer (optional instrumentation)
    pub profiler: Option<crate::profiler::Profiler>,
    pub tracer: Option<crate::tracer::Tracer>,

    // VM Trace (full instruction-level trace to file)
    pub trace_vm_enabled: bool,
    pub trace_file: Option<std::io::BufWriter<std::fs::File>>,
    pub trace_start: std::time::Instant,
}

// --- Hot VM Data ---
// Flat VM — Vec/HashMap only in setup paths, NOT in execute_tick hot path.
// Hot fields are declared first, cold fields behind `Box<ColdVm>`.

#[derive(Debug)]
#[repr(C)]
pub struct Vm {
    // ════════════════════════════════════════════════════════════════
    //  HOT PATH — accessed by almost every instruction in run()
    // ════════════════════════════════════════════════════════════════
    // Register file (2048 bytes, fits in L1)
    pub regs: [Register; NUM_REGS],

    // Execution state
    pub pc: usize,                                    // Program counter (index into code_ptr)
    pub running: bool,                                // True while dispatch loop should continue
    pub call_stack: [usize; MAX_CALL_DEPTH],           // Return addresses for CALL/RET
    pub call_depth: u8,                               // Current call depth

    // Code + constants — raw pointers into owned backing in ColdVm
    // Used in lieu of Vec indexing to avoid bounds checks in hot path.
    pub code_ptr: *const u64,            // Pointer to program bytecode (u64 instructions)
    pub code_len: usize,                 // Number of instructions
    pub consts_ptr: *const f64,          // Pointer to f64 constant pool
    pub const_count: u32,                // Number of f64 constants
    pub i64_consts_ptr: *const i64,      // Pointer to i64 constant pool
    pub i64_const_count: u32,            // Number of i64 constants

    // Scalar engine state (hot — updated by external engine each tick)
    pub last_price: f64,                 // Most recent price
    pub position_size: f64,              // Current position size
    pub has_pending_order: bool,         // Set true by SEND_ORDER; cleared by engine

    // Entry points (warm, but small — ~72 bytes)
    pub entry_names: [u64; 8],          // Entry point names (packed as u64, up to 8 chars)
    pub entry_offsets: [u32; 8],        // Corresponding code offsets
    pub entry_count: u8,                // Number of registered entry points
    handler_cache: [u32; 4],            // Cached offsets for 4 standard handlers (no linear scan)

    // Cold data (behind pointer, ~30+ KB out of hot cache line)
    pub cold: Box<ColdVm>,
}

impl Vm {
    // --- Constructor ---
    // Builds a Vm from a compiled QfrProgram. Copies code/constants into owned Vecs
    // and sets up raw pointers, entry-point cache, handler cache, and initial state.

    pub fn new(program: QfrProgram) -> Self {
        // Take ownership of compiled code and constants
        let _code_owned: Vec<u64> = program.code.iter().map(|i| i.raw()).collect();
        let code_ptr = _code_owned.as_ptr();
        let code_len = _code_owned.len();

        let _consts_owned = program.f64_consts.clone();
        let _i64_consts_owned = program.i64_consts.clone();
        let const_strings = program.string_consts.clone();
        let consts_ptr = _consts_owned.as_ptr();
        let const_count = _consts_owned.len() as u32;
        let i64_consts_ptr = _i64_consts_owned.as_ptr();
        let i64_const_count = _i64_consts_owned.len() as u32;

        // Pack entry point names into u64 for fast comparison
        let mut entry_names = [0u64; 8];
        let mut entry_offsets = [0u32; 8];
        let entry_count = program.entries.len().min(8) as u8;
        for (i, e) in program
            .entries
            .iter()
            .enumerate()
            .take(entry_count as usize)
        {
            let mut name_bytes = [0u8; 8];
            let src = e.name.as_bytes();
            let n = src.len().min(8);
            name_bytes[..n].copy_from_slice(&src[..n]);
            entry_names[i] = u64::from_le_bytes(name_bytes);
            entry_offsets[i] = e.code_offset;
        }

        // Initialize lookup tables — filled later during registration phase
        let indicator_by_str = vec![u16::MAX; const_strings.len()];
        let balance_by_str = vec![u16::MAX; const_strings.len()];

        // Pre-compute handler cache: no linear scan needed in hot path
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

        // Assemble the Vm with all default state
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
            last_price: 0.0,
            position_size: 0.0,
            has_pending_order: false,
            entry_names,
            entry_offsets,
            entry_count,
            handler_cache,
            cold: Box::new(ColdVm {
                _code_owned,
                _consts_owned,
                _i64_consts_owned,
                const_pool: program.const_pool.clone(),
                const_strings,
                indicators: [0.0; MAX_INDICATORS],
                indicator_by_str,
                indicator_map: HashMap::new(),
                balances: [0.0; MAX_BALANCES],
                balance_by_str,
                balance_map: HashMap::new(),
                depth_bids_price: [0.0; MAX_DEPTH_LEVELS],
                depth_bids_qty: [0.0; MAX_DEPTH_LEVELS],
                depth_asks_price: [0.0; MAX_DEPTH_LEVELS],
                depth_asks_qty: [0.0; MAX_DEPTH_LEVELS],
                depth_bids_len: 0,
                depth_asks_len: 0,
                persist: [PersistSlot::default(); PERSIST_SLOTS],
                window_arena: vec![0.0; WINDOW_ARENA_SIZE],
                window_meta: [WindowMeta::default(); MAX_WINDOWS],
                ema_states: [EmaState::default(); MAX_EMA_STATES],
                profiler: None,
                tracer: None,
                trace_vm_enabled: false,
                trace_file: None,
                trace_start: std::time::Instant::now(),
            }),
        };
        // Initialize EMA alphas from the compiled program
        for (i, alpha) in program.ema_alphas.iter().enumerate() {
            if i < MAX_EMA_STATES {
                vm.cold.ema_states[i].alpha = *alpha;
            }
        }
        vm
    }

    // --- Register Access Helpers ---
    // Unsafe getters/setters with unchecked indexing (hot-path).

    #[inline(always)]
    pub fn set_int(&mut self, reg: u8, val: i64) {
        unsafe {
            *self.regs.get_unchecked_mut(reg as usize) = Register::from_i64(val);
        }
    }

    #[inline(always)]
    pub fn set_float(&mut self, reg: u8, val: f64) {
        unsafe {
            *self.regs.get_unchecked_mut(reg as usize) = Register::from_f64(val);
        }
    }

    #[inline(always)]
    pub fn reg_i(&self, idx: usize) -> i64 {
        unsafe { self.regs.get_unchecked(idx).i }
    }

    #[inline(always)]
    pub fn reg_f(&self, idx: usize) -> f64 {
        unsafe { self.regs.get_unchecked(idx).f }
    }

    #[inline(always)]
    pub fn int(&self, reg: u8) -> i64 {
        unsafe { self.regs.get_unchecked(reg as usize).i }
    }

    #[inline(always)]
    pub fn float(&self, reg: u8) -> f64 {
        unsafe { self.regs.get_unchecked(reg as usize).f }
    }

    // --- Entry Point Execution ---
    // Called externally by the engine to execute a named handler (e.g. "on_trade").
    // Looks up the offset (via cache or linear scan), then enters the dispatch loop.

    pub fn call(&mut self, entry_name: &str) {
        const HANDLER_LABELS: [&str; 4] = ["on_trade", "on_eval", "on_fill", "on_depth"];
        // Check handler cache first (fast path for standard handlers)
        let offset = if let Some(idx) = HANDLER_LABELS.iter().position(|&n| n == entry_name) {
            let cached = self.handler_cache[idx];
            if cached != u32::MAX {
                cached as usize
            } else {
                return; // Handler not found — nothing to execute
            }
        } else {
            // Fall back to linear scan of all entry points
            match self.entry_offset(entry_name) {
                Some(o) => o as usize,
                None => return,
            }
        };
        // Profiler: record handler entry
        if let Some(ref mut p) = self.cold.profiler {
            p.start_handler(entry_name);
        }
        self.pc = offset;
        self.running = true;
        self.call_depth = 0;
        self.run();
        if let Some(ref mut p) = self.cold.profiler {
            p.end_handler();
        }
    }

    // Linear search for an entry point by name (up to 8-char prefix match).
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

    // --- Dispatch Loop (Bare) ---
    // Fastest execution mode: no profiling, no tracing, no VM trace.
    // Fetches a u64 instruction, extracts opcode for jump-table dispatch, advances PC.

    #[inline(always)]
    fn run_bare(&mut self) {
        while self.running {
            let instr = unsafe { *self.code_ptr.add(self.pc) };
            self.pc += 1;
            unsafe {
                let handler = *JUMP_TABLE.get_unchecked((instr & 0xFF) as usize);
                handler(self, instr);
            }
        }
    }

    // --- Dispatch Loop (Profiled) ---
    // Same as bare but records opcode frequency via the profiler.

    #[inline(never)]
    fn run_profiled(&mut self) {
        while self.running {
            let instr = unsafe { *self.code_ptr.add(self.pc) };
            self.pc += 1;
            let opcode = (instr & 0xFF) as u8;
            if let Some(ref mut p) = self.cold.profiler {
                p.record_opcode(crate::opcodes::Opcode::from_u8(opcode));
            }
            unsafe {
                let handler = *JUMP_TABLE.get_unchecked(opcode as usize);
                handler(self, instr);
            }
        }
    }

    // --- Dispatch Loop (Traced) ---
    // Same as bare but calls trace_op after each instruction (for instruction-level tracing).

    #[inline(never)]
    fn run_traced(&mut self) {
        while self.running {
            let instr = unsafe { *self.code_ptr.add(self.pc) };
            self.pc += 1;
            let opcode = (instr & 0xFF) as u8;
            unsafe {
                let handler = *JUMP_TABLE.get_unchecked(opcode as usize);
                handler(self, instr);
            }
            self.trace_op(opcode, instr);
        }
    }

    // --- Dispatch Loop (VM Trace) ---
    // Full register-state trace written to a file after each instruction.

    #[inline(never)]
    fn run_with_tracevm(&mut self) {
        while self.running {
            let instr = unsafe { *self.code_ptr.add(self.pc) };
            self.pc += 1;
            let opcode = (instr & 0xFF) as u8;
            unsafe {
                let handler = *JUMP_TABLE.get_unchecked(opcode as usize);
                handler(self, instr);
            }
            self.trace_vm_instruction(opcode, instr);
        }
    }

    // Selects the appropriate dispatch loop based on profiling/tracing configuration.

    #[inline(always)]
    fn run(&mut self) {
        let has_profiler = self.cold.profiler.is_some();
        let has_tracer = self.cold.tracer.is_some();
        if self.cold.trace_vm_enabled {
            self.run_with_tracevm();
        } else if has_profiler {
            self.run_profiled();
        } else if has_tracer {
            self.run_traced();
        } else {
            self.run_bare();
        }
    }

    // Stub: placeholder for trace_op callback (currently unused by default tracer).
    fn trace_op(&mut self, _opcode: u8, _instr: u64) {}

    // Writes a detailed instruction trace line to the VM trace file.
    // Includes: timestamp (us), PC, opcode name, register values, and immediate.
    fn trace_vm_instruction(&mut self, opcode: u8, instr: u64) {
        use crate::opcodes::Opcode;
        let dt = self.cold.trace_start.elapsed();
        let dt_us = dt.as_secs_f64() * 1_000_000.0;
        let rd = ((instr >> 8) & 0xFF) as u8;
        let rs1 = ((instr >> 16) & 0xFF) as u8;
        let rs2 = ((instr >> 24) & 0xFF) as u8;
        let imm = (instr >> 32) as i32;
        let op = Opcode::from_u8(opcode);
        // Format register values: float-pretty for regs >= 192, raw int otherwise
        let fmt_reg = |r: u8| -> String {
            if r >= 192 {
                format!("{:.4}", unsafe { self.regs.get_unchecked(r as usize).f })
            } else {
                format!("{}", unsafe { self.regs.get_unchecked(r as usize).i })
            }
        };
        let rds = fmt_reg(rd);
        let r1s = fmt_reg(rs1);
        let r2s = fmt_reg(rs2);
        let trace_line = format!(
            "{:.3} PC={} {} rd={}({}) rs1={}({}) rs2={}({}) imm={}\n",
            dt_us,
            self.pc - 1,
            op,
            rd,
            rds,
            rs1,
            r1s,
            rs2,
            r2s,
            imm
        );
        if let Some(ref mut f) = self.cold.trace_file {
            let _ = f.write(trace_line.as_bytes());
        }
    }

    // Opens a file and enables VM trace mode. All future instruction executions
    // will be logged with register state to the given path.

    pub fn enable_trace_vm(&mut self, path: &str) {
        match std::fs::File::create(path) {
            Ok(file) => {
                self.cold.trace_file = Some(std::io::BufWriter::new(file));
                self.cold.trace_vm_enabled = true;
                self.cold.trace_start = std::time::Instant::now();
            }
            Err(e) => {
                eprintln!("[vm] failed to open trace file '{}': {}", path, e);
            }
        }
    }

    // --- State Setters ---
    // Called by the engine before each tick to push external state into the VM.

    // Sets a named balance value. Grows the balance list if this is a new name.
    pub fn set_balance(&mut self, asset: &str, val: f64) {
        if let Some(&slot) = self.cold.balance_map.get(asset) {
            if (slot as usize) < MAX_BALANCES {
                self.cold.balances[slot as usize] = val;
            }
            return;
        }
        let slot = self.cold.balance_map.len() as u16;
        if (slot as usize) < MAX_BALANCES {
            self.cold.balances[slot as usize] = val;
        }
        self.cold.balance_map.insert(asset.to_string(), slot);
    }

    pub fn set_last_price(&mut self, price: f64) {
        self.last_price = price;
    }

    pub fn set_position_size(&mut self, size: f64) {
        self.position_size = size;
    }

    // Copies ordered bid levels into the depth book.
    pub fn set_depth_bids(&mut self, bids: &[quince_core::types::DepthLevel]) {
        let n = bids.len().min(MAX_DEPTH_LEVELS);
        for i in 0..n {
            self.cold.depth_bids_price[i] = bids[i].price;
            self.cold.depth_bids_qty[i] = bids[i].qty;
        }
        self.cold.depth_bids_len = n as u8;
    }

    // Copies ordered ask levels into the depth book.
    pub fn set_depth_asks(&mut self, asks: &[quince_core::types::DepthLevel]) {
        let n = asks.len().min(MAX_DEPTH_LEVELS);
        for i in 0..n {
            self.cold.depth_asks_price[i] = asks[i].price;
            self.cold.depth_asks_qty[i] = asks[i].qty;
        }
        self.cold.depth_asks_len = n as u8;
    }

    // Sets a named indicator value. Grows the indicator list if new.
    pub fn set_indicator(&mut self, name: &str, val: f64) {
        if let Some(&slot) = self.cold.indicator_map.get(name) {
            if (slot as usize) < MAX_INDICATORS {
                self.cold.indicators[slot as usize] = val;
            }
            return;
        }
        let slot = self.cold.indicator_map.len() as u16;
        if (slot as usize) < MAX_INDICATORS {
            self.cold.indicators[slot as usize] = val;
        }
        self.cold.indicator_map.insert(name.to_string(), slot);
    }

    // Returns the slot for a named indicator, creating it if it doesn't exist.
    pub fn ensure_indicator_slot(&mut self, name: &str) -> u16 {
        if let Some(&slot) = self.cold.indicator_map.get(name) {
            return slot;
        }
        let slot = self.cold.indicator_map.len() as u16;
        self.cold.indicator_map.insert(name.to_string(), slot);
        slot
    }

    // Looks up the slot for a named indicator (returns None if not found).
    pub fn indicator_slot(&self, name: &str) -> Option<u16> {
        self.cold.indicator_map.get(name).copied()
    }

    // Returns the slot for a named balance, creating it if it doesn't exist.
    pub fn ensure_balance_slot(&mut self, name: &str) -> u16 {
        if let Some(&slot) = self.cold.balance_map.get(name) {
            return slot;
        }
        let slot = self.cold.balance_map.len() as u16;
        self.cold.balance_map.insert(name.to_string(), slot);
        slot
    }

    // Sets a balance value by its slot index (no name lookup).
    pub fn set_balance_by_slot(&mut self, slot: u16, val: f64) {
        let idx = slot as usize;
        debug_assert!(idx < MAX_BALANCES);
        unsafe {
            *self.cold.balances.get_unchecked_mut(idx) = val;
        }
    }

    pub fn set_indicator_by_slot(&mut self, slot: u16, val: f64) {
        let idx = slot as usize;
        debug_assert!(idx < MAX_INDICATORS);
        unsafe {
            *self.cold.indicators.get_unchecked_mut(idx) = val;
        }
    }

    // Phase 4g: sets alpha (smoothing factor) for a given EMA state slot.
    pub fn set_ema_alpha(&mut self, state_id: u8, alpha: f64) {
        if (state_id as usize) < MAX_EMA_STATES {
            self.cold.ema_states[state_id as usize].alpha = alpha;
        }
    }

    // Rebuilds indicator_by_str / balance_by_str from the current name→slot lists.
    // Must be called after all indicator/balance registrations (e.g. from Engine).
    // This allows vm_getind / vm_getbal opcodes to resolve string constants to slots
    // in O(1) via the string-constant index.
    pub fn finalize_const_lookups(&mut self) {
        self.cold.indicator_by_str.clear();
        self.cold
            .indicator_by_str
            .resize(self.cold.const_strings.len(), u16::MAX);
        for (str_idx, s) in self.cold.const_strings.iter().enumerate() {
            if let Some(&slot) = self.cold.indicator_map.get(s) {
                if str_idx < self.cold.indicator_by_str.len() {
                    self.cold.indicator_by_str[str_idx] = slot;
                }
            }
        }
        self.cold.balance_by_str.clear();
        self.cold
            .balance_by_str
            .resize(self.cold.const_strings.len(), u16::MAX);
        for (str_idx, s) in self.cold.const_strings.iter().enumerate() {
            if let Some(&slot) = self.cold.balance_map.get(s) {
                if str_idx < self.cold.balance_by_str.len() {
                    self.cold.balance_by_str[str_idx] = slot;
                }
            }
        }
    }

    // --- Snapshot / Restore (hot-reload) ---
    // Captures and restores the full VM execution state across hot-reload cycles.

    // Creates a snapshot of the VM state (regs, persist, pc, indicators, balances).
    pub fn snapshot(&self) -> VmSnapshot {
        VmSnapshot {
            regs: self.regs,
            persist: self.cold.persist,
            pc: self.pc,
            indicators: self.cold.indicators,
            balances: self.cold.balances,
        }
    }

    // Restores VM state from a previous snapshot.
    pub fn restore(&mut self, snap: &VmSnapshot) {
        self.regs = snap.regs;
        self.cold.persist = snap.persist;
        self.pc = snap.pc;
        self.cold.indicators = snap.indicators;
        self.cold.balances = snap.balances;
    }

    // Decodes all instructions in the code buffer (for inspection/debugging).
    pub fn code_instr(&self) -> Vec<Instruction> {
        let mut v = Vec::with_capacity(self.code_len);
        for i in 0..self.code_len {
            let raw = unsafe { *self.code_ptr.add(i) };
            v.push(Instruction::decode(&raw.to_le_bytes()));
        }
        v
    }
}

// --- Snapshot Struct ---
// Holds the portion of VM state that survives hot-reload.

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
// Each opcode handler is a function pointer in JUMP_TABLE.
// Handlers follow the signature: fn(&mut Vm, instr: u64)

// --- Instruction Field Extractors ---
// These unsafe helpers extract bit fields from a packed u64 instruction.
// Layout: [opcode:8][rd:8][rs1:8][rs2:8][imm:32]

#[inline(always)]
unsafe fn rd(instr: u64) -> u8 {
    ((instr >> 8) & 0xFF) as u8  // Destination register
}
#[inline(always)]
unsafe fn rs1(instr: u64) -> u8 {
    ((instr >> 16) & 0xFF) as u8  // First source register
}
#[inline(always)]
unsafe fn rs2(instr: u64) -> u8 {
    ((instr >> 24) & 0xFF) as u8  // Second source register (or extra data)
}
#[inline(always)]
unsafe fn imm(instr: u64) -> i32 {
    (instr >> 32) as i32  // Signed 32-bit immediate
}
#[inline(always)]
unsafe fn immu(instr: u64) -> u32 {
    (instr >> 32) as u32  // Unsigned 32-bit immediate
}

// NaN/Inf sanitizer — debug-only assertion that float values are finite.
#[inline(always)]
fn sanitize_f(val: f64) -> f64 {
    debug_assert!(val.is_finite(), "NaN/Inf in float register");
    val
}

// --- Opcode Handlers ---

pub mod handlers {
    use super::*;

    // ── Int arithmetic ──
    // All integer ops use wrapping arithmetic (no overflow panic).

    // rd = rs1 + rs2 (wrapping add)
    #[inline(always)]
    pub unsafe fn vm_add(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        debug_assert!((rs2(instr) as usize) < NUM_REGS);
        let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
        let b = vm.regs.get_unchecked(rs2(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = a.wrapping_add(b);
    }

    // rd = rs1 - rs2 (wrapping sub)
    #[inline(always)]
    pub unsafe fn vm_sub(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        debug_assert!((rs2(instr) as usize) < NUM_REGS);
        vm.regs.get_unchecked_mut(r).i = vm
            .regs
            .get_unchecked(rs1(instr) as usize)
            .i
            .wrapping_sub(vm.regs.get_unchecked(rs2(instr) as usize).i);
    }

    // rd = rs1 * rs2 (wrapping mul)
    #[inline(always)]
    pub unsafe fn vm_mul(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        debug_assert!((rs2(instr) as usize) < NUM_REGS);
        vm.regs.get_unchecked_mut(r).i = vm
            .regs
            .get_unchecked(rs1(instr) as usize)
            .i
            .wrapping_mul(vm.regs.get_unchecked(rs2(instr) as usize).i);
    }

    // rd = rs1 / rs2 (returns 0 on division by zero)
    #[inline(always)]
    pub unsafe fn vm_div(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        debug_assert!((rs2(instr) as usize) < NUM_REGS);
        let divisor = vm.regs.get_unchecked(rs2(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = if divisor == 0 {
            0
        } else {
            vm.regs.get_unchecked(rs1(instr) as usize).i / divisor
        };
    }

    // rd = rs1 % rs2 (returns 0 on division by zero)
    #[inline(always)]
    pub unsafe fn vm_mod(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        debug_assert!((rs2(instr) as usize) < NUM_REGS);
        let divisor = vm.regs.get_unchecked(rs2(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = if divisor == 0 {
            0
        } else {
            vm.regs.get_unchecked(rs1(instr) as usize).i % divisor
        };
    }

    // rd = -rs1 (wrapping neg)
    #[inline(always)]
    pub unsafe fn vm_neg(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        vm.regs.get_unchecked_mut(r).i =
            vm.regs.get_unchecked(rs1(instr) as usize).i.wrapping_neg();
    }

    // rd = rs1 + imm (wrapping add immediate)
    #[inline(always)]
    pub unsafe fn vm_addi(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        vm.regs.get_unchecked_mut(r).i = vm
            .regs
            .get_unchecked(rs1(instr) as usize)
            .i
            .wrapping_add(imm(instr) as i64);
    }

    // rd = rs1 - imm (wrapping sub immediate)
    #[inline(always)]
    pub unsafe fn vm_subi(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        vm.regs.get_unchecked_mut(r).i = vm
            .regs
            .get_unchecked(rs1(instr) as usize)
            .i
            .wrapping_sub(imm(instr) as i64);
    }

    // rd = rs1 * imm (wrapping mul immediate)
    #[inline(always)]
    pub unsafe fn vm_muli(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        vm.regs.get_unchecked_mut(r).i = vm
            .regs
            .get_unchecked(rs1(instr) as usize)
            .i
            .wrapping_mul(imm(instr) as i64);
    }

    // rd = rs1 / imm (returns 0 on division by zero)
    #[inline(always)]
    pub unsafe fn vm_divi(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        let divisor = imm(instr) as i64;
        vm.regs.get_unchecked_mut(r).i = if divisor == 0 {
            0
        } else {
            vm.regs.get_unchecked(rs1(instr) as usize).i / divisor
        };
    }

    // ── Float arithmetic ──

    // Macro that generates a float binary-op handler: a $op b
    macro_rules! float_binop {
        ($name:ident, $op:tt) => {
            #[inline(always)]
pub unsafe fn $name(vm: &mut Vm, instr: u64) {
                let r = rd(instr) as usize;
                debug_assert!(r < NUM_REGS);
                debug_assert!((rs1(instr) as usize) < NUM_REGS);
                debug_assert!((rs2(instr) as usize) < NUM_REGS);
                let a = vm.regs.get_unchecked(rs1(instr) as usize).f;
                let b = vm.regs.get_unchecked(rs2(instr) as usize).f;
                vm.regs.get_unchecked_mut(r).f = sanitize_f(a $op b);
            }
        };
    }

    float_binop!(vm_fadd, +);  // rd = rs1 + rs2 (float)
    float_binop!(vm_fsub, -);  // rd = rs1 - rs2 (float)
    float_binop!(vm_fmul, *);  // rd = rs1 * rs2 (float)

    // rd = rs1 / rs2 (float, returns 0.0 on division by zero)
    #[inline(always)]
    pub unsafe fn vm_fdiv(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let divisor = vm.regs.get_unchecked(rs2(instr) as usize).f;
        vm.regs.get_unchecked_mut(r).f = if divisor == 0.0 {
            0.0
        } else {
            sanitize_f(vm.regs.get_unchecked(rs1(instr) as usize).f / divisor)
        };
    }

    // rd = -rs1 (float negate)
    #[inline(always)]
    pub unsafe fn vm_fneg(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        vm.regs.get_unchecked_mut(r).f = sanitize_f(-vm.regs.get_unchecked(rs1(instr) as usize).f);
    }

    // ── Int comparison ──
    // Sets rd = 1 if comparison is true, 0 otherwise.

    macro_rules! int_cmp {
        ($name:ident, $cmp:tt) => {
            #[inline(always)]
pub unsafe fn $name(vm: &mut Vm, instr: u64) {
                let r = rd(instr) as usize;
                debug_assert!(r < NUM_REGS);
                debug_assert!((rs1(instr) as usize) < NUM_REGS);
                debug_assert!((rs2(instr) as usize) < NUM_REGS);
                let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
                let b = vm.regs.get_unchecked(rs2(instr) as usize).i;
                vm.regs.get_unchecked_mut(r).i = if a $cmp b { 1 } else { 0 };
            }
        };
    }

    int_cmp!(vm_eq, ==);   // rd = (rs1 == rs2) ? 1 : 0
    int_cmp!(vm_ne, !=);   // rd = (rs1 != rs2) ? 1 : 0
    int_cmp!(vm_lt, <);    // rd = (rs1 <  rs2) ? 1 : 0
    int_cmp!(vm_gt, >);    // rd = (rs1 >  rs2) ? 1 : 0
    int_cmp!(vm_le, <=);   // rd = (rs1 <= rs2) ? 1 : 0
    int_cmp!(vm_ge, >=);   // rd = (rs1 >= rs2) ? 1 : 0

    // ── Float comparison ──
    // Sets rd = 1 if comparison is true, 0 otherwise. Result is i64.

    macro_rules! float_cmp {
        ($name:ident, $cmp:tt) => {
            #[inline(always)]
pub unsafe fn $name(vm: &mut Vm, instr: u64) {
                let r = rd(instr) as usize;
                debug_assert!(r < NUM_REGS);
                debug_assert!((rs1(instr) as usize) < NUM_REGS);
                debug_assert!((rs2(instr) as usize) < NUM_REGS);
                let a = vm.regs.get_unchecked(rs1(instr) as usize).f;
                let b = vm.regs.get_unchecked(rs2(instr) as usize).f;
                vm.regs.get_unchecked_mut(r).i = if a $cmp b { 1 } else { 0 };
            }
        };
    }

    float_cmp!(vm_feq, ==);   // rd = (rs1 == rs2) ? 1 : 0 (float)
    float_cmp!(vm_fne, !=);   // rd = (rs1 != rs2) ? 1 : 0 (float)
    float_cmp!(vm_flt, <);    // rd = (rs1 <  rs2) ? 1 : 0 (float)
    float_cmp!(vm_fgt, >);    // rd = (rs1 >  rs2) ? 1 : 0 (float)
    float_cmp!(vm_fle, <=);   // rd = (rs1 <= rs2) ? 1 : 0 (float)
    float_cmp!(vm_fge, >=);   // rd = (rs1 >= rs2) ? 1 : 0 (float)

    // ── Immediate comparison ──

    // rd = (rs1 == imm) ? 1 : 0
    #[inline(always)]
    pub unsafe fn vm_eqi(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = if a == imm(instr) as i64 { 1 } else { 0 };
    }

    // rd = (rs1 < imm) ? 1 : 0
    #[inline(always)]
    pub unsafe fn vm_lti(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = if a < imm(instr) as i64 { 1 } else { 0 };
    }

    // rd = (rs1 > imm) ? 1 : 0
    #[inline(always)]
    pub unsafe fn vm_gti(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = if a > imm(instr) as i64 { 1 } else { 0 };
    }

    // ── Bitwise operations ──

    // Macro that generates a bitwise binary-op handler.
    macro_rules! bitwise_binop {
        ($name:ident, $op:tt) => {
            #[inline(always)]
pub unsafe fn $name(vm: &mut Vm, instr: u64) {
                let r = rd(instr) as usize;
                debug_assert!(r < NUM_REGS);
                debug_assert!((rs1(instr) as usize) < NUM_REGS);
                debug_assert!((rs2(instr) as usize) < NUM_REGS);
                let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
                let b = vm.regs.get_unchecked(rs2(instr) as usize).i;
                vm.regs.get_unchecked_mut(r).i = a $op b;
            }
        };
    }

    bitwise_binop!(vm_bitand, &);   // rd = rs1 & rs2
    bitwise_binop!(vm_bitor, |);    // rd = rs1 | rs2
    bitwise_binop!(vm_bitxor, ^);   // rd = rs1 ^ rs2

    // rd = ~rs1 (bitwise NOT)
    #[inline(always)]
    pub unsafe fn vm_bitnot(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        vm.regs.get_unchecked_mut(r).i = !vm.regs.get_unchecked(rs1(instr) as usize).i;
    }

    // rd = rs1 << rs2 (wrapping shift left)
    #[inline(always)]
    pub unsafe fn vm_shl(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        debug_assert!((rs2(instr) as usize) < NUM_REGS);
        let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
        let b = vm.regs.get_unchecked(rs2(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = a.wrapping_shl(b as u32);
    }

    // rd = rs1 >> rs2 (logical shift right, zero-extend)
    #[inline(always)]
    pub unsafe fn vm_shr(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        debug_assert!((rs2(instr) as usize) < NUM_REGS);
        let a = vm.regs.get_unchecked(rs1(instr) as usize).i as u64;
        let b = vm.regs.get_unchecked(rs2(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = a.wrapping_shr(b as u32) as i64;
    }

    // ── Control flow ──

    // Unconditional jump: PC += imm
    #[inline(always)]
    pub unsafe fn vm_jmp(vm: &mut Vm, instr: u64) {
        let target = (vm.pc as i64).wrapping_add(imm(instr) as i64) as usize;
        debug_assert!(
            target < vm.code_len,
            "jmp target {target} >= code_len {}",
            vm.code_len
        );
        vm.pc = target;
    }

    // Conditional jump: PC += imm if rs1 == 0
    #[inline(always)]
    pub unsafe fn vm_jz(vm: &mut Vm, instr: u64) {
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        if vm.regs.get_unchecked(rs1(instr) as usize).i == 0 {
            let target = (vm.pc as i64).wrapping_add(imm(instr) as i64) as usize;
            debug_assert!(
                target < vm.code_len,
                "jz target {target} >= code_len {}",
                vm.code_len
            );
            vm.pc = target;
        }
    }

    // Conditional jump: PC += imm if rs1 != 0
    #[inline(always)]
    pub unsafe fn vm_jnz(vm: &mut Vm, instr: u64) {
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        if vm.regs.get_unchecked(rs1(instr) as usize).i != 0 {
            let target = (vm.pc as i64).wrapping_add(imm(instr) as i64) as usize;
            debug_assert!(
                target < vm.code_len,
                "jnz target {target} >= code_len {}",
                vm.code_len
            );
            vm.pc = target;
        }
    }

    // Subroutine call: push return address onto call_stack, then PC += imm
    #[inline(always)]
    pub unsafe fn vm_call(vm: &mut Vm, instr: u64) {
        let depth = vm.call_depth as usize;
        debug_assert!(
            depth < MAX_CALL_DEPTH,
            "call_depth {depth} >= MAX_CALL_DEPTH"
        );
        if depth < MAX_CALL_DEPTH {
            *vm.call_stack.get_unchecked_mut(depth) = vm.pc;
            vm.call_depth += 1;
        } else {
            vm.running = false;
        }
        let target = (vm.pc as i64).wrapping_add(imm(instr) as i64) as usize;
        debug_assert!(
            target < vm.code_len,
            "call target {target} >= code_len {}",
            vm.code_len
        );
        vm.pc = target;
    }

    // Return from subroutine: pop return address from call_stack; if depth 0, halt.
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

    // rd = rs1 (copy register value, preserves union type)
    #[inline(always)]
    pub unsafe fn vm_mov(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let s = rs1(instr) as usize;
        *vm.regs.get_unchecked_mut(r) = *vm.regs.get_unchecked(s);
    }

    // rd = imm (load 32-bit signed immediate, sign-extended to i64)
    #[inline(always)]
    pub unsafe fn vm_ldi(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        vm.regs.get_unchecked_mut(r).i = imm(instr) as i64;
    }

    // rd = 40-bit immediate encoded across imm(32) + rs2(8), sign-extended to i64
    #[inline(always)]
    pub unsafe fn vm_ldi64(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let low = (instr >> 32) as u64; // imm = bits 32-63 = low 32 of 40-bit val
        let high = ((instr >> 24) & 0xFF) as u64; // rs2 = bits 24-31 = high 8 of 40-bit val
        let val = (high << 32) | low;
        let sign = (val >> 39) & 1;
        vm.regs.get_unchecked_mut(r).i = if sign == 1 {
            (val | 0xFFFF_FF00_0000_0000) as i64
        } else {
            val as i64
        };
    }

    // rd = i64_consts[idx]  — load i64 from constant pool by index (immediate)
    #[inline(always)]
    pub unsafe fn vm_ldi64_c(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let idx = immu(instr) as usize;
        debug_assert!(idx < vm.i64_const_count as usize);
        vm.regs.get_unchecked_mut(r).i = *vm.i64_consts_ptr.add(idx);
    }

    // rd = consts[idx]  — load f64 from constant pool by index (immediate)
    #[inline(always)]
    pub unsafe fn vm_ldcf64(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let idx = immu(instr) as usize;
        debug_assert!(idx < vm.const_count as usize);
        vm.regs.get_unchecked_mut(r).f = *vm.consts_ptr.add(idx);
    }

    // rd = idx  — load string constant index into register (as i64)
    #[inline(always)]
    pub unsafe fn vm_ldcstr(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let idx = immu(instr) as usize;
        debug_assert!(idx < vm.cold.const_strings.len());
        vm.regs.get_unchecked_mut(r).i = idx as i64;
    }

    // ── Type conversion ──

    // rd = (f64)rs1  — convert integer register to float
    #[inline(always)]
    pub unsafe fn vm_i2f(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let val = vm.regs.get_unchecked(rs1(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).f = val as f64;
    }

    // rd = (i64)rs1  — convert float register to integer (truncate)
    #[inline(always)]
    pub unsafe fn vm_f2i(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let val = vm.regs.get_unchecked(rs1(instr) as usize).f;
        vm.regs.get_unchecked_mut(r).i = val as i64;
    }

    // ── Engine builtins ──

    // rd = indicator value at string-constant index rs1 (0.0 if not found)
    #[inline(always)]
    pub unsafe fn vm_getind(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let str_idx = vm.regs.get_unchecked(rs1(instr) as usize).i as usize;
        debug_assert!(str_idx < vm.cold.indicator_by_str.len());
        let slot = *vm.cold.indicator_by_str.get_unchecked(str_idx);
        let val = if slot != u16::MAX {
            *vm.cold.indicators.get_unchecked(slot as usize)
        } else {
            0.0
        };
        vm.regs.get_unchecked_mut(r).f = val;
    }

    // rd = last_price (most recent trade price set by engine)
    #[inline(always)]
    pub unsafe fn vm_getprice(vm: &mut Vm, instr: u64) {
        let _ = instr;
        let r = rd(instr) as usize;
        vm.regs.get_unchecked_mut(r).f = vm.last_price;
    }

    // rd = position_size (current position set by engine)
    #[inline(always)]
    pub unsafe fn vm_getpos(vm: &mut Vm, instr: u64) {
        let _ = instr;
        let r = rd(instr) as usize;
        vm.regs.get_unchecked_mut(r).f = vm.position_size;
    }

    // rd = balance value at string-constant index rs1 (0.0 if not found)
    #[inline(always)]
    pub unsafe fn vm_getbal(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let str_idx = vm.regs.get_unchecked(rs1(instr) as usize).i as usize;
        debug_assert!(str_idx < vm.cold.balance_by_str.len());
        let slot = *vm.cold.balance_by_str.get_unchecked(str_idx);
        let val = if slot != u16::MAX {
            *vm.cold.balances.get_unchecked(slot as usize)
        } else {
            0.0
        };
        vm.regs.get_unchecked_mut(r).f = val;
    }

    // rd = bid quantity at depth level rs1 (0.0 if beyond book)
    #[inline(always)]
    pub unsafe fn vm_getdepthbid(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let level = vm.regs.get_unchecked(rs1(instr) as usize).i as usize;
        debug_assert!(level < vm.cold.depth_bids_len as usize);
        let val = *vm.cold.depth_bids_qty.get_unchecked(level);
        vm.regs.get_unchecked_mut(r).f = val;
    }

    // rd = ask quantity at depth level rs1 (0.0 if beyond book)
    #[inline(always)]
    pub unsafe fn vm_getdepthask(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let level = vm.regs.get_unchecked(rs1(instr) as usize).i as usize;
        debug_assert!(level < vm.cold.depth_asks_len as usize);
        let val = *vm.cold.depth_asks_qty.get_unchecked(level);
        vm.regs.get_unchecked_mut(r).f = val;
    }

    // --- SendOrder logic ---
    // Reads order parameters from fixed registers (REG_SEND_SIDE/QTY/PRICE/TYPE/REDUCE),
    // sets has_pending_order = true, and logs the order details.
    // Actual order submission is handled by the engine (outside the VM).

    #[inline(always)]
    pub unsafe fn vm_sendorder(vm: &mut Vm, instr: u64) {
        let _ = instr;
        debug_assert!((REG_SEND_SIDE as usize) < NUM_REGS);
        debug_assert!((REG_SEND_QTY as usize) < NUM_REGS);
        debug_assert!((REG_SEND_PRICE as usize) < NUM_REGS);
        debug_assert!((REG_SEND_TYPE as usize) < NUM_REGS);
        debug_assert!((REG_SEND_REDUCE as usize) < NUM_REGS);
        vm.has_pending_order = true;
        if tracing::enabled!(tracing::Level::INFO) {
            tracing::info!(
                "QFL: SEND_ORDER side={} qty={} price={} type={} reduce={}",
                vm.regs.get_unchecked(REG_SEND_SIDE as usize).i,
                vm.regs.get_unchecked(REG_SEND_QTY as usize).f,
                vm.regs.get_unchecked(REG_SEND_PRICE as usize).f,
                vm.regs.get_unchecked(REG_SEND_TYPE as usize).i,
                vm.regs.get_unchecked(REG_SEND_REDUCE as usize).i,
            );
        }
    }

    // ── Persist operations ──
    // Hot-reload-safe state: tagged slots that survive across code reloads.

    // rd = persist[imm]  — read persist slot; if tag=0 read int, else read float
    #[inline(always)]
    pub unsafe fn vm_persistget(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let slot = immu(instr) as usize;
        debug_assert!(slot < PERSIST_SLOTS);
        let ps = vm.cold.persist.get_unchecked(slot);
        if ps.tag == 0 {
            vm.regs.get_unchecked_mut(r).i = ps.int_val;
        } else {
            vm.regs.get_unchecked_mut(r).f = ps.float_val;
        }
    }

    // persist[imm] = rd  — write persist slot; auto-tag based on register number
    // (rd >= INT_REG_COUNT → float, else int)
    #[inline(always)]
    pub unsafe fn vm_persistset(vm: &mut Vm, instr: u64) {
        let slot = immu(instr) as usize;
        let r = rd(instr) as usize;
        debug_assert!(slot < PERSIST_SLOTS);
        let ps = vm.cold.persist.get_unchecked_mut(slot);
        if r >= INT_REG_COUNT as usize {
            ps.tag = 1;
            ps.float_val = vm.regs.get_unchecked(r).f;
        } else {
            ps.tag = 0;
            ps.int_val = vm.regs.get_unchecked(r).i;
        }
    }

    // ── Logging ──

    // Log message from string constant at rs1 (no value)
    #[inline(always)]
    pub unsafe fn vm_log(vm: &mut Vm, instr: u64) {
        let str_idx = vm.regs.get_unchecked(rs1(instr) as usize).i as usize;
        let msg = if str_idx < vm.cold.const_strings.len() {
            vm.cold.const_strings.get_unchecked(str_idx).as_str()
        } else {
            ""
        };
        tracing::info!("QFL: {}", msg);
    }

    // Log message from string constant at rs1 with f64 value from rs2
    #[inline(always)]
    pub unsafe fn vm_log2(vm: &mut Vm, instr: u64) {
        let str_idx = vm.regs.get_unchecked(rs1(instr) as usize).i as usize;
        let val = vm.regs.get_unchecked(rs2(instr) as usize).f;
        if str_idx < vm.cold.const_strings.len() {
            tracing::info!(
                "QFL: {}: {}",
                vm.cold.const_strings.get_unchecked(str_idx),
                val
            );
        } else {
            tracing::info!("QFL: {}", val);
        }
    }

    // Halts execution by setting running = false.
    #[inline(always)]
    pub unsafe fn vm_halt(vm: &mut Vm, instr: u64) {
        let _ = instr;
        vm.running = false;
    }

    // ── Window opcodes ──
    // Rolling window operations: push a value, query aggregates (mean, stddev, min, max, sum).

    // Push value onto window wid. Maintains ring buffer, running sum/sum_sq,
    // and O(1) min/max via monotonic deques. Returns the pushed value in rd.
    // On first push, auto-initializes the window with capacity=64 at offset=wid*64.

    #[inline(always)]
    pub unsafe fn vm_windowpush(vm: &mut Vm, instr: u64) {
        let wid = immu(instr) as usize;
        let val = vm.regs.get_unchecked(rs1(instr) as usize).f;
        let r = rd(instr) as usize;
        debug_assert!(wid < MAX_WINDOWS);
        let meta = vm.cold.window_meta.get_unchecked_mut(wid);
            // Auto-initialize on first push
            if meta.capacity == 0 {
                meta.capacity = 64;
                meta.offset = (wid as u16) * 64;
                meta.head = 0;
                meta.len = 0;
                meta.sum = 0.0;
                meta.sum_sq = 0.0;
                meta.min = val;
                meta.max = val;
                meta.min_dq_front = 0;
                meta.min_dq_back = 0;
                meta.max_dq_front = 0;
                meta.max_dq_back = 0;
            }
            let cap = meta.capacity as usize;
            let off = meta.offset as usize;
            let head = meta.head as usize;

            // Write value to ring buffer head position
            *vm.cold.window_arena.get_unchecked_mut(off + head) = val;

            // Min deque: pop back while back value > new value (maintains increasing monotonic order)
            {
                let dq = &mut meta.min_deque;
                let mut back = meta.min_dq_back;
                while meta.min_dq_front != back {
                    let prev = back.wrapping_sub(1) & 63;
                    let pos = dq[prev as usize] as usize;
                    if *vm.cold.window_arena.get_unchecked(off + pos) > val {
                        back = prev;
                    } else {
                        break;
                    }
                }
                dq[back as usize] = head as u8;
                meta.min_dq_back = back.wrapping_add(1) & 63;
            }

            // Max deque: pop back while back value < new value (maintains decreasing monotonic order)
            {
                let dq = &mut meta.max_deque;
                let mut back = meta.max_dq_back;
                while meta.max_dq_front != back {
                    let prev = back.wrapping_sub(1) & 63;
                    let pos = dq[prev as usize] as usize;
                    if *vm.cold.window_arena.get_unchecked(off + pos) < val {
                        back = prev;
                    } else {
                        break;
                    }
                }
                dq[back as usize] = head as u8;
                meta.max_dq_back = back.wrapping_add(1) & 63;
            }

            if (meta.len as usize) < cap {
                // Window not yet full: just advance head
                meta.head = ((head + 1) % cap) as u16;
                meta.len += 1;
                meta.sum += val;
                meta.sum_sq += val * val;
            } else {
                // Window full: overwrite oldest value, update running aggregates
                let old = *vm.cold.window_arena.get_unchecked(off + head);
                meta.head = ((head + 1) % cap) as u16;
                meta.sum = meta.sum - old + val;
                meta.sum_sq = meta.sum_sq - old * old + val * val;

                // Evict from deque fronts if the evicted position matches
                if meta.min_dq_front != meta.min_dq_back
                    && meta.min_deque[meta.min_dq_front as usize] == head as u8
                {
                    meta.min_dq_front = meta.min_dq_front.wrapping_add(1) & 63;
                }
                if meta.max_dq_front != meta.max_dq_back
                    && meta.max_deque[meta.max_dq_front as usize] == head as u8
                {
                    meta.max_dq_front = meta.max_dq_front.wrapping_add(1) & 63;
                }
            }

            // Read min/max from deque fronts (O(1))
            if meta.min_dq_front != meta.min_dq_back {
                let pos = meta.min_deque[meta.min_dq_front as usize] as usize;
                meta.min = *vm.cold.window_arena.get_unchecked(off + pos);
            } else {
                meta.min = val;
            }
            if meta.max_dq_front != meta.max_dq_back {
                let pos = meta.max_deque[meta.max_dq_front as usize] as usize;
                meta.max = *vm.cold.window_arena.get_unchecked(off + pos);
            } else {
                meta.max = val;
            }

            vm.regs.get_unchecked_mut(r).f = val;
    }

    // Macro generating window unary query handlers (mean, stddev, min, max, sum).
    // Reads the result from WindowMeta into rd. Does nothing if window empty.
    macro_rules! window_unary {
        ($name:ident, $method:ident) => {
            #[inline(always)]
            pub unsafe fn $name(vm: &mut Vm, instr: u64) {
                let wid = immu(instr) as usize;
                let r = rd(instr) as usize;
                debug_assert!(wid < MAX_WINDOWS);
                let meta = vm.cold.window_meta.get_unchecked(wid);
                let result = meta.$method();
                if meta.len > 0 {
                    vm.regs.get_unchecked_mut(r).f = result;
                }
            }
        };
    }

    window_unary!(vm_windowmean, mean);     // rd = mean of window wid
    window_unary!(vm_windowstddev, stddev); // rd = stddev of window wid
    window_unary!(vm_windowmin, min);       // rd = min of window wid
    window_unary!(vm_windowmax, max);       // rd = max of window wid
    window_unary!(vm_windowsum, sum);       // rd = sum of window wid

    // ── Phase 4g: fused feature opcodes ──

    // EMA update: rd = ema_state[sid].update(rs1)
    // On first call (initialized=false), seeds value = input.
    // On subsequent calls: value = alpha * input + (1-alpha) * prev_value.

    #[inline(always)]
    pub unsafe fn vm_ema(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let val = vm.regs.get_unchecked(rs1(instr) as usize).f;
        let sid = rs2(instr) as usize;
        debug_assert!(sid < MAX_EMA_STATES);
        let st = vm.cold.ema_states.get_unchecked_mut(sid);
            if st.initialized {
                st.value = st.alpha * val + (1.0 - st.alpha) * st.value;
            } else {
                st.value = val;
                st.initialized = true;
            }
            vm.regs.get_unchecked_mut(r).f = sanitize_f(st.value);
    }

    // ── Power operations ──

    // rd = rs1 ^ rs2  (integer exponentiation via exponentiation by squaring)
    // Returns 0 for negative exp, 1 for exp == 0.
    #[inline(always)]
    pub unsafe fn vm_pow(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        debug_assert!((rs2(instr) as usize) < NUM_REGS);
        let base = vm.regs.get_unchecked(rs1(instr) as usize).i;
        let exp = vm.regs.get_unchecked(rs2(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = if exp <= 0 {
            if exp == 0 {
                1
            } else {
                0
            }
        } else {
            let mut result = 1i64;
            let mut b = base;
            let mut e = exp;
            while e > 0 {
                if e & 1 == 1 {
                    result = result.wrapping_mul(b);
                }
                b = b.wrapping_mul(b);
                e >>= 1;
            }
            result
        };
    }

    // rd = rs1 ^ rs2  (float exponentiation via powf)
    #[inline(always)]
    pub unsafe fn vm_fpow(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        debug_assert!((rs2(instr) as usize) < NUM_REGS);
        let a = vm.regs.get_unchecked(rs1(instr) as usize).f;
        let b = vm.regs.get_unchecked(rs2(instr) as usize).f;
        vm.regs.get_unchecked_mut(r).f = sanitize_f(a.powf(b));
    }
}
