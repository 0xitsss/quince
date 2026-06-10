// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! QFL bytecode VM — register-based interpreter with direct threaded dispatch.
//!
//! # Architecture
//!
//! The VM executes compiled [`QfrProgram`]s via a 256-entry function pointer table
//! ([`DISPATCH_TABLE`]). Each instruction is a packed `u64`, decoded by bit-field
//! extractors (`rd`, `rs1`, `rs2`, `imm`).
//!
//! ## Hot / Cold split
//!
//! The hot path (registers, PC, call stack, raw code pointer) lives in [`Vm`] (~2 KB,
//! fits in L1). Cold data (indicators, balances, depth book, windows, persist) lives
//! behind `Box<ColdVm>` (~30+ KB, L2/L3). This keeps the dispatch loop cache-friendly.
//!
//! ## Register file
//!
//! 256 slots: regs `0..=191` are conventionally integer (`i64`), `192..=255` float
//! (`f64`). Stored as a `union Register` (`#[repr(C)]`) for zero-overhead access.
//!
//! ## Dispatch
//!
//! The single entry point is [`Vm::call`] which looks up an entry offset by name,
//! sets `vm.pc`, and calls [`Vm::run`]. `run` fetches the first instruction and
//! dispatches via [`DISPATCH_TABLE`]. Each handler finishes with
//! `become dispatch_next(vm, instr)` — a guaranteed tail-call that advances `pc`,
//! fetches, and dispatches the next instruction. Control-flow handlers (`vm_jmp`,
//! `vm_call`, etc.) set `pc` directly before tail-calling. `vm_halt` returns
//! normally, unwinding the flat dispatch stack back to `run`.
//!
//! ## Safety
//!
//! Handlers use unchecked register access (`get_unchecked`) and raw pointer arithmetic
//! on `code_ptr`. Preconditions are documented per-handler via `# Safety` sections.
//! The VM is not thread-safe; each [`Vm`] is pinned to one thread.

use crate::ir::{ConstEntry, QfrProgram};
use crate::opcodes::Instruction;
use std::arch::x86_64::_mm_prefetch;
use std::collections::HashMap;
use std::intrinsics::{likely, unlikely};
use std::io::Write;

// --- Architectural constants ---

pub const NUM_REGS: usize = 256; // Total register file slots (2048 bytes)
pub const INT_REG_COUNT: u8 = 192; // Regs 0-191 are integer-typed; 192-255 are float-typed
pub const PERSIST_SLOTS: usize = 64; // Number of persist slots for hot-reload-safe state
pub const MAX_CALL_DEPTH: usize = 64; // Max nested call/return depth
pub const MAX_INDICATORS: usize = 1024; // Max number of named indicator slots
pub const MAX_BALANCES: usize = 128; // Max number of named balance slots
pub const MAX_WINDOWS: usize = 64; // Max number of rolling windows
pub const WINDOW_ARENA_SIZE: usize = 65536; // Total ring-buffer elements across all windows
pub const MAX_DEPTH_LEVELS: usize = 64; // Max order-book depth levels per side
pub const MAX_EMA_STATES: usize = 256; // Max number of EMA state slots

// --- SendOrder register convention ---
// When issuing a SEND_ORDER instruction, the VM reads these fixed registers:
pub const REG_SEND_SIDE: u8 = 250; // Order side (i64: 0=buy, 1=sell)
pub const REG_SEND_QTY: u8 = 192; // Order quantity (f64)
pub const REG_SEND_PRICE: u8 = 193; // Order price (f64)
pub const REG_SEND_TYPE: u8 = 253; // Order type (i64: e.g. market/limit)
pub const REG_SEND_REDUCE: u8 = 254; // Reduce-only flag (i64)

// --- Register File ---
// A 256-slot register file. Each slot stores either an i64 or f64 (union).
// Regs 0-191 are conventionally integer; regs 192-255 are conventionally float.

/// A single register slot — stores either an `i64` or `f64` via `union`.
///
/// Regs `0..=191` are conventionally integer, `192..=255` float.
/// Access via [`Self::from_i64`], [`Self::from_f64`], or directly through the union
/// fields (`reg.i`, `reg.f`).
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
    /// Wrap an `i64` into a register slot.
    #[inline(always)]
    pub fn from_i64(val: i64) -> Self {
        Register { i: val }
    }
    /// Wrap an `f64` into a register slot.
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

/// A single persist slot — survives across hot-reload cycles.
///
/// `tag` determines which field carries the value:
/// - `0` в†’ [`int_val`](Self::int_val)
/// - `1` в†’ [`float_val`](Self::float_val)
#[derive(Debug, Clone, Copy)]
pub struct PersistSlot {
    /// Tag: `0` = integer, `1` = float.
    pub tag: u8,
    /// Integer value (valid when [`tag`](Self::tag) == 0).
    pub int_val: i64,
    /// Float value (valid when [`tag`](Self::tag) == 1).
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

/// EMA (Exponential Moving Average) state for one slot.
///
/// Used by the `vm_ema` opcode. On first push (`initialized == false`) the value
/// is seeded directly; thereafter it updates as `value = alpha * input + (1 - alpha) * value`.
#[derive(Debug, Clone, Copy)]
pub struct EmaState {
    /// Smoothing factor (0..1), set at compile time from `program.ema_alphas`.
    pub alpha: f64,
    /// Current EMA value.
    pub value: f64,
    /// `false` before first push — first value seeds `value` directly.
    pub initialized: bool,
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
    pub offset: u16,   // Start index into window_arena for this window
    pub capacity: u16, // Ring buffer capacity (typically 64)
    pub head: u16,     // Current write position (ring index)
    pub len: u16,      // Number of elements currently in window
    pub sum: f64,      // Running sum (for O(1) mean)
    pub sum_sq: f64,   // Running sum of squares (for O(1) variance/stddev)
    pub min: f64,      // Current minimum value in window
    pub max: f64,      // Current maximum value in window
    // O(1) sliding min/max via monotonic deque (indices into ring buffer)
    pub min_deque: [u8; 64], // Fixed-size deque tracking indices of increasing minima
    pub max_deque: [u8; 64], // Fixed-size deque tracking indices of decreasing maxima
    pub min_dq_front: u8,    // Front pointer for min deque (circular buffer in [0,64))
    pub min_dq_back: u8,     // Back pointer for min deque
    pub max_dq_front: u8,    // Front pointer for max deque
    pub max_dq_back: u8,     // Back pointer for max deque
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

/// Cold (L2/L3) VM data — behind a `Box` to keep [`Vm`] cache-friendly.
///
/// Contains large arrays (~30+ KB) pushed out of L1: indicators, balances,
/// depth book, persist slots, window arena, EMA states, and profiling/tracing
/// infrastructure. Accessed through [`Vm::cold`].
#[derive(Debug)]
#[repr(C)]
pub struct ColdVm {
    // Large flat arrays (pushed out of L1, behind Box)
    pub indicators: [f64; MAX_INDICATORS], // Named indicator values by slot
    pub indicator_by_str: Vec<u16>,        // String constant index в†’ indicator slot
    pub balances: [f64; MAX_BALANCES],     // Named balance values by slot
    pub balance_by_str: Vec<u16>,          // String constant index в†’ balance slot

