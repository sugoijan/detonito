use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use detonito_core::{
    EngineCell, EngineState, MineLayout, Observation, PlayEngine, RevealOutcome,
    SOLVER_TIERS_CORPUS_VERSION, SolveDepth, SolverConfig, SolverTiersCorpus, SolverTiersScenario,
    StatefulSolver, parse_solver_tiers_corpus_json,
};

fn default_corpus_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("solver_tiers_corpus.json")
}

fn corpus_path() -> PathBuf {
    std::env::var("DETONITO_SOLVER_TIERS_CORPUS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_corpus_path())
}

fn load_corpus(path: &Path) -> SolverTiersCorpus {
    let input = fs::read_to_string(path).unwrap_or_else(|err| {
        panic!(
            "failed to read solver tiers corpus at `{}`: {err}. \
             Run `cargo run -p detonito-core --bin gen_solver_tiers_corpus` first.",
            path.display()
        )
    });

    let corpus: SolverTiersCorpus = parse_solver_tiers_corpus_json(&input)
        .unwrap_or_else(|err| panic!("failed to parse corpus at `{}`: {err}", path.display()));

    assert_eq!(
        corpus.version, SOLVER_TIERS_CORPUS_VERSION,
        "unsupported corpus version {} (expected {})",
        corpus.version, SOLVER_TIERS_CORPUS_VERSION
    );

    corpus
}

struct ScenarioSpec {
    name: String,
    size: (u8, u8),
    layouts: Vec<MineLayout>,
    sample_size: usize,
}

fn scenario_from_corpus(entry: SolverTiersScenario) -> ScenarioSpec {
    let layouts: Vec<MineLayout> = entry
        .layouts
        .iter()
        .map(|stored| {
            stored
                .to_layout(entry.size)
                .unwrap_or_else(|err| panic!("invalid layout in scenario `{}`: {err}", entry.name))
        })
        .collect();

    ScenarioSpec {
        name: entry.name,
        size: entry.size,
        layouts,
        sample_size: 10,
    }
}

fn load_scenarios() -> Vec<ScenarioSpec> {
    let path = corpus_path();
    let corpus = load_corpus(&path);

    let mut scenarios = Vec::new();
    for scenario in corpus.scenarios {
        if scenario.layouts.is_empty() {
            continue;
        }
        scenarios.push(scenario_from_corpus(scenario));
    }

    assert!(
        !scenarios.is_empty(),
        "corpus has no scenarios with layouts"
    );
    scenarios
}

fn center(size: (u8, u8)) -> (u8, u8) {
    (size.0 / 2, size.1 / 2)
}

fn solve_game_to_completion(layout: &MineLayout) {
    let cfg = SolverConfig::default();
    let mut engine = PlayEngine::new(layout.clone());
    let first = center(engine.size());

    engine
        .reveal(first)
        .expect("opening reveal should not fail");

    let step_budget = usize::from(layout.total_cells()).saturating_mul(8);

    // One StatefulSolver per game — preserves CDCL learned clauses across steps.
    let mut solver = StatefulSolver::new(cfg);

    for _ in 0..step_budget {
        match engine.state() {
            EngineState::Won => return,
            EngineState::Lost => panic!("solver lost a game"),
            EngineState::Ready | EngineState::Active => {}
        }

        let obs = Observation::from_engine(&engine);
        let out = solver
            .solve(&obs, SolveDepth::UntilGuessConclusion)
            .expect("solver should not fail");

        if out.forced_safe.is_empty() && out.forced_mines.is_empty() {
            return; // Guess required — stop here
        }

        for &coords in &out.forced_mines {
            if matches!(engine.cell_at(coords), EngineCell::Hidden) {
                engine.toggle_flag(coords).expect("flag should succeed");
            }
        }

        for &coords in &out.forced_safe {
            if matches!(engine.cell_at(coords), EngineCell::Hidden) {
                let result = engine.reveal(coords).expect("reveal should succeed");
                assert!(
                    !matches!(result, RevealOutcome::HitMine),
                    "solver marked mine as safe"
                );
            }
        }
    }
}

fn bench_solver_tiers(c: &mut Criterion) {
    for spec in load_scenarios() {
        let mut group = c.benchmark_group(format!("solver_tiers/{}", spec.name));
        group.sample_size(spec.sample_size);
        group.measurement_time(Duration::from_secs(6));
        group.throughput(Throughput::Elements(
            (u64::from(spec.size.0) * u64::from(spec.size.1))
                .saturating_mul(spec.layouts.len() as u64),
        ));

        group.bench_with_input(
            BenchmarkId::new("stateful_sat", "default"),
            &spec.layouts,
            |b, layouts| {
                b.iter(|| {
                    for layout in layouts {
                        solve_game_to_completion(black_box(layout));
                    }
                });
            },
        );

        group.finish();
    }
}

criterion_group!(solver_benches, bench_solver_tiers);
criterion_main!(solver_benches);
