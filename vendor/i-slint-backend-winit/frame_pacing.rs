// RPad: ceiling-rounded render interval, limited to the display refresh rate.
// The environment override is read once, not on every redraw.
fn max_refresh_millihertz() -> u32 {
    static LIMIT: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(|| {
        let value = std::env::var("RPAD_MAX_FPS").unwrap_or_default();
        if value.eq_ignore_ascii_case("monitor") {
            return u32::MAX;
        }
        value
            .parse::<u32>()
            .ok()
            .filter(|fps| (30..=1000).contains(fps))
            .unwrap_or(180)
            * 1000
    })
}

pub(crate) fn render_interval(refresh_millihertz: Option<u32>) -> std::time::Duration {
    render_interval_with_limit(refresh_millihertz, max_refresh_millihertz())
}

pub(crate) fn render_interval_with_limit(
    refresh_millihertz: Option<u32>,
    limit: u32,
) -> std::time::Duration {
    let refresh = refresh_millihertz
        .filter(|rate| *rate > 0)
        .unwrap_or(60_000)
        .min(limit.max(1)) as u64;

    std::time::Duration::from_nanos(1_000_000_000_000u64.div_ceil(refresh))
}
