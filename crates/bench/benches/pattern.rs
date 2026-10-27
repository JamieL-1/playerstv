//! The test game draws every frame on the CPU; this keeps that cost known
//! and small next to what a recorder costs.
use criterion::{criterion_group, criterion_main, Criterion};
use ptv_bench::pattern::Pattern;

fn render(c: &mut Criterion) {
    for (w, h) in [(1920, 1080), (2560, 1440)] {
        let p = Pattern::new(w, h, 60);
        let mut buf = vec![0; p.frame_bytes()];
        let mut index = 0;
        c.bench_function(&format!("pattern::render {w}x{h}"), |b| {
            b.iter(|| {
                p.render(index, &mut buf);
                index += 1;
            })
        });
    }
}

criterion_group!(benches, render);
criterion_main!(benches);
