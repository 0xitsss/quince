// SPDX-FileCopyrightText: 2026 0xitsss
//
// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Quince-Commercial
//! QFL VM performance profiler.
//!
//! Tracks opcode execution counts, per-opcode RDTSC cycles, and per-handler
//! timing. Zero-allocation in the hot path when `None`.
//!
//! Entry points: [`Profiler::record_opcode()`], [`Profiler::profile()`].

use crate::opcodes::Opcode;
use std::time::Instant;

/// Read the x86_64 timestamp counter (RDTSC) for cycle-accurate profiling.
/// Returns 0 on non-x86 platforms (no cycle data available).
#[inline]
pub fn rdtsc() -> u64 {
    #[cfg(all(target_arch = "x86_64", feature = "profiling"))]
    {
        unsafe { std::arch::x86_64::_rdtsc() }
    }
    #[cfg(not(all(target_arch = "x86_64", feature = "profiling")))]
    {
        0
    }
}

/// Opcode execution profile for a single run.
#[derive(Debug, Clone)]
pub struct OpcodeProfile {
    pub opcode: Opcode,
    pub count: u64,
    pub cycles: u64,
}

/// Per-handler timing sample.
#[derive(Debug, Clone)]
pub struct HandlerSample {
    pub name: String,
    pub elapsed_ns: u64,
    pub instr_count: u64,
}

/// Execution profiler.
#[derive(Debug, Clone)]
pub struct Profiler {
    pub(crate) opcode_counts: [u64; 65], // 0..=MaxOpcode (64)
    pub(crate) opcode_cycles: [u64; 65], // accumulated RDTSC cycles per opcode
    pub(crate) handler_samples: Vec<HandlerSample>,
    pub(crate) current_handler: Option<String>,
    pub(crate) handler_start: Option<Instant>,
    pub(crate) handler_start_instr: u64,
    /// Total instruction count across all executions.
    pub total_instructions: u64,
}

impl Profiler {
    pub fn new() -> Self {
        Profiler {
            opcode_counts: [0u64; 65],
            opcode_cycles: [0u64; 65],
            handler_samples: Vec::new(),
            current_handler: None,
            handler_start: None,
            handler_start_instr: 0,
            total_instructions: 0,
        }
    }

    /// Record one executed opcode.
    #[inline]
    pub fn record_opcode(&mut self, op: Opcode) {
        self.opcode_counts[op as u8 as usize] += 1;
        self.total_instructions += 1;
    }

    /// Record one executed opcode with RDTSC cycle delta.
    #[inline]
    pub fn record_opcode_tsc(&mut self, op: Opcode, cycles: u64) {
        let idx = op as u8 as usize;
        self.opcode_counts[idx] += 1;
        self.opcode_cycles[idx] += cycles;
        self.total_instructions += 1;
    }

    /// Start timing a handler.
    pub fn start_handler(&mut self, name: &str) {
        self.current_handler = Some(name.to_string());
        self.handler_start = Some(Instant::now());
        self.handler_start_instr = self.total_instructions;
    }

    /// End timing the current handler and record the sample.
    pub fn end_handler(&mut self) {
        if let (Some(name), Some(start)) = (self.current_handler.take(), self.handler_start.take())
        {
            let elapsed = start.elapsed();
            let instr_count = self.total_instructions - self.handler_start_instr;
            self.handler_samples.push(HandlerSample {
                name,
                elapsed_ns: elapsed.as_nanos() as u64,
                instr_count,
            });
        }
    }

    /// Get per-opcode execution counts and cycles (sorted descending by count).
    pub fn opcode_profile(&self) -> Vec<OpcodeProfile> {
        let mut profiles: Vec<OpcodeProfile> = self
            .opcode_counts
            .iter()
            .enumerate()
            .filter(|(_, &c)| c > 0)
            .map(|(i, &count)| OpcodeProfile {
                opcode: Opcode::from_u8(i as u8),
                count,
                cycles: self.opcode_cycles[i],
            })
            .collect();
        profiles.sort_by_key(|a| std::cmp::Reverse(a.count));
        profiles
    }

    /// Get all handler timing samples.
    pub fn handler_samples(&self) -> &[HandlerSample] {
        &self.handler_samples
    }

    /// Get mean handler latency in ns for a specific handler.
    pub fn mean_handler_ns(&self, name: &str) -> u64 {
        let samples: Vec<&HandlerSample> = self
            .handler_samples
            .iter()
            .filter(|s| s.name == name)
            .collect();
        if samples.is_empty() {
            return 0;
        }
        let sum: u64 = samples.iter().map(|s| s.elapsed_ns).sum();
        sum / samples.len() as u64
    }

