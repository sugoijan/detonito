use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use detonito_core::{
    EngineCell, EngineState, FlagSemantics, GameConfig, MineLayout, Observation, PlayEngine,
    RevealOutcome, SOLVER_TIERS_CORPUS_VERSION, SolveDepth, SolverConfig, SolverTiersCorpus,
    SolverTiersScenario, StatefulSolver, parse_solver_tiers_corpus_json,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let corpus_path = args
        .get(1)
        .map(String::as_str)
        .unwrap_or("solver_tiers_corpus.json");

    let dump_dir: Option<PathBuf> = args.get(2).map(PathBuf::from);

    let input = fs::read_to_string(corpus_path)
        .unwrap_or_else(|err| panic!("failed to read corpus at `{corpus_path}`: {err}"));

    let corpus: SolverTiersCorpus = parse_solver_tiers_corpus_json(&input)
        .unwrap_or_else(|err| panic!("failed to parse corpus: {err}"));

    assert_eq!(
        corpus.version, SOLVER_TIERS_CORPUS_VERSION,
        "unsupported corpus version {} (expected {})",
        corpus.version, SOLVER_TIERS_CORPUS_VERSION
    );

    let cfg = SolverConfig::default();

    if let Some(dir) = &dump_dir {
        fs::create_dir_all(dir)
            .unwrap_or_else(|err| panic!("failed to create dump dir `{}`: {err}", dir.display()));
        println!(
            "Snapshots for GuessRequired positions will be written to: {}",
            dir.display()
        );
    }

    let total_start = Instant::now();
    let mut grand_total_games: u64 = 0;
    let mut grand_total_solver_calls: u64 = 0;
    let mut grand_total_sat_calls: u64 = 0;
    let mut grand_total_micros: u128 = 0;

    for scenario in &corpus.scenarios {
        if scenario.layouts.is_empty() {
            continue;
        }
        run_scenario(scenario, &cfg, dump_dir.as_deref(), &mut |counters| {
            grand_total_games += counters.games;
            grand_total_solver_calls += counters.solver_calls;
            grand_total_sat_calls += counters.sat_calls;
            grand_total_micros += counters.solve_micros;
        });
    }

    let total_ms = total_start.elapsed().as_millis();
    println!();
    println!(
        "TOTAL: {grand_total_games} games, {grand_total_solver_calls} solver calls, \
         {grand_total_sat_calls} SAT calls, solve_micros={grand_total_micros}, wall_ms={total_ms}"
    );
}

#[derive(Default)]
struct ScenarioCounters {
    games: u64,
    solver_calls: u64,
    sat_calls: u64,
    solve_micros: u128,
    guess_required: u64,
    p1_safe: u64,
    p1_mines: u64,
    p2_sat_calls: u64,
    p2_safe: u64,
    p2_mines: u64,
    p3_sat_calls: u64,
    cache_hits: u64,
    components_checked: u64,
}

fn run_scenario(
    scenario: &SolverTiersScenario,
    cfg: &SolverConfig,
    dump_dir: Option<&Path>,
    report: &mut dyn FnMut(&ScenarioCounters),
) {
    let mut counters = ScenarioCounters::default();

    for (layout_idx, stored) in scenario.layouts.iter().enumerate() {
        let layout = stored
            .to_layout(scenario.size)
            .expect("invalid layout in corpus");
        play_game(
            &layout,
            &scenario.name,
            layout_idx,
            cfg,
            dump_dir,
            &mut counters,
        );
    }

    let avg_ms = if counters.games == 0 {
        0.0
    } else {
        counters.solve_micros as f64 / counters.games as f64 / 1000.0
    };
    let cache_rate = if counters.components_checked == 0 {
        0.0
    } else {
        counters.cache_hits as f64 / counters.components_checked as f64 * 100.0
    };

    println!(
        "{}: {} games, {} solver calls, {} SAT calls (p2={} p3={}), \
         p1_safe={} p1_mines={} p2_safe={} p2_mines={}, \
         cache_hits={:.1}% ({}/{}), avg_solve={:.2}ms, guesses={}",
        scenario.name,
        counters.games,
        counters.solver_calls,
        counters.sat_calls,
        counters.p2_sat_calls,
        counters.p3_sat_calls,
        counters.p1_safe,
        counters.p1_mines,
        counters.p2_safe,
        counters.p2_mines,
        cache_rate,
        counters.cache_hits,
        counters.components_checked,
        avg_ms,
        counters.guess_required,
    );

    report(&counters);
}

fn center(size: (u8, u8)) -> (u8, u8) {
    (size.0 / 2, size.1 / 2)
}

