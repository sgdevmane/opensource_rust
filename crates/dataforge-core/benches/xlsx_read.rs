use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_xlsx_placeholder(c: &mut Criterion) {
    c.bench_function("xlsx_read_placeholder", |b| b.iter(|| black_box(42)));
}

criterion_group!(benches, bench_xlsx_placeholder);
criterion_main!(benches);
