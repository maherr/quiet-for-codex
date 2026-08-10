//! Fixed-cardinality counters for the quiet retained-render pipeline.
//!
//! These counters deliberately have no dynamic labels, paths, commands, or session identifiers.
//! They make render-cost regressions observable without retaining user content or allowing metric
//! cardinality to grow with conversation length.

#[cfg(any(debug_assertions, test, feature = "quiet-bench"))]
use std::sync::atomic::AtomicU64;
#[cfg(any(debug_assertions, test, feature = "quiet-bench"))]
use std::sync::atomic::Ordering;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(usize)]
pub(crate) enum QuietMetric {
    SourceAppend,
    ToolRunSeal,
    FullProjection,
    RangeReplacement,
    InlineReflow,
    ClassificationCacheHit,
    ClassificationCacheMiss,
    ClassificationCacheInvalidation,
    ClassificationInFlightBypass,
    PreparedCellCacheHit,
    PreparedCellCacheMiss,
    LivePrimaryRebuild,
    LiveTokenRebuild,
    LiveRateLimitRebuild,
    FrameRequestImmediate,
    FrameRequestDelayed,
    FrameEmitted,
    RenderUnder1Ms,
    RenderUnder2Ms,
    RenderUnder4Ms,
    RenderUnder8Ms,
    RenderUnder12Ms,
    RenderUnder16Ms,
    RenderUnder33Ms,
    RenderOver33Ms,
    RenderMaxMicros,
    SourceAppendOpen,
    RangeReplacementSeal,
    RangeReplacementLifecycle,
    RangeReplacementStructural,
    RangeReplacementUserToggle,
    SealAssistantStream,
    SealPlanStream,
    SealReasoningStream,
    SealBarrier,
    SealTurnBoundary,
    SealTurnComplete,
    SealReplayEnd,
    FullProjectionReset,
    // Owned replay commits incrementally while drawing is deferred, so this remains zero unless
    // a future replay path regresses to a full projection replacement.
    #[allow(dead_code)]
    FullProjectionReplayCommit,
    FullProjectionGlobalMode,
    FullProjectionUnexpected,
    InlineReflowSeal,
    InlineReflowStructural,
    ClassificationMemoPopulate,
    ClassificationOutputScan,
    SealClassificationDivergence,
    PreparedCellCacheInvalidation,
    LayoutStableCellMeasurement,
    LayoutVisibleCellDraw,
    LiveTailMaxRows,
    LiveTailHoleObservation,
    BottomHeightOrdinaryChange,
    BottomHeightDetailedChange,
    BottomHeightModalChange,
}

#[cfg(any(debug_assertions, test, feature = "quiet-bench"))]
const METRIC_COUNT: usize = QuietMetric::BottomHeightModalChange as usize + 1;