    // Depth book (order book snapshots)
    pub depth_bids_price: [f64; MAX_DEPTH_LEVELS], // Bid prices per level
    pub depth_bids_qty: [f64; MAX_DEPTH_LEVELS],   // Bid quantities per level
    pub depth_asks_price: [f64; MAX_DEPTH_LEVELS], // Ask prices per level
    pub depth_asks_qty: [f64; MAX_DEPTH_LEVELS],   // Ask quantities per level
    pub depth_bids_len: u8,                        // Number of valid bid levels
    pub depth_asks_len: u8,                        // Number of valid ask levels

    // Persist (hot-reload safe state, snapshot/restore)
    pub persist: [PersistSlot; PERSIST_SLOTS],

    // Rolling windows (ring-buffer arena + per-window metadata)
    pub window_arena: Vec<f64>, // Flat ring-buffer storage for all windows
    pub window_meta: [WindowMeta; MAX_WINDOWS], // Metadata for each window

    // Fused feature states
    pub ema_states: [EmaState; MAX_EMA_STATES], // EMA state slots

    // Ownership holders — backing allocations for raw pointers held in hot Vm.
    // These Vecs must NOT be modified after Vm construction (raw pointers into them).
    pub(crate) _code_owned: Vec<u64>,
    pub(crate) _consts_owned: Vec<f64>,
    pub(crate) _i64_consts_owned: Vec<i64>,

    // Constants pool (backward compat — string constants used by log, getind, getbal)
    pub const_pool: Vec<ConstEntry>,
    pub const_strings: Vec<String>,

    // Nameв†’slot mapping (hash-based, O(1) lookup instead of O(n) linear scan)
    pub indicator_map: HashMap<String, u16>,
    pub balance_map: HashMap<String, u16>,

    // Profiler / Tracer (optional instrumentation)
    pub profiler: Option<crate::profiler::Profiler>,
    pub tracer: Option<crate::tracer::Tracer>,

    // VM Trace (full instruction-level trace to file)
    pub trace_vm_enabled: bool,
    pub trace_file: Option<std::io::BufWriter<std::fs::File>>,
    pub trace_start: std::time::Instant,

    // Debug-only ring buffer for strategy log messages
    #[cfg(debug_assertions)]
    pub log_buffer: Option<crate::log_buffer::LogBuffer>,
}

/// Hot VM — the primary interpreter struct, sized to fit in L1 cache (~2 KB).
///
/// Registers, PC, call stack, and raw code pointers live here. All cold
/// (large-array) state lives in [`ColdVm`] behind a `Box`.
///
/// # Thread safety
///
/// `Vm` is **not** `Send` or `Sync`. Each instance must remain on one thread.
#[derive(Debug)]
#[repr(C)]
pub struct Vm {
    // в•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђ
    //  HOT PATH — accessed by almost every instruction in run()
    // в•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђ
    // Register file (2048 bytes, fits in L1)
    pub regs: [Register; NUM_REGS],

    // Execution state
    pub pc: usize,                           // Program counter (index into code_ptr)
    pub running: bool,                       // True while dispatch loop should continue
    pub call_stack: [usize; MAX_CALL_DEPTH], // Return addresses for CALL/RET
    pub call_depth: u8,                      // Current call depth

    // Code + constants — raw pointers into owned backing in ColdVm
    // Used in lieu of Vec indexing to avoid bounds checks in hot path.
    pub code_ptr: *const u64, // Pointer to program bytecode (u64 instructions)
    pub code_len: usize,      // Number of instructions
    pub consts_ptr: *const f64, // Pointer to f64 constant pool
    pub const_count: u32,     // Number of f64 constants
    pub i64_consts_ptr: *const i64, // Pointer to i64 constant pool
    pub i64_const_count: u32, // Number of i64 constants

    // Scalar engine state (hot — updated by external engine each tick)
    pub last_price: f64,         // Most recent price
    pub position_size: f64,      // Current position size
    pub has_pending_order: bool, // Set true by SEND_ORDER; cleared by engine

    // Entry points (warm, but small — ~72 bytes)
    pub entry_names: [u64; 8], // Entry point names (packed as u64, up to 8 chars)
    pub entry_offsets: [u32; 8], // Corresponding code offsets
    pub entry_count: u8,       // Number of registered entry points
    handler_cache: [u32; 4],   // Cached offsets for 4 standard handlers (no linear scan)

    // Cold data (behind pointer, ~30+ KB out of hot cache line)
    pub cold: Box<ColdVm>,
}

impl Vm {
    // --- Constructor ---

