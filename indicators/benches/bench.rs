use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

use quince_indicators::*;

fn bench_wma(c: &mut Criterion) {
    let mut group = c.benchmark_group("wma");
    for period in [10usize, 50, 200] {
        group.bench_with_input(BenchmarkId::new("update", period), &period, |b, &p| {
            let mut wma = ma::Wma::new(p);
            for i in 0..p + 5 {
                wma.update((i as f64).sin());
            }
            b.iter(|| {
                for i in 0..100 {
                    black_box(wma.update((i as f64).sin()));
                }
            });
        });
    }
    group.finish();
}

fn bench_lsma(c: &mut Criterion) {
    let mut group = c.benchmark_group("lsma");
    for period in [10usize, 50, 200] {
        group.bench_with_input(BenchmarkId::new("update", period), &period, |b, &p| {
            let mut lsma = ma::Lsma::new(p);
            for i in 0..p + 5 {
                lsma.update((i as f64).sin());
            }
            b.iter(|| {
                for i in 0..100 {
                    black_box(lsma.update((i as f64).sin()));
                }
            });
        });
    }
    group.finish();
}

fn bench_cci(c: &mut Criterion) {
    let mut group = c.benchmark_group("cci");
    for period in [14usize, 50, 200] {
        group.bench_with_input(BenchmarkId::new("update", period), &period, |b, &p| {
            let mut cci = oscillator::Cci::new(p, 0.015);
            for i in 0..p + 5 {
                let v = (i as f64).sin();
                cci.update(v, v, v);
            }
            b.iter(|| {
                for i in 0..100 {
                    let v = (i as f64).sin();
                    black_box(cci.update(v, v, v));
                }
            });
        });
    }
    group.finish();
}

fn bench_stochastic(c: &mut Criterion) {
    let mut group = c.benchmark_group("stochastic");
    for period in [14usize, 50, 200] {
        group.bench_with_input(BenchmarkId::new("update", period), &period, |b, &p| {
            let mut stoch = oscillator::Stochastic::new(p);
            for i in 0..p + 5 {
                let v = (i as f64).sin();
                stoch.update(v, v, v);
            }
            b.iter(|| {
                for i in 0..100 {
                    let v = (i as f64).sin();
                    black_box(stoch.update(v, v, v));
                }
            });
        });
    }
    group.finish();
}

fn bench_mfi(c: &mut Criterion) {
    let mut group = c.benchmark_group("mfi");
    for period in [14usize, 50, 200] {
        group.bench_with_input(BenchmarkId::new("update", period), &period, |b, &p| {
            let mut mfi = flow::Mfi::new(p);
            for i in 0..p + 5 {
                let candle = Candle::new(
                    (i as f64).sin(),
                    (i as f64).sin() + 1.0,
                    (i as f64).sin() - 1.0,
                    (i as f64).sin(),
                    100.0,
                );
                mfi.update(&candle);
            }
            b.iter(|| {
                for i in 0..100 {
                    let candle = Candle::new(
                        (i as f64).sin(),
                        (i as f64).sin() + 1.0,
                        (i as f64).sin() - 1.0,
                        (i as f64).sin(),
                        100.0,
                    );
                    black_box(mfi.update(&candle));
                }
            });
        });
    }
    group.finish();
}

fn bench_bollinger(c: &mut Criterion) {
    let mut group = c.benchmark_group("bollinger");
    for period in [20usize, 100, 200] {
        group.bench_with_input(BenchmarkId::new("update", period), &period, |b, &p| {
            let mut bb = volatility::BollingerBands::new(p, 2.0);
            for i in 0..p + 5 {
                bb.update((i as f64).sin());
            }
            b.iter(|| {
                for i in 0..100 {
                    black_box(bb.update((i as f64).sin()));
                }
            });
        });
    }
    group.finish();
}

fn bench_zscore(c: &mut Criterion) {
    let mut group = c.benchmark_group("zscore");
    for period in [20usize, 100, 200] {
        group.bench_with_input(BenchmarkId::new("update", period), &period, |b, &p| {
            let mut z = structure::ZScore::new(p);
            for i in 0..p + 5 {
                z.update((i as f64).sin());
            }
            b.iter(|| {
                for i in 0..100 {
                    black_box(z.update((i as f64).sin()));
                }
            });
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_wma,
    bench_lsma,
    bench_cci,
    bench_stochastic,
    bench_mfi,
    bench_bollinger,
    bench_zscore,
);
criterion_main!(benches);
