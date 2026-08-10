use super::*;
use std::collections::HashSet;

use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;

#[test]
fn open_run_stays_unsealed_until_a_semantic_boundary() {
    let mut lifecycle = ToolRunLifecycle::default();
    lifecycle.begin_turn("turn-1".to_string());

    assert!(
        lifecycle
            .before_classified_source_append(SourceClass::Action(1))
            .is_none()
    );
    assert!(
        lifecycle
            .before_classified_source_append(SourceClass::Action(1))
            .is_none()
    );

    let seal = lifecycle
        .seal_turn("turn-1")
        .expect("turn completion should seal the open run");
    assert_eq!(seal.run_id(), RunId(1));
    assert_eq!(seal.action_count(), 2);
    assert_eq!(seal.source_range(), (SourceCellId(0), SourceCellId(1)));
}

#[test]
fn visible_barrier_seals_before_receiving_its_source_id() {
    let mut lifecycle = ToolRunLifecycle::default();
    lifecycle.begin_turn("turn-1".to_string());
    lifecycle.before_classified_source_append(SourceClass::Action(2));
    lifecycle.before_classified_source_append(SourceClass::Transparent);

    let seal = lifecycle
        .before_classified_source_append(SourceClass::Barrier)
        .expect("barrier should seal before it is appended");
    assert_eq!(seal.action_count(), 2);
    assert_eq!(seal.source_range(), (SourceCellId(0), SourceCellId(1)));

    lifecycle.before_classified_source_append(SourceClass::Action(1));
    let next = lifecycle.seal_open_run().expect("second run should seal");
    assert_eq!(next.run_id(), RunId(2));
    assert_eq!(next.source_range(), (SourceCellId(3), SourceCellId(3)));
}

#[test]
fn transparent_cell_without_an_open_run_does_not_create_one() {
    let mut lifecycle = ToolRunLifecycle::default();

    lifecycle.before_classified_source_append(SourceClass::Transparent);

    assert!(lifecycle.seal_open_run().is_none());
}

#[test]
fn late_turn_seal_cannot_close_a_newer_run() {
    let mut lifecycle = ToolRunLifecycle::default();
    lifecycle.begin_turn("turn-a".to_string());
    lifecycle.before_classified_source_append(SourceClass::Action(1));
    let prior = lifecycle
        .begin_turn("turn-b".to_string())
        .expect("new turn should seal stale prior work");
    assert_eq!(prior.run_id(), RunId(1));
    lifecycle.before_classified_source_append(SourceClass::Action(2));

    assert!(lifecycle.seal_turn("turn-a").is_none());
    let current = lifecycle
        .seal_turn("turn-b")
        .expect("matching turn should seal its run");
    assert_eq!(current.run_id(), RunId(2));
    assert_eq!(current.action_count(), 2);
}

#[test]
fn reset_discards_an_open_run_without_reusing_its_run_id() {
    let mut lifecycle = ToolRunLifecycle::default();
    lifecycle.begin_turn("turn-before-clear".to_string());
    lifecycle.before_classified_source_append(SourceClass::Action(1));

    lifecycle.reset_open_run();
    assert!(lifecycle.seal_open_run().is_none());

    lifecycle.begin_turn("turn-after-clear".to_string());
    lifecycle.before_classified_source_append(SourceClass::Action(2));
    let seal = lifecycle
        .seal_open_run()
        .expect("post-reset work should form a fresh run");
    assert_eq!(seal.run_id(), RunId(2));
    assert_eq!(seal.source_range(), (SourceCellId(1), SourceCellId(1)));
}

#[derive(Clone, Debug)]
struct ModelRun {
    run_id: u64,
    first_source: u64,
    last_source: u64,
    actions: usize,
    turn: Option<String>,
}

fn assert_seal_matches(actual: Option<ToolRunSealCell>, expected: Option<ModelRun>) {
    match (actual, expected) {
        (None, None) => {}
        (Some(actual), Some(expected)) => {
            assert_eq!(actual.run_id(), RunId(expected.run_id));
            assert_eq!(actual.action_count(), expected.actions);
            assert_eq!(
                actual.source_range(),
                (
                    SourceCellId(expected.first_source),
                    SourceCellId(expected.last_source)
                )
            );
        }
        (actual, expected) => panic!("seal mismatch: actual={actual:?}, expected={expected:?}"),
    }
}

#[test]
fn generated_lifecycle_sequences_preserve_ids_order_turns_and_idempotence() {
    for seed in 0_u64..128 {
        let mut rng = StdRng::seed_from_u64(seed);
        let mut lifecycle = ToolRunLifecycle::default();
        let mut next_source = 0_u64;
        let mut next_run = 1_u64;
        let mut current_turn: Option<String> = None;
        let mut open: Option<ModelRun> = None;
        let mut sealed_runs = HashSet::new();

        for _ in 0..512 {
            let (actual, expected) = match rng.random_range(0_u8..8) {
                0 => {
                    let turn = format!("turn-{}", rng.random_range(0_u8..4));
                    let expected = if current_turn.as_deref() == Some(turn.as_str()) {
                        None
                    } else {
                        current_turn = Some(turn.clone());
                        open.take()
                    };
                    (lifecycle.begin_turn(turn), expected)
                }
                1 | 2 => {
                    let actions = rng.random_range(1_usize..=3);
                    let source = next_source;
                    next_source = next_source.checked_add(1).expect("model source id");
                    let run = open.get_or_insert_with(|| {
                        let run = ModelRun {
                            run_id: next_run,
                            first_source: source,
                            last_source: source,
                            actions: 0,
                            turn: current_turn.clone(),
                        };
                        next_run = next_run.checked_add(1).expect("model run id");
                        run
                    });
                    run.last_source = source;
                    run.actions = run.actions.saturating_add(actions);
                    (
                        lifecycle.before_classified_source_append(SourceClass::Action(actions)),
                        None,
                    )
                }
                3 => {
                    let source = next_source;
                    next_source = next_source.checked_add(1).expect("model source id");
                    if let Some(run) = open.as_mut() {
                        run.last_source = source;
                    }
                    (
                        lifecycle.before_classified_source_append(SourceClass::Transparent),
                        None,
                    )
                }
                4 => {
                    next_source = next_source.checked_add(1).expect("model source id");
                    (
                        lifecycle.before_classified_source_append(SourceClass::Barrier),
                        open.take(),
                    )
                }
                5 => {
                    let turn = format!("turn-{}", rng.random_range(0_u8..4));
                    let expected = if open.as_ref().and_then(|run| run.turn.as_deref())
                        == Some(turn.as_str())
                    {
                        open.take()
                    } else {
                        None
                    };
                    (lifecycle.seal_turn(&turn), expected)
                }
                6 => (lifecycle.seal_open_run(), open.take()),
                _ => {
                    lifecycle.reset_open_run();
                    current_turn = None;
                    open = None;
                    (None, None)
                }
            };

            if let Some(seal) = actual.as_ref() {
                assert!(
                    sealed_runs.insert(seal.run_id()),
                    "seed {seed}: duplicate RunId {:?}",
                    seal.run_id()
                );
            }
            assert_seal_matches(actual, expected);
        }

        assert_seal_matches(lifecycle.seal_open_run(), open.take());
        assert!(lifecycle.seal_open_run().is_none(), "seed {seed}");
        assert_eq!(lifecycle.next_source_id, next_source, "seed {seed}");
        assert_eq!(lifecycle.next_run_id, next_run, "seed {seed}");
    }
}
