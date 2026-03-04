use std::hint::black_box;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use detonito_core::{GameConfig, LayoutGenerator, MineLayout, NoGuessLayoutGenerator};
use rand::{RngExt, SeedableRng, rngs::SmallRng};

fn generate_no_guess_layout(config: GameConfig, seed: u64) -> MineLayout {
    let first_move = (config.size.0 / 2, config.size.1 / 2);
    NoGuessLayoutGenerator::new(seed, first_move).generate(config)
}

struct TierSpec {
    name: &'static str,
    config: GameConfig,
    seed_pool_size: usize,
    meta_seed: u64,
    measure_secs: u64,
}

fn fixed_seeds(meta_seed: u64, count: usize) -> Vec<u64> {
    let mut rng = SmallRng::seed_from_u64(meta_seed);
    (0..count).map(|_| rng.random::<u64>()).collect()
}

fn tiers() -> Vec<TierSpec> {
    vec![
        TierSpec {
            name: "beginner",
            config: GameConfig::new_unchecked((9, 9), 10),
            seed_pool_size: 30,
            meta_seed: 0xBEEF_0001,
            measure_secs: 5,
        },
        TierSpec {
            name: "intermediate",
            config: GameConfig::new_unchecked((16, 16), 40),
            seed_pool_size: 20,
            meta_seed: 0xBEEF_0002,
            measure_secs: 10,
        },
        TierSpec {
            name: "expert",
            config: GameConfig::new_unchecked((30, 16), 99),
            seed_pool_size: 10,
            meta_seed: 0xBEEF_0003,
            measure_secs: 50,
        },
        TierSpec {
            name: "evil",
            config: GameConfig::new_unchecked((30, 20), 130),
            seed_pool_size: 10,
            meta_seed: 0xBEEF_0004,
            measure_secs: 100,
        },
    ]
}

fn bench_gen_tiers(c: &mut Criterion) {
    for spec in tiers() {
        let mut group = c.benchmark_group(format!("gen_tiers/{}", spec.name));
        let seeds = fixed_seeds(spec.meta_seed, spec.seed_pool_size);
        assert!(!seeds.is_empty(), "seed pool must not be empty");
        let mut seed_idx = 0usize;

        group.sample_size(10);
        group.measurement_time(Duration::from_secs(spec.measure_secs));
        group.throughput(Throughput::Elements(1));

        group.bench_with_input(
            BenchmarkId::new("no_guess", "single_layout"),
            &spec.config,
            |b, &config| {
                b.iter(|| {
                    let seed = seeds[seed_idx % seeds.len()];
                    seed_idx = seed_idx.wrapping_add(1);
                    black_box(generate_no_guess_layout(black_box(config), black_box(seed)));
                });
            },
        );

        group.finish();
    }
}

criterion_group!(gen_benches, bench_gen_tiers);
criterion_main!(gen_benches);
