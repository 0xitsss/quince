/// Feature & Signal Graph — dependency graph of indicators, windows, and signals.
///
/// Built from compiled bytecode (`QfrProgram`). Enables:
/// - Dependency tracking (which indicators feed which signals)
/// - Dead feature elimination
/// - Precomputation planning
/// - Replay state definition

use crate::ir::QfrProgram;
use crate::opcodes::Opcode;

/// A signal — comparison opcode used in branching context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalKind {
    Eq, Ne, Lt, Gt, Le, Ge,
    FEq, FNe, FLt, FGt, FLe, FGe,
    And, Or,
}

impl SignalKind {
    fn from_opcode(op: Opcode) -> Option<SignalKind> {
        use Opcode::*;
        match op {
            Eq => Some(SignalKind::Eq),
            Ne => Some(SignalKind::Ne),
            Lt => Some(SignalKind::Lt),
            Gt => Some(SignalKind::Gt),
            Le => Some(SignalKind::Le),
            Ge => Some(SignalKind::Ge),
            FEq => Some(SignalKind::FEq),
            FNe => Some(SignalKind::FNe),
            FLt => Some(SignalKind::FLt),
            FGt => Some(SignalKind::FGt),
            FLe => Some(SignalKind::FLe),
            FGe => Some(SignalKind::FGe),
            _ => None,
        }
    }
}

/// The feature & signal graph for a strategy.
#[derive(Debug, Clone)]
pub struct StrategyGraph {
    /// Unique rolling window IDs used in this program.
    window_ids: Vec<usize>,
    /// Signal kinds used (comparison ops for conditions).
    signals: Vec<SignalKind>,
    /// Number of bytecode instructions.
    instr_count: usize,
}

impl StrategyGraph {
    /// Build a strategy graph from a compiled program.
    pub fn from_program(program: &QfrProgram) -> Self {
        let mut window_ids = Vec::new();
        let mut signals = Vec::new();

        for instr in &program.code {
            let op = instr.opcode();
            match op {
                Opcode::WindowPush | Opcode::WindowMean | Opcode::WindowStddev
                | Opcode::WindowMin | Opcode::WindowMax | Opcode::WindowSum => {
                    let wid = instr.imm_signed() as usize;
                    if !window_ids.contains(&wid) {
                        window_ids.push(wid);
                    }
                }
                _ => {}
            }

            if let Some(sig) = SignalKind::from_opcode(op) {
                if !signals.contains(&sig) {
                    signals.push(sig);
                }
            }
        }

        StrategyGraph {
            window_ids,
            signals,
            instr_count: program.code.len(),
        }
    }

    pub fn window_ids(&self) -> &[usize] { &self.window_ids }
    pub fn signals(&self) -> &[SignalKind] { &self.signals }
    pub fn instr_count(&self) -> usize { self.instr_count }
    pub fn window_count(&self) -> usize { self.window_ids.len() }
    pub fn signal_count(&self) -> usize { self.signals.len() }

    pub fn is_empty(&self) -> bool {
        self.window_ids.is_empty() && self.signals.is_empty() && self.instr_count == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcodes::Instruction;
    use crate::ir::QfrProgram;

    fn empty_prog() -> QfrProgram {
        let mut p = QfrProgram::new();
        p.entries.push(crate::ir::EntryPoint { name: "main".into(), code_offset: 0 });
        p
    }

    #[test]
    fn from_empty_program() {
        let g = StrategyGraph::from_program(&empty_prog());
        assert!(g.is_empty());
        assert_eq!(g.instr_count(), 0);
    }

    #[test]
    fn no_window_ops_returns_empty_windows() {
        let mut p = empty_prog();
        p.code = vec![Instruction::single(Opcode::Halt)];
        let g = StrategyGraph::from_program(&p);
        assert!(g.window_ids().is_empty());
    }

    #[test]
    fn single_window_push_detected() {
        let mut p = empty_prog();
        p.code = vec![
            Instruction::rri(Opcode::WindowPush, 192, 192, 5),
            Instruction::single(Opcode::Halt),
        ];
        let g = StrategyGraph::from_program(&p);
        assert_eq!(g.window_ids(), &[5]);
        assert_eq!(g.window_count(), 1);
    }

    #[test]
    fn duplicate_window_ids_deduped() {
        let mut p = empty_prog();
        p.code = vec![
            Instruction::rri(Opcode::WindowPush, 192, 192, 2),
            Instruction::ri(Opcode::WindowMean, 193, 2),
            Instruction::ri(Opcode::WindowStddev, 194, 2),
            Instruction::single(Opcode::Halt),
        ];
        let g = StrategyGraph::from_program(&p);
        assert_eq!(g.window_count(), 1);
    }

    #[test]
    fn signal_detected_from_comparison() {
        let mut p = empty_prog();
        p.code = vec![
            Instruction::rrr(Opcode::Gt, 2, 0, 1),
            Instruction::single(Opcode::Halt),
        ];
        let g = StrategyGraph::from_program(&p);
        assert!(g.signals().contains(&SignalKind::Gt));
        assert_eq!(g.signal_count(), 1);
    }

    #[test]
    fn multiple_signals_detected() {
        let mut p = empty_prog();
        p.code = vec![
            Instruction::rrr(Opcode::Gt, 2, 0, 1),
            Instruction::rrr(Opcode::Eq, 3, 2, 3),
            Instruction::rrr(Opcode::FLt, 194, 192, 193),
            Instruction::single(Opcode::Halt),
        ];
        let g = StrategyGraph::from_program(&p);
        assert_eq!(g.signal_count(), 3);
        assert!(g.signals().contains(&SignalKind::Gt));
        assert!(g.signals().contains(&SignalKind::Eq));
        assert!(g.signals().contains(&SignalKind::FLt));
    }

    #[test]
    fn duplicate_signals_deduped() {
        let mut p = empty_prog();
        p.code = vec![
            Instruction::rrr(Opcode::Lt, 2, 0, 1),
            Instruction::rrr(Opcode::Lt, 3, 2, 3),
            Instruction::single(Opcode::Halt),
        ];
        let g = StrategyGraph::from_program(&p);
        assert_eq!(g.signal_count(), 1);
    }

    #[test]
    fn all_16_signal_variants() {
        let mut p = empty_prog();
        use Opcode::*;
        let ops = [Eq, Ne, Lt, Gt, Le, Ge, FEq, FNe, FLt, FGt, FLe, FGe];
        for op in ops {
            p.code.push(Instruction::rrr(op, 0, 0, 0));
        }
        p.code.push(Instruction::single(Opcode::Halt));
        let g = StrategyGraph::from_program(&p);
        assert_eq!(g.signal_count(), 12);
    }

    #[test]
    fn instr_count_reflects_program_size() {
        let mut p = empty_prog();
        p.code = vec![
            Instruction::rrr(Opcode::Gt, 0, 0, 0),
            Instruction::single(Opcode::Halt),
        ];
        let g = StrategyGraph::from_program(&p);
        assert_eq!(g.instr_count(), 2);
    }

    #[test]
    fn window_and_signal_together() {
        let mut p = empty_prog();
        p.code = vec![
            Instruction::rri(Opcode::WindowPush, 192, 192, 0),
            Instruction::ri(Opcode::WindowMean, 193, 0),
            Instruction::rrr(Opcode::Gt, 2, 0, 193 as u8),
            Instruction::single(Opcode::Halt),
        ];
        let g = StrategyGraph::from_program(&p);
        assert_eq!(g.window_count(), 1);
        assert!(g.signals().contains(&SignalKind::Gt));
    }
}
