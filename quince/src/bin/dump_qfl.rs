use std::env;
use quince_qfl::compiler::compile;
use quince_qfl::parser::parse;
use quince_qfl::optimize::optimize;
use quince_qfl::ir::QfrProgram;

fn dump_prog(label: &str, prog: &QfrProgram) {
    println!("═══ {label} ═══");
    println!("  entries: {}  instrs: {}  consts: {}",
        prog.entries.len(), prog.code.len(), prog.const_pool.len());
    for e in &prog.entries {
        println!("  entry {} @{}", e.name, e.code_offset);
    }
    for (i, instr) in prog.code.iter().enumerate() {
        let marker = if prog.entries.iter().any(|e| e.code_offset as usize == i) {
            "→"
        } else {
            " "
        };
        println!("  {marker} {:>4}: {instr}", i);
    }
    println!();
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: dump_qfl <file.qfl> [file2.qfl ...]");
        std::process::exit(1);
    }

    for path in &args[1..] {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => { eprintln!("read {path}: {e}"); continue; }
        };
        let program = match parse(&src) {
            Ok(p) => p,
            Err(e) => { eprintln!("parse {path}: {e}"); continue; }
        };
        let mut before = match compile(&program) {
            Ok(p) => p,
            Err(e) => { eprintln!("compile {path}: {:?}", e); continue; }
        };
        optimize(&mut before);

        println!("────────────────────────────────────────────");
        println!("  file: {path}");
        dump_prog("BEFORE", &before);
        dump_prog("AFTER (optimized)", &before);
    }
}
