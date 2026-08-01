use criterion::{Criterion, criterion_group, criterion_main};
use path_tracer::render::scene::Scene;
use std::hint::black_box;

fn benchmark_scene_loading(c: &mut Criterion) {
    c.bench_function("scene_new_dorm_light", |b| {
        b.iter(|| {
            pollster::block_on(Scene::new(black_box("assets/dorm_light.glb")))
                .expect("Failed to load scene")
        });
    });

    c.bench_function("scene_mc", |b| {
        b.iter(|| {
            pollster::block_on(Scene::new(black_box("assets/mc.glb")))
                .expect("Failed to load scene")
        });
    });

    c.bench_function("scene_dragon", |b| {
        b.iter(|| {
            pollster::block_on(Scene::new(black_box("assets/dragon.glb")))
                .expect("Failed to load scene")
        });
    });
}

fn benchmark_pick_material(c: &mut Criterion) {
    let mut scene = pollster::block_on(Scene::new(black_box("assets/cornell_custom.glb")))
        .expect("Failed to load scene");
    scene.camera.translate_and_rotate_to(
        Some(glam::Vec3::new(0.0, 0.0, 6.0)),
        Some(20.0_f32.to_radians()),
        Some(10.0_f32.to_radians()),
    );

    c.bench_function("pick_material_cornell_custom", |b| {
        b.iter(|| {
            let result = scene.takeout_gpu_camera();
            black_box(result);
        });
    });
}

criterion_group!(benches, benchmark_scene_loading, benchmark_pick_material);
criterion_main!(benches);