    /// Build a VM from a compiled [`QfrProgram`].
    ///
    /// Takes ownership of the program's code and constant pools, sets up raw
    /// pointers for zero-cost dispatch, and initialises all runtime state.
    pub fn new(program: QfrProgram) -> Self {
        let (_code_owned, code_ptr, code_len) = Self::take_code(&program);
        let (
            _consts_owned,
            _i64_consts_owned,
            const_strings,
            consts_ptr,
            const_count,
            i64_consts_ptr,
            i64_const_count,
        ) = Self::take_consts(&program);
        let (entry_names, entry_offsets, entry_count) = Self::pack_entries(&program);
        let indicator_by_str = vec![u16::MAX; const_strings.len()];
        let balance_by_str = vec![u16::MAX; const_strings.len()];
        let handler_cache = Self::build_handler_cache(&entry_names, &entry_offsets, entry_count);
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
                #[cfg(debug_assertions)]
                log_buffer: Some(crate::log_buffer::LogBuffer::new(256)),
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

    // --- Constructor helpers ---

    /// Take ownership of the program's bytecode and return `(owned_vec, ptr, len)`.
    fn take_code(program: &QfrProgram) -> (Vec<u64>, *const u64, usize) {
        let owned: Vec<u64> = program.code.iter().map(|i| i.raw()).collect();
        let ptr = owned.as_ptr();
        let len = owned.len();
        (owned, ptr, len)
    }

    /// Clone constant pools and return raw pointers for zero-cost access.
    #[allow(clippy::too_many_arguments)]
    fn take_consts(
        program: &QfrProgram,
    ) -> (
        Vec<f64>,
        Vec<i64>,
        Vec<String>,
        *const f64,
        u32,
        *const i64,
        u32,
    ) {
        let f64_owned = program.f64_consts.clone();
        let i64_owned = program.i64_consts.clone();
        let strings = program.string_consts.clone();
        let f64_ptr = f64_owned.as_ptr();
        let f64_cnt = f64_owned.len() as u32;
        let i64_ptr = i64_owned.as_ptr();
        let i64_cnt = i64_owned.len() as u32;
        (
            f64_owned, i64_owned, strings, f64_ptr, f64_cnt, i64_ptr, i64_cnt,
        )
    }

    /// Pack entry-point names into 8-byte u64s for O(1) comparison in the hot path.
    fn pack_entries(program: &QfrProgram) -> ([u64; 8], [u32; 8], u8) {
        let mut names = [0u64; 8];
        let mut offsets = [0u32; 8];
        let count = program.entries.len().min(8) as u8;
        for (i, e) in program.entries.iter().enumerate().take(count as usize) {
            let mut name_bytes = [0u8; 8];
            let src = e.name.as_bytes();
            let n = src.len().min(8);
            name_bytes[..n].copy_from_slice(&src[..n]);
            names[i] = u64::from_le_bytes(name_bytes);
            offsets[i] = e.code_offset;
        }
        (names, offsets, count)
    }

    /// Build the handler-offset cache for the four standard entry points
    /// (`on_trade`, `on_eval`, `on_fill`, `on_depth`) so the hot path avoids
    /// a linear scan.
    fn build_handler_cache(
        entry_names: &[u64; 8],
        entry_offsets: &[u32; 8],
        entry_count: u8,
    ) -> [u32; 4] {
        const LABELS: [&str; 4] = ["on_trade", "on_eval", "on_fill", "on_depth"];
        let mut cache = [u32::MAX; 4];
        for (i, &label) in LABELS.iter().enumerate() {
            let label_bytes = label.as_bytes();
            let label_len = label_bytes.len();
            for j in 0..entry_count as usize {
                let stored = entry_names[j].to_le_bytes();
                if stored[..label_len] == label_bytes[..label_len] {
                    cache[i] = entry_offsets[j];
                    break;
                }
            }
        }
        cache
    }

    // --- Register Access Helpers ---

    /// Set reg `reg` to `val` (integer). Uses unchecked indexing for speed.
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
        if let Some(ref mut p) = self.cold.profiler {
            p.start_handler(entry_name);
        }
        self.pc = offset;
        self.running = true;
        self.call_depth = 0;
        #[cfg(feature = "profiling")]
        match entry_name {
            "on_trade" => {
                puffin::profile_scope!("vm::call(on_trade)");
                self.run();
            }
            "on_eval" => {
                puffin::profile_scope!("vm::call(on_eval)");
                self.run();
            }
            "on_fill" => {
                puffin::profile_scope!("vm::call(on_fill)");
                self.run();
            }
            "on_depth" => {
                puffin::profile_scope!("vm::call(on_depth)");
                self.run();
            }
            _ => {
                puffin::profile_scope!("vm::call(?)");
                self.run();
            }
        }
        #[cfg(not(feature = "profiling"))]
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

    // в”Ђв”Ђ OS Core Isolation + Memory Pinning в”Ђв”Ђ

    /// Pin the current thread to a specific CPU core.
    #[cfg(target_os = "windows")]
    pub fn pin_to_core(core_id: usize) -> Result<(), String> {
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentThread() -> *mut std::ffi::c_void;
            fn SetThreadAffinityMask(hThread: *mut std::ffi::c_void, mask: usize) -> usize;
        }
        unsafe {
            let mask: usize = 1 << core_id;
            let ret = SetThreadAffinityMask(GetCurrentThread(), mask);
            if ret == 0 {
                Err(format!("SetThreadAffinityMask failed for core {}", core_id))
            } else {
                Ok(())
            }
        }
    }

