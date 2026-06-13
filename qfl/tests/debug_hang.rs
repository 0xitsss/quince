use quince_qfl::*;

fn test_qfl(name: &str, src: &str) {
    let prog = parser::parse(src).expect("parse");
    let mut qfr = compiler::compile_checked(&prog).expect("compile");
    optimize::optimize(&mut qfr);
    let mut vm = vm::Vm::new(qfr);
    vm.regs[0].f = 100.0;
    vm.regs[1].f = 1.0;
    vm.regs[3].i = 1;
    vm.call("on_trade");
    println!("  {} OK", name);
}

#[test]
fn debug_all_patterns() {
    test_qfl(
        "with_persist",
        r#"
@persist x : i64 = 0
on trade(t) {
    local p = t.price
    if p > 50 {
        x = 1
    }
}
"#,
    );

    test_qfl(
        "with_using",
        r#"
@using ema:5:10
on trade(t) {
    local p = t.price
    if p > 50 {
        quince.log("test")
    }
}
"#,
    );

    test_qfl(
        "with_get",
        r#"
@using ema:5:10
on trade(t) {
    local p = t.price
    local v = quince.get("ema5")
    if p > 50 {
        quince.log("test")
    }
}
"#,
    );

    test_qfl(
        "persist_get_if",
        r#"
@persist x : i64 = 0
on trade(t) {
    local p = t.price
    if x > 0 {
        if p < 50 {
            quince.log("nested")
        }
    }
}
"#,
    );

    test_qfl(
        "nested_if_persist",
        r#"
@persist x : i64 = 0
@persist y : f64 = 0.0
on trade(t) {
    local p = t.price
    if x > 0 {
        if p < 50 {
            x = 0
            y = 0.0
        }
    }
}
"#,
    );

    test_qfl(
        "order_in_nested",
        r#"
@persist pos : i64 = 0
on trade(t) {
    local p = t.price
    if pos > 0 {
        if p < 50 {
            quince.order(1, 1.0, 0)
            pos = 0
        }
    }
}
"#,
    );

    test_qfl(
        "compound_second_if",
        r#"
@persist pos : i64 = 0
@using ema:5:10
on trade(t) {
    local p = t.price
    local v = quince.get("ema5")
    if pos > 0 {
        if p < 50 {
            quince.order(1, 1.0, 0)
            pos = 0
        }
    }
    if pos <= 0 and v > 0 {
        quince.order(0, 1.0, 0)
        pos = 1
    }
}
"#,
    );

    println!("ALL OK");
}
