// RPad: ceiling-rounded render interval for the current display, capped at 180 Hz.
pub(crate) fn render_interval(refresh_millihertz: Option<u32>) -> std::time::Duration {
    let refresh = refresh_millihertz.filter(|rate| *rate > 0).unwrap_or(60_000).min(180_000) as u64;

    std::time::Duration::from_nanos(1_000_000_000_000u64.div_ceil(refresh))
}
