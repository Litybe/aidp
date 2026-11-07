//! Minimal 5-field cron (`min hour dom month dow`), Vixie semantics for dom/dow.
//! Supports `*`, `n`, `a-b`, `*/s`, `a-b/s` and comma lists. A leading 6th
//! (seconds) field is tolerated and ignored. Local time.

use chrono::{DateTime, Datelike, Duration, Local, Timelike};

#[derive(Clone, Debug)]
pub struct Schedule {
    minute: [bool; 60],
    hour: [bool; 24],
    dom: [bool; 32],
    month: [bool; 13],
    dow: [bool; 7],
    dom_star: bool,
    dow_star: bool,
}

fn parse_field(spec: &str, min: u32, max: u32, out: &mut [bool]) -> Result<bool, String> {
    let mut star = false;
    for item in spec.split(',') {
        let (range, step) = match item.split_once('/') {
            Some((r, s)) => (
                r,
                s.parse::<u32>()
                    .map_err(|_| format!("bad step in {item:?}"))?,
            ),
            None => (item, 1),
        };
        if step == 0 {
            return Err(format!("step 0 in {item:?}"));
        }
        let (lo, hi) = if range == "*" {
            star = true;
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            (
                a.parse().map_err(|_| format!("bad range {range:?}"))?,
                b.parse().map_err(|_| format!("bad range {range:?}"))?,
            )
        } else {
            let v: u32 = range.parse().map_err(|_| format!("bad value {range:?}"))?;
            // a single value with a step means "from v to max"
            if step > 1 {
                (v, max)
            } else {
                (v, v)
            }
        };
        if lo < min || hi > max || lo > hi {
            return Err(format!("{range:?} outside {min}-{max}"));
        }
        let mut v = lo;
        while v <= hi {
            out[v as usize] = true;
            v += step;
        }
    }
    Ok(star)
}

pub fn parse(expr: &str) -> Result<Schedule, String> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    let fields: &[&str] = if fields.len() == 6 {
        &fields[1..]
    } else {
        &fields[..]
    };
    if fields.len() != 5 {
        return Err(format!(
            "expected 5 fields (min hour dom month dow), got {}",
            fields.len()
        ));
    }
    let mut s = Schedule {
        minute: [false; 60],
        hour: [false; 24],
        dom: [false; 32],
        month: [false; 13],
        dow: [false; 7],
        dom_star: false,
        dow_star: false,
    };
    parse_field(fields[0], 0, 59, &mut s.minute)?;
    parse_field(fields[1], 0, 23, &mut s.hour)?;
    s.dom_star = parse_field(fields[2], 1, 31, &mut s.dom)?;
    parse_field(fields[3], 1, 12, &mut s.month)?;
    // dow: accept 7 as Sunday
    let dow_spec = fields[4].replace('7', "0");
    let mut dow8 = [false; 8];
    s.dow_star = parse_field(&dow_spec, 0, 6, &mut dow8)?;
    s.dow[..7].copy_from_slice(&dow8[..7]);
    Ok(s)
}

impl Schedule {
    pub fn matches(&self, t: &DateTime<Local>) -> bool {
        if !self.minute[t.minute() as usize]
            || !self.hour[t.hour() as usize]
            || !self.month[t.month() as usize]
        {
            return false;
        }
        let dom = self.dom[t.day() as usize];
        let dow = self.dow[t.weekday().num_days_from_sunday() as usize];
        match (self.dom_star, self.dow_star) {
            (true, true) => true,
            (true, false) => dow,
            (false, true) => dom,
            (false, false) => dom || dow,
        }
    }

    /// First matching minute strictly after `from` (searches up to one year ahead).
    pub fn next_after(&self, from: DateTime<Local>) -> Option<DateTime<Local>> {
        let mut t = from.with_second(0)?.with_nanosecond(0)? + Duration::minutes(1);
        for _ in 0..(366 * 24 * 60) {
            if self.matches(&t) {
                return Some(t);
            }
            t += Duration::minutes(1);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn weekday_morning() {
        let s = parse("0 8 * * 1-5").unwrap();
        let from = Local.with_ymd_and_hms(2026, 9, 4, 9, 0, 0).unwrap(); // Friday 09:00
        let next = s.next_after(from).unwrap();
        assert_eq!(
            (
                next.weekday().num_days_from_sunday(),
                next.hour(),
                next.minute()
            ),
            (1, 8, 0)
        ); // Monday 08:00
    }

    #[test]
    fn steps_and_lists() {
        let s = parse("*/15 9,17 * * *").unwrap();
        let from = Local.with_ymd_and_hms(2026, 9, 4, 9, 20, 0).unwrap();
        let next = s.next_after(from).unwrap();
        assert_eq!((next.hour(), next.minute()), (9, 30));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("60 * * * *").is_err());
        assert!(parse("* * *").is_err());
    }
}