    /// Reset all counters.
    pub fn reset(&mut self) {
        self.opcode_counts = [0u64; 65];
        self.opcode_cycles = [0u64; 65];
        self.handler_samples.clear();
        self.current_handler = None;
        self.handler_start = None;
        self.handler_start_instr = 0;
        self.total_instructions = 0;
    }
}

impl Default for Profiler {
    fn default() -> Self {
        Profiler::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcodes::Opcode as O;

    #[test]
    fn new_profiler_has_zero_counts() {
        let p = Profiler::new();
        assert_eq!(p.total_instructions, 0);
        assert!(p.opcode_profile().is_empty());
    }

    #[test]
    fn record_opcode_increments_count() {
        let mut p = Profiler::new();
        p.record_opcode(O::Add);
        p.record_opcode(O::Add);
        p.record_opcode(O::Sub);
        let profile = p.opcode_profile();
        assert_eq!(p.total_instructions, 3);
        // Add should have count 2
        let add_count = profile
            .iter()
            .find(|x| x.opcode == O::Add)
            .map(|x| x.count)
            .unwrap_or(0);
        assert_eq!(add_count, 2);
    }

    #[test]
    fn handler_timing_records_sample() {
        let mut p = Profiler::new();
        p.start_handler("on_trade");
        p.record_opcode(O::Ldi);
        p.record_opcode(O::Add);
        p.end_handler();

        assert_eq!(p.handler_samples().len(), 1);
        assert_eq!(p.handler_samples()[0].name, "on_trade");
        assert_eq!(p.handler_samples()[0].instr_count, 2);
    }

    #[test]
    fn handler_timing_multiple_samples() {
        let mut p = Profiler::new();
        p.start_handler("on_eval");
        p.record_opcode(O::Ldi);
        p.end_handler();
        p.start_handler("on_eval");
        p.record_opcode(O::Ldi);
        p.record_opcode(O::Ldi);
        p.end_handler();

        assert_eq!(p.handler_samples().len(), 2);
        assert_eq!(p.handler_samples()[0].name, "on_eval");
        assert_eq!(p.handler_samples()[1].instr_count, 2);
        // mean_ns should be > 0 since real time passes
        assert!(p.mean_handler_ns("on_eval") > 0);
    }

    #[test]
    fn opcode_profile_sorted_descending() {
        let mut p = Profiler::new();
        p.record_opcode(O::Sub);
        p.record_opcode(O::Sub);
        p.record_opcode(O::Sub);
        p.record_opcode(O::Add);
        p.record_opcode(O::Add);
        p.record_opcode(O::Mul);

        let profile = p.opcode_profile();
        assert_eq!(profile.len(), 3);
        assert!(profile[0].count >= profile[1].count);
        assert!(profile[1].count >= profile[2].count);
    }

    #[test]
    fn reset_clears_all() {
        let mut p = Profiler::new();
        p.record_opcode(O::Add);
        p.start_handler("test");
        p.end_handler();
        p.reset();
        assert_eq!(p.total_instructions, 0);
        assert!(p.handler_samples().is_empty());
        assert!(p.opcode_profile().is_empty());
    }

    #[test]
    fn end_handler_without_start_does_not_crash() {
        let mut p = Profiler::new();
        p.end_handler(); // no-op
    }

    #[test]
    fn record_opcode_max_opcode() {
        let mut p = Profiler::new();
        p.record_opcode(O::Halt);
        assert!(p.total_instructions > 0);
    }

    #[test]
    fn mean_handler_ns_for_unknown_handler_returns_zero() {
        let p = Profiler::new();
        assert_eq!(p.mean_handler_ns("nonexistent"), 0);
    }

    #[test]
    fn handler_timing_respects_instr_boundary() {
        let mut p = Profiler::new();
        // Record some instructions before handler
        p.record_opcode(O::Ldi);
        p.start_handler("on_trade");
        p.record_opcode(O::Add);
        p.record_opcode(O::Sub);
        p.end_handler();
        // Record some after
        p.record_opcode(O::Ret);

        assert_eq!(p.handler_samples()[0].instr_count, 2);
        assert_eq!(p.total_instructions, 4); // Ldi, Add, Sub, Ret
    }

    #[test]
    fn handler_timing_nested_starts_overwrites() {
        let mut p = Profiler::new();
        p.start_handler("first");
        p.record_opcode(O::Ldi);
        p.start_handler("second"); // overwrites
        p.record_opcode(O::Add);
        p.end_handler();

        assert_eq!(p.handler_samples().len(), 1);
        assert_eq!(p.handler_samples()[0].name, "second");
    }

    #[test]
    fn profile_default_is_empty() {
        let p = Profiler::default();
        assert_eq!(p.total_instructions, 0);
    }
}