// Production release builds have no metrics reader, so their instrumentation calls compile to
// no-ops. Debug builds and the explicit benchmark feature retain the fixed-cardinality counters.
#[cfg(any(debug_assertions, test, feature = "quiet-bench"))]
static COUNTERS: [AtomicU64; METRIC_COUNT] = [const { AtomicU64::new(0) }; METRIC_COUNT];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ToolRunSealReason {
    AssistantStream,
    PlanStream,
    ReasoningStream,
    Barrier,
    TurnBoundary,
    TurnComplete,
    ReplayEnd,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RangeReplacementReason {
    Seal,
    Lifecycle,
    Structural,
    UserToggle,
}

#[cfg(test)]
std::thread_local! {
    static TEST_COUNTERS: std::cell::Cell<[u64; METRIC_COUNT]> = const {
        std::cell::Cell::new([0; METRIC_COUNT])
    };
}

pub(crate) fn bump(metric: QuietMetric) {
    #[cfg(any(debug_assertions, test, feature = "quiet-bench"))]
    {
        let index = metric as usize;
        COUNTERS[index].fetch_add(1, Ordering::Relaxed);
    }
    #[cfg(not(any(debug_assertions, test, feature = "quiet-bench")))]
    let _ = metric;
    #[cfg(test)]
    TEST_COUNTERS.with(|counters| {
        let mut next = counters.get();
        let test_index = metric as usize;
        next[test_index] = next[test_index].saturating_add(1);
        counters.set(next);
    });
}

pub(crate) fn record_max(metric: QuietMetric, value: u64) {
    #[cfg(any(debug_assertions, test, feature = "quiet-bench"))]
    {
        let index = metric as usize;
        COUNTERS[index].fetch_max(value, Ordering::Relaxed);
    }
    #[cfg(not(any(debug_assertions, test, feature = "quiet-bench")))]
    let _ = (metric, value);
    #[cfg(test)]
    TEST_COUNTERS.with(|counters| {
        let mut next = counters.get();
        let test_index = metric as usize;
        next[test_index] = next[test_index].max(value);
        counters.set(next);
    });
}

pub(crate) fn bump_seal(reason: ToolRunSealReason) {
    bump(QuietMetric::ToolRunSeal);
    bump(match reason {
        ToolRunSealReason::AssistantStream => QuietMetric::SealAssistantStream,
        ToolRunSealReason::PlanStream => QuietMetric::SealPlanStream,
        ToolRunSealReason::ReasoningStream => QuietMetric::SealReasoningStream,
        ToolRunSealReason::Barrier => QuietMetric::SealBarrier,
        ToolRunSealReason::TurnBoundary => QuietMetric::SealTurnBoundary,
        ToolRunSealReason::TurnComplete => QuietMetric::SealTurnComplete,
        ToolRunSealReason::ReplayEnd => QuietMetric::SealReplayEnd,
    });
}

pub(crate) fn bump_range_replacement(reason: RangeReplacementReason) {
    bump(match reason {
        RangeReplacementReason::Seal => QuietMetric::RangeReplacementSeal,
        RangeReplacementReason::Lifecycle => QuietMetric::RangeReplacementLifecycle,
        RangeReplacementReason::Structural => QuietMetric::RangeReplacementStructural,
        RangeReplacementReason::UserToggle => QuietMetric::RangeReplacementUserToggle,
    });
}

#[cfg(any(debug_assertions, test, feature = "quiet-bench"))]
pub(crate) fn record_render_time(elapsed: std::time::Duration) {
    let micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
    record_max(QuietMetric::RenderMaxMicros, micros);
    bump(if elapsed < std::time::Duration::from_millis(1) {
        QuietMetric::RenderUnder1Ms
    } else if elapsed < std::time::Duration::from_millis(2) {
        QuietMetric::RenderUnder2Ms
    } else if elapsed < std::time::Duration::from_millis(4) {
        QuietMetric::RenderUnder4Ms
    } else if elapsed < std::time::Duration::from_millis(8) {
        QuietMetric::RenderUnder8Ms
    } else if elapsed < std::time::Duration::from_millis(12) {
        QuietMetric::RenderUnder12Ms
    } else if elapsed < std::time::Duration::from_millis(16) {
        QuietMetric::RenderUnder16Ms
    } else if elapsed < std::time::Duration::from_millis(33) {
        QuietMetric::RenderUnder33Ms
    } else {
        QuietMetric::RenderOver33Ms
    });
}

#[cfg(test)]
pub(crate) fn reset_for_test() {
    TEST_COUNTERS.with(|counters| counters.set([0; METRIC_COUNT]));
}

#[cfg(test)]
pub(crate) fn get_for_test(metric: QuietMetric) -> u64 {
    TEST_COUNTERS.with(|counters| counters.get()[metric as usize])
}

#[cfg(feature = "quiet-bench")]
pub(crate) fn get_for_bench(metric: QuietMetric) -> u64 {
    COUNTERS[metric as usize].load(Ordering::Relaxed)
}

#[cfg(test)]
#[path = "quiet_metrics_tests.rs"]
mod tests;
