use super::*;

#[test]
fn counter_positive_control_detects_one_increment() {
    reset_for_test();
    assert_eq!(get_for_test(QuietMetric::RangeReplacement), 0);
    bump(QuietMetric::RangeReplacement);
    assert_eq!(get_for_test(QuietMetric::RangeReplacement), 1);
}

#[test]
fn max_and_dimension_positive_controls_turn_red() {
    reset_for_test();
    record_max(QuietMetric::LiveTailMaxRows, 3);
    record_max(QuietMetric::LiveTailMaxRows, 2);
    bump_seal(ToolRunSealReason::ReasoningStream);
    bump_range_replacement(RangeReplacementReason::Structural);

    assert_eq!(get_for_test(QuietMetric::LiveTailMaxRows), 3);
    assert_eq!(get_for_test(QuietMetric::ToolRunSeal), 1);
    assert_eq!(get_for_test(QuietMetric::SealReasoningStream), 1);
    assert_eq!(get_for_test(QuietMetric::RangeReplacementStructural), 1);
}
