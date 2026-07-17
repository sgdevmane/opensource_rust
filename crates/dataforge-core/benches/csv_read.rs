use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_csv_placeholder(c: &mut Criterion) {
    c.bench_function("csv_read_placeholder", |b| b.iter(|| black_box(42)));
}

criterion_group!(benches, bench_csv_placeholder);
criterion_main!(benches);
