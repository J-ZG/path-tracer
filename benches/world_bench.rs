use criterion::{Criterion, criterion_group, criterion_main};
use path_tracer::render::world::Scene;
use std::hint::black_box;

fn benchmark_scene_loading(c: &mut Criterion) {
    let bytes_box = std::fs::read("assets/glb/box.glb").expect("failed to load scene");
    let bytes_dragon = std::fs::read("assets/glb/dragon.glb").expect("failed to load scene");

    c.bench_function("scene_new_box", |b| {
        b.iter(|| Scene::new(&bytes_box).expect("failed to load scene"));
    });

    let mut group = c.benchmark_group("slow_scenes");
    group.sample_size(30);
    group.bench_function("scene_new_dragon", |b| {
        b.iter(|| Scene::new(&bytes_dragon).expect("failed to load scene"));
    });
    group.finish();
}

fn benchmark_pick_material(c: &mut Criterion) {
    let bytes = std::fs::read("assets/glb/box.glb").expect("failed to load scene");

    let scene = Scene::new(&bytes).expect("failed to load scene");

    c.bench_function("pick_material_box", |b| {
        b.iter(|| {
            let result = scene.pick_material(
                black_box(glam::Vec3::new(-3.42, 3.54, 3.76)),
                black_box(-1.11),
                black_box(-0.84),
            );
            black_box(result);
        });
    });
}

criterion_group!(benches, benchmark_scene_loading, benchmark_pick_material);
criterion_main!(benches);
