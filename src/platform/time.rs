//! Snapshot timestamps stay in UTC; Vita RTC supplies the system timezone.
pub fn stamp(seconds: u64) -> String {
    #[cfg(target_os = "vita")]
    if let Some(local) = local_time(seconds) {
        return format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            local.year, local.month, local.day, local.hour, local.minute, local.second
        );
    }
    format_at_offset(seconds, 0)
}

#[cfg(target_os = "vita")]
fn local_time(seconds: u64) -> Option<vitasdk_sys::SceDateTime> {
    use vitasdk_sys::*;
    unsafe {
        let mut utc = std::mem::zeroed::<SceDateTime>();
        let mut utc_tick = std::mem::zeroed::<SceRtcTick>();
        let mut local_tick = std::mem::zeroed::<SceRtcTick>();
        let mut local = std::mem::zeroed::<SceDateTime>();
        if sceRtcSetTime64_t(&mut utc, seconds) < 0
            || sceRtcGetTick(&utc, &mut utc_tick) < 0
            || sceRtcConvertUtcToLocalTime(&utc_tick, &mut local_tick) < 0
            || sceRtcSetTick(&mut local, &local_tick) < 0
        {
            return None;
        }
        Some(local)
    }
}

fn format_at_offset(seconds: u64, offset: i64) -> String {
    let seconds = i128::from(seconds) + i128::from(offset);
    let z = seconds.div_euclid(86400) + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i128::from(month <= 2);
    let within_day = seconds.rem_euclid(86400);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        within_day / 3600,
        within_day / 60 % 60,
        within_day % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timezone_conversion_handles_day_year_and_leap_boundaries() {
        assert_eq!(
            format_at_offset(1704067200, 8 * 3600),
            "2024-01-01 08:00:00"
        );
        assert_eq!(
            format_at_offset(1704067200, -5 * 3600),
            "2023-12-31 19:00:00"
        );
        assert_eq!(format_at_offset(951782400, 19800), "2000-02-29 05:30:00");
        assert_eq!(format_at_offset(0, -3600), "1969-12-31 23:00:00");
    }
}