    #[cfg(target_os = "linux")]
    pub fn pin_to_core(core_id: usize) -> Result<(), String> {
        extern "C" {
            fn sched_setaffinity(pid: i32, cpusetsize: usize, mask: *mut u64) -> i32;
        }
        unsafe {
            let mut mask: u64 = 1 << core_id;
            let ret = sched_setaffinity(0, std::mem::size_of::<u64>(), &mut mask);
            if ret != 0 {
                Err(format!("sched_setaffinity failed for core {}", core_id))
            } else {
                Ok(())
            }
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    pub fn pin_to_core(_core_id: usize) -> Result<(), String> {
        Err("core pinning not supported on this platform".to_string())
    }

    /// Lock the VM's hot memory pages into RAM to prevent swapping.
    #[cfg(target_os = "windows")]
    pub fn lock_hot_memory(&mut self) -> Result<(), String> {
        #[link(name = "kernel32")]
        extern "system" {
            fn VirtualLock(lpAddress: *mut std::ffi::c_void, dwSize: usize) -> i32;
        }
        unsafe {
            let regs_ptr = self.regs.as_ptr() as *mut std::ffi::c_void;
            if VirtualLock(regs_ptr, std::mem::size_of_val(&self.regs)) == 0 {
                return Err("VirtualLock(regs) failed".to_string());
            }
            let code_ptr = self.code_ptr as *mut std::ffi::c_void;
            if VirtualLock(code_ptr, self.code_len * std::mem::size_of::<u64>()) == 0 {
                return Err("VirtualLock(code) failed".to_string());
            }
            Ok(())
        }
    }

    #[cfg(target_os = "linux")]
    pub fn lock_hot_memory(&mut self) -> Result<(), String> {
        extern "C" {
            fn mlock(addr: *const std::ffi::c_void, len: usize) -> i32;
        }
        unsafe {
            let regs_ptr = self.regs.as_ptr() as *const std::ffi::c_void;
            if mlock(regs_ptr, std::mem::size_of_val(&self.regs)) != 0 {
                return Err("mlock(regs) failed".to_string());
            }
            let code_ptr = self.code_ptr as *const std::ffi::c_void;
            if mlock(code_ptr, self.code_len * std::mem::size_of::<u64>()) != 0 {
                return Err("mlock(code) failed".to_string());
            }
            Ok(())
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    pub fn lock_hot_memory(&mut self) -> Result<(), String> {
        let _ = self;
        Err("memory locking not supported on this platform".to_string())
    }
}

// --- Direct Threaded Dispatch ---
// Replaces computed-goto jump table with function pointer table + become tail calls.
// Each handler does its work then tail-calls the next handler via `become`.
// DISPATCH_TABLE is a 256-entry const array of function pointers indexed by opcode.

type Handler = unsafe fn(&mut Vm, u64);

const DISPATCH_TABLE: [Handler; 256] = {
    let mut t: [Handler; 256] = [handlers::vm_halt; 256];
    t[0] = handlers::vm_add;
    t[1] = handlers::vm_sub;
    t[2] = handlers::vm_mul;
    t[3] = handlers::vm_div;
    t[4] = handlers::vm_mod;
    t[5] = handlers::vm_neg;
    t[6] = handlers::vm_addi;
    t[7] = handlers::vm_subi;
    t[8] = handlers::vm_muli;
    t[9] = handlers::vm_divi;
    t[10] = handlers::vm_fadd;
    t[11] = handlers::vm_fsub;
    t[12] = handlers::vm_fmul;
    t[13] = handlers::vm_fdiv;
    t[14] = handlers::vm_fneg;
    t[15] = handlers::vm_eq;
    t[16] = handlers::vm_ne;
    t[17] = handlers::vm_lt;
    t[18] = handlers::vm_gt;
    t[19] = handlers::vm_le;
    t[20] = handlers::vm_ge;
    t[21] = handlers::vm_feq;
    t[22] = handlers::vm_fne;
    t[23] = handlers::vm_flt;
    t[24] = handlers::vm_fgt;
    t[25] = handlers::vm_fle;
    t[26] = handlers::vm_fge;
    t[27] = handlers::vm_eqi;
    t[28] = handlers::vm_lti;
    t[29] = handlers::vm_gti;
    t[30] = handlers::vm_bitand;
    t[31] = handlers::vm_bitor;
    t[32] = handlers::vm_bitxor;
    t[33] = handlers::vm_bitnot;
    t[34] = handlers::vm_shl;
    t[35] = handlers::vm_shr;
    t[36] = handlers::vm_jmp;
    t[37] = handlers::vm_jz;
    t[38] = handlers::vm_jnz;
    t[39] = handlers::vm_call;
    t[40] = handlers::vm_ret;
    t[41] = handlers::vm_mov;
    t[42] = handlers::vm_ldi;
    t[43] = handlers::vm_ldi64;
    t[44] = handlers::vm_ldcf64;
    t[45] = handlers::vm_i2f;
    t[46] = handlers::vm_f2i;
    t[47] = handlers::vm_getind;
    t[48] = handlers::vm_getprice;
    t[49] = handlers::vm_getpos;
    t[50] = handlers::vm_getbal;
    t[51] = handlers::vm_getdepthbid;
    t[52] = handlers::vm_getdepthask;
    t[53] = handlers::vm_sendorder;
    t[54] = handlers::vm_persistget;
    t[55] = handlers::vm_persistset;
    t[56] = handlers::vm_log;
    t[57] = handlers::vm_halt;
    t[58] = handlers::vm_windowpush;
    t[59] = handlers::vm_windowmean;
    t[60] = handlers::vm_windowstddev;
    t[61] = handlers::vm_windowmin;
    t[62] = handlers::vm_windowmax;
    t[63] = handlers::vm_windowsum;
    t[64] = handlers::vm_ema;
    t[65] = handlers::vm_log2;
    t[66] = handlers::vm_ldi64_c;
    t[67] = handlers::vm_ldcstr;
    t[68] = handlers::vm_pow;
    t[69] = handlers::vm_fpow;
    t
};

/// Tail-call helper: advance PC, fetch next instruction, dispatch.
/// Every normal handler ends with `become dispatch_next(vm, instr)`.
/// Control-flow handlers (jmp, jz, jnz, call, ret) set `vm.pc` directly,
/// then fetch + `become DISPATCH_TABLE[...]` inline.
///
/// # Safety
/// - `vm.code_ptr` must point to valid bytecode with at least `vm.pc + 1` instructions.
/// - Caller must ensure `vm` is in a consistent state before dispatching.
#[inline(never)]
pub unsafe fn dispatch_next(vm: &mut Vm, instr: u64) {
    vm.trace_exec(instr);
    vm.pc = vm.pc.wrapping_add(1);
    let next = *vm.code_ptr.add(vm.pc);
    become DISPATCH_TABLE[(next & 0xFF) as usize](vm, next);
}

impl Vm {
    /// Entry point: dispatch to the first instruction.
    /// After that, each handler tail-calls the next via `become`.
    /// The stack stays flat — `vm_halt` returns directly to this function.
    #[inline(always)]
    fn run(&mut self) {
        unsafe {
            let first = *self.code_ptr.add(self.pc);
            DISPATCH_TABLE[(first & 0xFF) as usize](self, first);
        }
    }

    // Inline trace check — zero cost when trace_vm_enabled is false.
    #[inline(always)]
    fn trace_exec(&mut self, instr: u64) {
        if unlikely(self.cold.trace_vm_enabled) {
            self.trace_vm_instruction((instr & 0xFF) as u8, instr);
        }
    }

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

    /// Enable per-instruction VM tracing to a file.
    ///
    /// Each executed instruction is logged with: timestamp, PC, opcode, register
    /// values (rd, rs1, rs2), and immediate. Useful for debugging miscompilation.
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

    /// Set a named balance. Creates a new slot if the name is not yet registered.
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

    /// Copy ordered bid levels into the depth book.
    pub fn set_depth_bids(&mut self, bids: &[quince_core::types::DepthLevel]) {
        let n = bids.len().min(MAX_DEPTH_LEVELS);
        for (i, bid) in bids.iter().enumerate().take(n) {
            self.cold.depth_bids_price[i] = bid.price;
            self.cold.depth_bids_qty[i] = bid.qty;
        }
        self.cold.depth_bids_len = n as u8;
    }

    // Copies ordered ask levels into the depth book.
    pub fn set_depth_asks(&mut self, asks: &[quince_core::types::DepthLevel]) {
        let n = asks.len().min(MAX_DEPTH_LEVELS);
        for (i, ask) in asks.iter().enumerate().take(n) {
            self.cold.depth_asks_price[i] = ask.price;
            self.cold.depth_asks_qty[i] = ask.qty;
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

    // Rebuilds indicator_by_str / balance_by_str from the current nameв†’slot lists.
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

/// Snapshot of VM state that survives hot-reload.
///
/// Captured by [`Vm::snapshot`] before a hot-reload and restored by
/// [`Vm::restore`] afterwards. Carries registers, persist slots, program
/// counter, indicators, and balances.
#[derive(Debug, Clone)]
pub struct VmSnapshot {
    pub regs: [Register; NUM_REGS],
    pub persist: [PersistSlot; PERSIST_SLOTS],
    pub pc: usize,
    pub indicators: [f64; MAX_INDICATORS],
    pub balances: [f64; MAX_BALANCES],
}

// в•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђ
// Jump Table Handlers
// в•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђв•ђ
// Each opcode handler is a function pointer in JUMP_TABLE.
// Handlers follow the signature: fn(&mut Vm, instr: u64)

// --- Instruction Field Extractors ---
// These unsafe helpers extract bit fields from a packed u64 instruction.
// Layout: [opcode:8][rd:8][rs1:8][rs2:8][imm:32]

#[inline(always)]
unsafe fn rd(instr: u64) -> u8 {
    ((instr >> 8) & 0xFF) as u8 // Destination register
}
#[inline(always)]
unsafe fn rs1(instr: u64) -> u8 {
    ((instr >> 16) & 0xFF) as u8 // First source register
}
#[inline(always)]
unsafe fn rs2(instr: u64) -> u8 {
    ((instr >> 24) & 0xFF) as u8 // Second source register (or extra data)
}
#[inline(always)]
unsafe fn imm(instr: u64) -> i32 {
    (instr >> 32) as i32 // Signed 32-bit immediate
}
#[inline(always)]
unsafe fn immu(instr: u64) -> u32 {
    (instr >> 32) as u32 // Unsigned 32-bit immediate
}

// NaN/Inf sanitizer — branchless via SSE: replace NaN/Inf with 0.0.
#[inline(always)]
fn sanitize_f(val: f64) -> f64 {
    unsafe {
        let a = std::arch::x86_64::_mm_set_sd(val);
        let mask = std::arch::x86_64::_mm_cmpunord_sd(a, a);
        let masked = std::arch::x86_64::_mm_andnot_pd(mask, a);
        std::arch::x86_64::_mm_cvtsd_f64(masked)
    }
}

// --- Opcode Handlers ---

pub mod handlers {
    use super::*;

    // в”Ђв”Ђ Int arithmetic в”Ђв”Ђ
    // All integer ops use wrapping arithmetic (no overflow panic).

    // rd = rs1 + rs2 (wrapping add)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_add(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        debug_assert!((rs2(instr) as usize) < NUM_REGS);
        let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
        let b = vm.regs.get_unchecked(rs2(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = a.wrapping_add(b);
        become dispatch_next(vm, instr);
    }

    // rd = rs1 - rs2 (wrapping sub)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
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
        become dispatch_next(vm, instr);
    }

    // rd = rs1 * rs2 (wrapping mul)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
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
        become dispatch_next(vm, instr);
    }

    // rd = rs1 / rs2 (returns 0 on division by zero)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_div(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        debug_assert!((rs2(instr) as usize) < NUM_REGS);
        let divisor = vm.regs.get_unchecked(rs2(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = if unlikely(divisor == 0) {
            0
        } else {
            vm.regs.get_unchecked(rs1(instr) as usize).i / divisor
        };
        become dispatch_next(vm, instr);
    }

    // rd = rs1 % rs2 (returns 0 on division by zero)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_mod(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        debug_assert!((rs2(instr) as usize) < NUM_REGS);
        let divisor = vm.regs.get_unchecked(rs2(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = if unlikely(divisor == 0) {
            0
        } else {
            vm.regs.get_unchecked(rs1(instr) as usize).i % divisor
        };
        become dispatch_next(vm, instr);
    }

    // rd = -rs1 (wrapping neg)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_neg(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        vm.regs.get_unchecked_mut(r).i =
            vm.regs.get_unchecked(rs1(instr) as usize).i.wrapping_neg();
        become dispatch_next(vm, instr);
    }

    // rd = rs1 + imm (wrapping add immediate)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
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
        become dispatch_next(vm, instr);
    }

    // rd = rs1 - imm (wrapping sub immediate)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
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
        become dispatch_next(vm, instr);
    }

    // rd = rs1 * imm (wrapping mul immediate)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
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
        become dispatch_next(vm, instr);
    }

    // rd = rs1 / imm (returns 0 on division by zero)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_divi(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        let divisor = imm(instr) as i64;
        vm.regs.get_unchecked_mut(r).i = if unlikely(divisor == 0) {
            0
        } else {
            vm.regs.get_unchecked(rs1(instr) as usize).i / divisor
        };
        become dispatch_next(vm, instr);
    }

    // в”Ђв”Ђ Float arithmetic в”Ђв”Ђ

    // Macro that generates a float binary-op handler: a $op b
    macro_rules! float_binop {
        ($name:ident, $op:tt) => {
            /// # Safety
            /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
            #[inline(never)]
pub unsafe fn $name(vm: &mut Vm, instr: u64) {
                let r = rd(instr) as usize;
                debug_assert!(r < NUM_REGS);
                debug_assert!((rs1(instr) as usize) < NUM_REGS);
                debug_assert!((rs2(instr) as usize) < NUM_REGS);
                let a = vm.regs.get_unchecked(rs1(instr) as usize).f;
                let b = vm.regs.get_unchecked(rs2(instr) as usize).f;
                vm.regs.get_unchecked_mut(r).f = sanitize_f(a $op b);
                become dispatch_next(vm, instr);
            }
        };
    }

    float_binop!(vm_fadd, +); // rd = rs1 + rs2 (float)
    float_binop!(vm_fsub, -); // rd = rs1 - rs2 (float)
    float_binop!(vm_fmul, *); // rd = rs1 * rs2 (float)

    // rd = rs1 / rs2 (float, returns 0.0 on division by zero)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_fdiv(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let divisor = vm.regs.get_unchecked(rs2(instr) as usize).f;
        vm.regs.get_unchecked_mut(r).f = if unlikely(divisor == 0.0) {
            0.0
        } else {
            sanitize_f(vm.regs.get_unchecked(rs1(instr) as usize).f / divisor)
        };
        become dispatch_next(vm, instr);
    }

    // rd = -rs1 (float negate)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_fneg(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        vm.regs.get_unchecked_mut(r).f = sanitize_f(-vm.regs.get_unchecked(rs1(instr) as usize).f);
        become dispatch_next(vm, instr);
    }

    // в”Ђв”Ђ Int comparison в”Ђв”Ђ
    // Sets rd = 1 if comparison is true, 0 otherwise.

    macro_rules! int_cmp {
        ($name:ident, $cmp:tt) => {
            /// # Safety
            /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
            #[inline(never)]
pub unsafe fn $name(vm: &mut Vm, instr: u64) {
                let r = rd(instr) as usize;
                debug_assert!(r < NUM_REGS);
                debug_assert!((rs1(instr) as usize) < NUM_REGS);
                debug_assert!((rs2(instr) as usize) < NUM_REGS);
                let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
                let b = vm.regs.get_unchecked(rs2(instr) as usize).i;
                vm.regs.get_unchecked_mut(r).i = if a $cmp b { 1 } else { 0 };
                become dispatch_next(vm, instr);
            }
        };
    }

    int_cmp!(vm_eq, ==); // rd = (rs1 == rs2) ? 1 : 0
    int_cmp!(vm_ne, !=); // rd = (rs1 != rs2) ? 1 : 0
    int_cmp!(vm_lt, <); // rd = (rs1 <  rs2) ? 1 : 0
    int_cmp!(vm_gt, >); // rd = (rs1 >  rs2) ? 1 : 0
    int_cmp!(vm_le, <=); // rd = (rs1 <= rs2) ? 1 : 0
    int_cmp!(vm_ge, >=); // rd = (rs1 >= rs2) ? 1 : 0

    // в”Ђв”Ђ Float comparison в”Ђв”Ђ
    // Sets rd = 1 if comparison is true, 0 otherwise. Result is i64.

    macro_rules! float_cmp {
        ($name:ident, $cmp:tt) => {
            /// # Safety
            /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
            #[inline(never)]
pub unsafe fn $name(vm: &mut Vm, instr: u64) {
                let r = rd(instr) as usize;
                debug_assert!(r < NUM_REGS);
                debug_assert!((rs1(instr) as usize) < NUM_REGS);
                debug_assert!((rs2(instr) as usize) < NUM_REGS);
                let a = vm.regs.get_unchecked(rs1(instr) as usize).f;
                let b = vm.regs.get_unchecked(rs2(instr) as usize).f;
                vm.regs.get_unchecked_mut(r).i = if a $cmp b { 1 } else { 0 };
                become dispatch_next(vm, instr);
            }
        };
    }

    float_cmp!(vm_feq, ==); // rd = (rs1 == rs2) ? 1 : 0 (float)
    float_cmp!(vm_fne, !=); // rd = (rs1 != rs2) ? 1 : 0 (float)
    float_cmp!(vm_flt, <); // rd = (rs1 <  rs2) ? 1 : 0 (float)
    float_cmp!(vm_fgt, >); // rd = (rs1 >  rs2) ? 1 : 0 (float)
    float_cmp!(vm_fle, <=); // rd = (rs1 <= rs2) ? 1 : 0 (float)
    float_cmp!(vm_fge, >=); // rd = (rs1 >= rs2) ? 1 : 0 (float)

    // в”Ђв”Ђ Immediate comparison в”Ђв”Ђ

    // rd = (rs1 == imm) ? 1 : 0
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_eqi(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = if a == imm(instr) as i64 { 1 } else { 0 };
        become dispatch_next(vm, instr);
    }

    // rd = (rs1 < imm) ? 1 : 0
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_lti(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = if a < imm(instr) as i64 { 1 } else { 0 };
        become dispatch_next(vm, instr);
    }

    // rd = (rs1 > imm) ? 1 : 0
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_gti(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = if a > imm(instr) as i64 { 1 } else { 0 };
        become dispatch_next(vm, instr);
    }

    // в”Ђв”Ђ Bitwise operations в”Ђв”Ђ

    // Macro that generates a bitwise binary-op handler.
    macro_rules! bitwise_binop {
        ($name:ident, $op:tt) => {
            /// # Safety
            /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
            #[inline(never)]
pub unsafe fn $name(vm: &mut Vm, instr: u64) {
                let r = rd(instr) as usize;
                debug_assert!(r < NUM_REGS);
                debug_assert!((rs1(instr) as usize) < NUM_REGS);
                debug_assert!((rs2(instr) as usize) < NUM_REGS);
                let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
                let b = vm.regs.get_unchecked(rs2(instr) as usize).i;
                vm.regs.get_unchecked_mut(r).i = a $op b;
                become dispatch_next(vm, instr);
            }
        };
    }

    bitwise_binop!(vm_bitand, &); // rd = rs1 & rs2
    bitwise_binop!(vm_bitor, |); // rd = rs1 | rs2
    bitwise_binop!(vm_bitxor, ^); // rd = rs1 ^ rs2

    // rd = ~rs1 (bitwise NOT)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_bitnot(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        vm.regs.get_unchecked_mut(r).i = !vm.regs.get_unchecked(rs1(instr) as usize).i;
        become dispatch_next(vm, instr);
    }