/// Dump a TUI-compatible snapshot to `dir/<scenario>_layout<N>_step<M>.json`.
fn dump_guess_snapshot(
    engine: &PlayEngine,
    scenario_name: &str,
    layout_idx: usize,
    step: usize,
    dump_dir: &Path,
) {
    let config = GameConfig {
        size: engine.size(),
        mines: engine.total_mines(),
    };
    let cursor = center(engine.size());

    let snapshot_json = serde_json::json!({
        "version": 1u32,
        "config": config,
        "seed": 0u64,
        "first_move_policy": "FirstMoveZero",
        "cursor": cursor,
        "engine": engine,
        "flag_semantics": FlagSemantics::Strict,
    });

    let name = format!(
        "{}_layout{}_step{}.json",
        scenario_name.replace(' ', "_"),
        layout_idx,
        step,
    );
    let path = dump_dir.join(&name);

    match serde_json::to_string_pretty(&snapshot_json) {
        Ok(json) => match fs::write(&path, json) {
            Ok(()) => println!("  snapshot -> {}", path.display()),
            Err(err) => eprintln!(
                "WARNING: failed to write snapshot to {}: {err}",
                path.display()
            ),
        },
        Err(err) => eprintln!("WARNING: failed to serialize snapshot: {err}"),
    }
}

fn play_game(
    layout: &MineLayout,
    scenario_name: &str,
    layout_idx: usize,
    cfg: &SolverConfig,
    dump_dir: Option<&Path>,
    counters: &mut ScenarioCounters,
) {
    counters.games += 1;
    let mut engine = PlayEngine::new(layout.clone());
    let first = center(engine.size());

    let outcome = engine
        .reveal(first)
        .expect("opening reveal should not fail");
    if matches!(outcome, RevealOutcome::HitMine) {
        // This layout isn't safe for a center first-click; skip it.
        return;
    }

    let step_budget = usize::from(layout.total_cells()).saturating_mul(8);
    let mut step = 0usize;

    // One StatefulSolver per game — solver state is preserved across steps.
    let mut solver = StatefulSolver::new(cfg.clone());

    for _ in 0..step_budget {
        match engine.state() {
            EngineState::Won => return,
            EngineState::Lost => {
                eprintln!("WARNING: solver caused a loss on a game");
                return;
            }
            EngineState::Ready | EngineState::Active => {}
        }

        let obs = Observation::from_engine(&engine);
        let t0 = Instant::now();
        let out = solver
            .solve(&obs, SolveDepth::UntilGuessConclusion)
            .expect("solver should not fail on a valid board");
        counters.solve_micros += t0.elapsed().as_micros();
        counters.solver_calls += 1;
        counters.sat_calls += out.stats.sat_calls;
        counters.p1_safe += out.stats.phase1_forced_safe as u64;
        counters.p1_mines += out.stats.phase1_forced_mines as u64;
        counters.p2_sat_calls += out.stats.phase2_sat_calls;
        counters.p2_safe += out.stats.phase2_forced_safe as u64;
        counters.p2_mines += out.stats.phase2_forced_mines as u64;
        counters.p3_sat_calls += out.stats.phase3_sat_calls;
        counters.cache_hits += out.stats.components_cache_hits as u64;
        counters.components_checked += out.stats.components_checked as u64;
        step += 1;

        if matches!(out.guess_status, detonito_core::GuessStatus::Contradiction) {
            eprintln!("WARNING: solver returned Contradiction");
            return;
        }

        if out.forced_safe.is_empty() && out.forced_mines.is_empty() {
            counters.guess_required += 1;
            if let Some(dir) = dump_dir {
                dump_guess_snapshot(&engine, scenario_name, layout_idx, step, dir);
            }
            return; // Can't proceed without guessing
        }

        let mut progressed = false;

        // Apply forced flags first to reduce future frontier.
        for &coords in &out.forced_mines {
            if matches!(engine.cell_at(coords), EngineCell::Hidden) {
                engine.toggle_flag(coords).expect("flag should succeed");
                progressed = true;
            }
        }

        // Apply forced reveals.
        for &coords in &out.forced_safe {
            match engine.cell_at(coords) {
                EngineCell::Hidden => {
                    let result = engine.reveal(coords).expect("reveal should succeed");
                    if matches!(result, RevealOutcome::HitMine) {
                        eprintln!("SOLVER BUG: revealed a mine marked as safe at {coords:?}");
                    }
                    progressed = true;
                }
                EngineCell::Flagged | EngineCell::Revealed(_) => {}
            }
        }

        if !progressed {
            // The solver found "forced" moves but all were already applied — give up.
            counters.guess_required += 1;
            if let Some(dir) = dump_dir {
                dump_guess_snapshot(&engine, scenario_name, layout_idx, step, dir);
            }
            return;
        }
    }

    eprintln!("WARNING: exceeded step budget for game");
}
