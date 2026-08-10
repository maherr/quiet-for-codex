use codex_tui::quiet_bench::FooterOnlyFrameFixture;
use codex_tui::quiet_bench::OwnedFrameFixture;
use divan::Bencher;
use std::time::Instant;

fn main() {
    if std::env::var_os("CODEX_QUIET_BENCH_REPORT").is_some() {
        let samples = std::env::var("CODEX_QUIET_BENCH_SAMPLES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(2_000)
            .max(100);
        println!("quiet_render latency_report samples={samples}");
        println!(
            "machine os={} arch={} cpus={}",
            std::env::consts::OS,
            std::env::consts::ARCH,
            std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get),
        );
        let owned_80 = report_owned_latency("owned_frame_80x30", 80, 30, samples);
        let control_80 =
            report_footer_control_latency("footer_only_control_80x30", 80, 30, samples);
        report_owned_control_delta("owned_over_footer_only_control_80x30", owned_80, control_80);
        let owned_160 = report_owned_latency("owned_frame_160x50", 160, 50, samples);
        let control_160 =
            report_footer_control_latency("footer_only_control_160x50", 160, 50, samples);
        report_owned_control_delta(
            "owned_over_footer_only_control_160x50",
            owned_160,
            control_160,
        );
        report_input_latency("running_status_input_to_draw_80x30", 80, 30, samples);
        report_cache_ratio(/*width*/ 80, /*height*/ 30, /*frames*/ 100);
        return;
    }
    divan::main();
}

fn report_input_latency(name: &str, width: u16, height: u16, samples: usize) {
    let mut fixture = OwnedFrameFixture::new(width, height);
    for _ in 0..100 {
        std::hint::black_box(fixture.handle_input_and_render());
    }
    let mut timings = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = Instant::now();
        std::hint::black_box(fixture.handle_input_and_render());
        timings.push(started.elapsed().as_nanos());
    }
    print_latency(name, &mut timings);
}

fn report_cache_ratio(width: u16, height: u16, frames: usize) {
    let mut fixture = OwnedFrameFixture::new(width, height);
    let (hits_before, misses_before) = fixture.prepared_cache_counts();
    for _ in 0..frames {
        std::hint::black_box(fixture.render_status_frame());
    }
    let (hits_after, misses_after) = fixture.prepared_cache_counts();
    let hits = hits_after.saturating_sub(hits_before);
    let misses = misses_after.saturating_sub(misses_before);
    let total = hits.saturating_add(misses);
    let percent = if total == 0 {
        0.0
    } else {
        hits as f64 * 100.0 / total as f64
    };
    println!(
        "prepared_line_cache_80x30 frames={frames} hits={hits} misses={misses} hit_percent={percent:.3}"
    );
}

fn report_owned_latency(name: &str, width: u16, height: u16, samples: usize) -> LatencySummary {
    let mut fixture = OwnedFrameFixture::new(width, height);
    for _ in 0..100 {
        std::hint::black_box(fixture.render_status_frame());
    }
    let mut timings = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = Instant::now();
        std::hint::black_box(fixture.render_status_frame());
        timings.push(started.elapsed().as_nanos());
    }
    print_latency(name, &mut timings)
}

fn report_footer_control_latency(
    name: &str,
    width: u16,
    height: u16,
    samples: usize,
) -> LatencySummary {
    let mut fixture = FooterOnlyFrameFixture::new(width, height);
    for _ in 0..100 {
        std::hint::black_box(fixture.render_status_frame());
    }
    let mut timings = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = Instant::now();
        std::hint::black_box(fixture.render_status_frame());
        timings.push(started.elapsed().as_nanos());
    }
    print_latency(name, &mut timings)
}

#[derive(Clone, Copy)]
struct LatencySummary {
    p95_us: f64,
}

fn print_latency(name: &str, timings: &mut [u128]) -> LatencySummary {
    timings.sort_unstable();
    let percentile = |percent: usize| {
        let index = timings.len().saturating_sub(1).saturating_mul(percent) / 100;
        timings[index] as f64 / 1_000.0
    };
    let max = timings.last().copied().unwrap_or_default() as f64 / 1_000.0;
    let p95_us = percentile(95);
    println!(
        "{name} p50_us={:.3} p95_us={:.3} max_us={max:.3}",
        percentile(50),
        p95_us,
    );
    LatencySummary { p95_us }
}

fn report_owned_control_delta(name: &str, owned: LatencySummary, control: LatencySummary) {
    let delta_us = owned.p95_us - control.p95_us;
    let ratio = if control.p95_us == 0.0 {
        f64::INFINITY
    } else {
        owned.p95_us / control.p95_us
    };
    println!(
        "{name} p95_delta_us={delta_us:.3} p95_ratio={ratio:.3} diagnostic_threshold_exceeded={}",
        delta_us >= 2_000.0 && ratio > 1.25,
    );
}

#[divan::bench]
fn owned_frame_80x30(bencher: Bencher) {
    let mut fixture = OwnedFrameFixture::new(/*width*/ 80, /*height*/ 30);
    bencher.bench_local(|| divan::black_box(fixture.render_status_frame()));
}

#[divan::bench]
fn owned_frame_160x50(bencher: Bencher) {
    let mut fixture = OwnedFrameFixture::new(/*width*/ 160, /*height*/ 50);
    bencher.bench_local(|| divan::black_box(fixture.render_status_frame()));
}

#[divan::bench]
fn footer_only_control_80x30(bencher: Bencher) {
    let mut fixture = FooterOnlyFrameFixture::new(/*width*/ 80, /*height*/ 30);
    bencher.bench_local(|| divan::black_box(fixture.render_status_frame()));
}

#[divan::bench]
fn footer_only_control_160x50(bencher: Bencher) {
    let mut fixture = FooterOnlyFrameFixture::new(/*width*/ 160, /*height*/ 50);
    bencher.bench_local(|| divan::black_box(fixture.render_status_frame()));
}

#[divan::bench]
fn running_status_input_to_draw_80x30(bencher: Bencher) {
    let mut fixture = OwnedFrameFixture::new(/*width*/ 80, /*height*/ 30);
    bencher.bench_local(|| divan::black_box(fixture.handle_input_and_render()));
}