    // rd = rs1 << rs2 (wrapping shift left)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_shl(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        debug_assert!((rs2(instr) as usize) < NUM_REGS);
        let a = vm.regs.get_unchecked(rs1(instr) as usize).i;
        let b = vm.regs.get_unchecked(rs2(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = a.wrapping_shl(b as u32);
        become dispatch_next(vm, instr);
    }

    // rd = rs1 >> rs2 (logical shift right, zero-extend)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_shr(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        debug_assert!((rs2(instr) as usize) < NUM_REGS);
        let a = vm.regs.get_unchecked(rs1(instr) as usize).i as u64;
        let b = vm.regs.get_unchecked(rs2(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).i = a.wrapping_shr(b as u32) as i64;
        become dispatch_next(vm, instr);
    }

    // в”Ђв”Ђ Control flow в”Ђв”Ђ

    // Unconditional jump: PC += imm
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_jmp(vm: &mut Vm, instr: u64) {
        let target = (vm.pc as i64)
            .wrapping_add(1)
            .wrapping_add(imm(instr) as i64) as usize;
        if unlikely(target >= vm.code_len) {
            vm.running = false;
            return;
        }
        vm.pc = target;
        let next = *vm.code_ptr.add(vm.pc);
        become DISPATCH_TABLE[(next & 0xFF) as usize](vm, next);
    }

    // Conditional jump: PC += imm if rs1 == 0
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_jz(vm: &mut Vm, instr: u64) {
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        if unlikely(vm.regs.get_unchecked(rs1(instr) as usize).i == 0) {
            let target = (vm.pc as i64).wrapping_add(imm(instr) as i64) as usize;
            if unlikely(target >= vm.code_len) {
                vm.running = false;
                return;
            }
            vm.pc = target;
        }
        become dispatch_next(vm, instr);
    }

    // Conditional jump: PC += imm if rs1 != 0
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_jnz(vm: &mut Vm, instr: u64) {
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        if likely(vm.regs.get_unchecked(rs1(instr) as usize).i != 0) {
            let target = (vm.pc as i64).wrapping_add(imm(instr) as i64) as usize;
            if unlikely(target >= vm.code_len) {
                vm.running = false;
                return;
            }
            vm.pc = target;
        }
        become dispatch_next(vm, instr);
    }

    // Subroutine call: push return address onto call_stack, then PC += imm
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_call(vm: &mut Vm, instr: u64) {
        let depth = vm.call_depth as usize;
        if unlikely(depth >= MAX_CALL_DEPTH) {
            vm.running = false;
            return;
        }
        *vm.call_stack.get_unchecked_mut(depth) = vm.pc + 1;
        vm.call_depth += 1;
        let target = (vm.pc as i64).wrapping_add(imm(instr) as i64) as usize;
        if unlikely(target >= vm.code_len) {
            vm.running = false;
            return;
        }
        vm.pc = target;
        let next = *vm.code_ptr.add(vm.pc);
        become DISPATCH_TABLE[(next & 0xFF) as usize](vm, next);
    }

    // Return from subroutine: pop return address from call_stack; if depth 0, halt.
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_ret(vm: &mut Vm, instr: u64) {
        let _ = instr;
        if unlikely(vm.call_depth == 0) {
            vm.running = false;
            return;
        }
        vm.call_depth -= 1;
        vm.pc = *vm.call_stack.get_unchecked(vm.call_depth as usize);
        let next = *vm.code_ptr.add(vm.pc);
        become DISPATCH_TABLE[(next & 0xFF) as usize](vm, next);
    }

    // в”Ђв”Ђ Data movement в”Ђв”Ђ

    // rd = rs1 (copy register value, preserves union type)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_mov(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let s = rs1(instr) as usize;
        *vm.regs.get_unchecked_mut(r) = *vm.regs.get_unchecked(s);
        become dispatch_next(vm, instr);
    }

    // rd = imm (load 32-bit signed immediate, sign-extended to i64)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_ldi(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        vm.regs.get_unchecked_mut(r).i = imm(instr) as i64;
        become dispatch_next(vm, instr);
    }

    // rd = 40-bit immediate encoded across imm(32) + rs2(8), sign-extended to i64
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_ldi64(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let low = instr >> 32;
        let high = (instr >> 24) & 0xFF;
        let val = (high << 32) | low;
        let sign = (val >> 39) & 1;
        vm.regs.get_unchecked_mut(r).i = if sign == 1 {
            (val | 0xFFFF_FF00_0000_0000) as i64
        } else {
            val as i64
        };
        become dispatch_next(vm, instr);
    }

    // rd = i64_consts[idx]  — load i64 from constant pool by index (immediate)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_ldi64_c(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let idx = immu(instr) as usize;
        if likely(idx < vm.i64_const_count as usize) {
            vm.regs.get_unchecked_mut(r).i = *vm.i64_consts_ptr.add(idx);
        }
        become dispatch_next(vm, instr);
    }

    // rd = consts[idx]  — load f64 from constant pool by index (immediate)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_ldcf64(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let idx = immu(instr) as usize;
        if likely(idx < vm.const_count as usize) {
            vm.regs.get_unchecked_mut(r).f = *vm.consts_ptr.add(idx);
        }
        become dispatch_next(vm, instr);
    }

    // rd = idx  — load string constant index into register (as i64)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_ldcstr(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let idx = immu(instr) as usize;
        debug_assert!(idx < vm.cold.const_strings.len());
        vm.regs.get_unchecked_mut(r).i = idx as i64;
        become dispatch_next(vm, instr);
    }

    // в”Ђв”Ђ Type conversion в”Ђв”Ђ

    // rd = (f64)rs1  — convert integer register to float
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_i2f(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let val = vm.regs.get_unchecked(rs1(instr) as usize).i;
        vm.regs.get_unchecked_mut(r).f = val as f64;
        become dispatch_next(vm, instr);
    }

    // rd = (i64)rs1  — convert float register to integer (truncate)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_f2i(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let val = vm.regs.get_unchecked(rs1(instr) as usize).f;
        vm.regs.get_unchecked_mut(r).i = val as i64;
        become dispatch_next(vm, instr);
    }

    // в”Ђв”Ђ Engine builtins в”Ђв”Ђ

    // rd = indicator value at string-constant index rs1 (0.0 if not found)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_getind(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let str_idx = vm.regs.get_unchecked(rs1(instr) as usize).i as usize;
        let slot = if str_idx < vm.cold.indicator_by_str.len() {
            *vm.cold.indicator_by_str.get_unchecked(str_idx)
        } else {
            u16::MAX
        };
        let val = if slot != u16::MAX {
            *vm.cold.indicators.get_unchecked(slot as usize)
        } else {
            0.0
        };
        vm.regs.get_unchecked_mut(r).f = val;
        become dispatch_next(vm, instr);
    }

    // rd = last_price (most recent trade price set by engine)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_getprice(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        vm.regs.get_unchecked_mut(r).f = vm.last_price;
        become dispatch_next(vm, instr);
    }

    // rd = position_size (current position set by engine)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_getpos(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        vm.regs.get_unchecked_mut(r).f = vm.position_size;
        become dispatch_next(vm, instr);
    }

    // rd = balance value at string-constant index rs1 (0.0 if not found)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_getbal(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let str_idx = vm.regs.get_unchecked(rs1(instr) as usize).i as usize;
        let slot = if str_idx < vm.cold.balance_by_str.len() {
            *vm.cold.balance_by_str.get_unchecked(str_idx)
        } else {
            u16::MAX
        };
        let val = if slot != u16::MAX {
            *vm.cold.balances.get_unchecked(slot as usize)
        } else {
            0.0
        };
        vm.regs.get_unchecked_mut(r).f = val;
        become dispatch_next(vm, instr);
    }

    // rd = bid quantity at depth level rs1 (0.0 if beyond book)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_getdepthbid(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let level = vm.regs.get_unchecked(rs1(instr) as usize).i as usize;
        let val = if level < vm.cold.depth_bids_len as usize {
            *vm.cold.depth_bids_qty.get_unchecked(level)
        } else {
            0.0
        };
        vm.regs.get_unchecked_mut(r).f = val;
        become dispatch_next(vm, instr);
    }

    // rd = ask quantity at depth level rs1 (0.0 if beyond book)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_getdepthask(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let level = vm.regs.get_unchecked(rs1(instr) as usize).i as usize;
        let val = if level < vm.cold.depth_asks_len as usize {
            *vm.cold.depth_asks_qty.get_unchecked(level)
        } else {
            0.0
        };
        vm.regs.get_unchecked_mut(r).f = val;
        become dispatch_next(vm, instr);
    }

    // --- SendOrder logic ---
    // Reads order parameters from fixed registers (REG_SEND_SIDE/QTY/PRICE/TYPE/REDUCE),
    // sets has_pending_order = true, and logs the order details.
    // Actual order submission is handled by the engine (outside the VM).

    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_sendorder(vm: &mut Vm, instr: u64) {
        vm.has_pending_order = true;
        tracing::debug!(
            "QFL: SEND_ORDER side={} qty={} price={} type={} reduce={}",
            vm.regs.get_unchecked(REG_SEND_SIDE as usize).i,
            vm.regs.get_unchecked(REG_SEND_QTY as usize).f,
            vm.regs.get_unchecked(REG_SEND_PRICE as usize).f,
            vm.regs.get_unchecked(REG_SEND_TYPE as usize).i,
            vm.regs.get_unchecked(REG_SEND_REDUCE as usize).i,
        );
        become dispatch_next(vm, instr);
    }

    // в”Ђв”Ђ Persist operations в”Ђв”Ђ
    // Hot-reload-safe state: tagged slots that survive across code reloads.

    // rd = persist[imm]  — read persist slot; if tag=0 read int, else read float
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_persistget(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        let slot = immu(instr) as usize;
        if likely(slot < PERSIST_SLOTS) {
            let ps = vm.cold.persist.get_unchecked(slot);
            if ps.tag == 0 {
                vm.regs.get_unchecked_mut(r).i = ps.int_val;
            } else {
                vm.regs.get_unchecked_mut(r).f = ps.float_val;
            }
        }
        become dispatch_next(vm, instr);
    }

    // persist[imm] = rd  — write persist slot; auto-tag based on register number
    // (rd >= INT_REG_COUNT в†’ float, else int)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_persistset(vm: &mut Vm, instr: u64) {
        let slot = immu(instr) as usize;
        let r = rd(instr) as usize;
        if likely(slot < PERSIST_SLOTS) {
            let ps = vm.cold.persist.get_unchecked_mut(slot);
            if r >= INT_REG_COUNT as usize {
                ps.tag = 1;
                ps.float_val = vm.regs.get_unchecked(r).f;
            } else {
                ps.tag = 0;
                ps.int_val = vm.regs.get_unchecked(r).i;
            }
        }
        become dispatch_next(vm, instr);
    }

    // в”Ђв”Ђ Logging в”Ђв”Ђ

    // Log message from string constant at rs1 (no value).
    // Pushes to ring buffer in debug builds; dumped to qflvm.log on graceful shutdown.
    // No-op in release builds (the ColdVm struct omits log_buffer when cfg(not(debug_assertions))).
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_log(vm: &mut Vm, instr: u64) {
        #[cfg(debug_assertions)]
        {
            let str_idx = vm.regs.get_unchecked(rs1(instr) as usize).i as usize;
            let msg = if str_idx < vm.cold.const_strings.len() {
                vm.cold.const_strings.get_unchecked(str_idx).as_str()
            } else {
                ""
            };
            if let Some(ref mut buf) = vm.cold.log_buffer {
                buf.push(format!("QFL: {}", msg));
            }
        }
        become dispatch_next(vm, instr);
    }

    // Log message from string constant at rs1 with f64 value from rs2.
    // Pushes to ring buffer in debug builds; dumped to qflvm.log on graceful shutdown.
    // No-op in release builds (the ColdVm struct omits log_buffer when cfg(not(debug_assertions))).
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_log2(vm: &mut Vm, instr: u64) {
        #[cfg(debug_assertions)]
        {
            let str_idx = vm.regs.get_unchecked(rs1(instr) as usize).i as usize;
            let val = vm.regs.get_unchecked(rs2(instr) as usize).f;
            if let Some(ref mut buf) = vm.cold.log_buffer {
                if str_idx < vm.cold.const_strings.len() {
                    buf.push(format!(
                        "QFL: {}: {}",
                        vm.cold.const_strings.get_unchecked(str_idx),
                        val
                    ));
                } else {
                    buf.push(format!("QFL: {}", val));
                }
            }
        }
        become dispatch_next(vm, instr);
    }

    // Halts execution by setting running = false.
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_halt(vm: &mut Vm, instr: u64) {
        let _ = instr;
        vm.running = false;
    }

    // в”Ђв”Ђ Window opcodes в”Ђв”Ђ
    // Rolling window operations: push a value, query aggregates (mean, stddev, min, max, sum).

    // Push value onto window wid. Maintains ring buffer, running sum/sum_sq,
    // and O(1) min/max via monotonic deques. Returns the pushed value in rd.
    // On first push, auto-initializes the window with capacity=64 at offset=wid*64.
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_windowpush(vm: &mut Vm, instr: u64) {
        let wid = immu(instr) as usize;
        let val = vm.regs.get_unchecked(rs1(instr) as usize).f;
        let r = rd(instr) as usize;
        if unlikely(wid >= MAX_WINDOWS) {
            become dispatch_next(vm, instr);
        }
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

        // Software prefetch: bring next ring buffer slot and current into L1
        let arena_ptr = vm.cold.window_arena.as_ptr();
        let next_head = (head + 1) % cap;
        _mm_prefetch(arena_ptr.add(off + next_head) as *const i8, 0);
        _mm_prefetch(arena_ptr.add(off + head) as *const i8, 1);

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
        become dispatch_next(vm, instr);
    }

    // Macro generating window unary query handlers (mean, stddev, min, max, sum).
    // Reads the result from WindowMeta into rd. Does nothing if window empty.
    macro_rules! window_unary {
        ($name:ident, $method:ident) => {
            /// # Safety
            /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
            #[inline(never)]
            pub unsafe fn $name(vm: &mut Vm, instr: u64) {
                let wid = immu(instr) as usize;
                let r = rd(instr) as usize;
                if unlikely(wid >= MAX_WINDOWS) {
                    become dispatch_next(vm, instr);
                }
                let meta = vm.cold.window_meta.get_unchecked(wid);
                let result = meta.$method();
                if meta.len > 0 {
                    vm.regs.get_unchecked_mut(r).f = result;
                }
                become dispatch_next(vm, instr);
            }
        };
    }

    window_unary!(vm_windowmean, mean); // rd = mean of window wid
    window_unary!(vm_windowstddev, stddev); // rd = stddev of window wid
    window_unary!(vm_windowmin, min); // rd = min of window wid
    window_unary!(vm_windowmax, max); // rd = max of window wid
    window_unary!(vm_windowsum, sum); // rd = sum of window wid

    // в”Ђв”Ђ Phase 4g: fused feature opcodes в”Ђв”Ђ

    // EMA update: rd = ema_state[sid].update(rs1)
    // On first call (initialized=false), seeds value = input.
    // On subsequent calls: value = alpha * input + (1-alpha) * prev_value.
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
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
        become dispatch_next(vm, instr);
    }

    // в”Ђв”Ђ Power operations в”Ђв”Ђ

    // rd = rs1 ^ rs2  (integer exponentiation via exponentiation by squaring)
    // Returns 0 for negative exp, 1 for exp == 0.
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
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
        become dispatch_next(vm, instr);
    }

    // rd = rs1 ^ rs2  (float exponentiation via powf)
    /// # Safety
    /// Caller must ensure the VM is in a valid state with initialized code pointer and register file.
    #[inline(always)]
    pub unsafe fn vm_fpow(vm: &mut Vm, instr: u64) {
        let r = rd(instr) as usize;
        debug_assert!(r < NUM_REGS);
        debug_assert!((rs1(instr) as usize) < NUM_REGS);
        debug_assert!((rs2(instr) as usize) < NUM_REGS);
        let a = vm.regs.get_unchecked(rs1(instr) as usize).f;
        let b = vm.regs.get_unchecked(rs2(instr) as usize).f;
        vm.regs.get_unchecked_mut(r).f = sanitize_f(a.powf(b));
        become dispatch_next(vm, instr);
    }
}
