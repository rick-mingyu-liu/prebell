//! Theme detection: industry groups whose liquid members move together.

use std::collections::HashMap;

use crate::facts::{round2, Direction, Quote, Theme, ThemeMember, ThemeStat};
use crate::premarket::{company_name, group, group_zh};

pub struct ThemeRules {
    /// Minimum |move| (percent) for a member to qualify.
    pub min_move: f64,
    /// Minimum |median| (percent) of the qualifying members.
    pub min_median: f64,
}

pub const PREMARKET_RULES: ThemeRules = ThemeRules {
    min_move: 2.0,
    min_median: 2.5,
};
pub const RECAP_RULES: ThemeRules = ThemeRules {
    min_move: 3.0,
    min_median: 3.5,
};

pub const THEME_MIN_MEMBERS: usize = 3;
pub const THEME_MAX: usize = 3;
pub const THEME_LIST_MEMBERS: usize = 6;
pub const THEME_STAT_MOVE: f64 = 1.5;
pub const THEME_STATS_MAX: usize = 15;

/// One member of a group after share-class dedupe.
struct Entry<'a> {
    ticker: &'a str,
    company: &'a str,
    pct: f64,
}

/// Company key shared by dual share classes: the name without a trailing
/// "(Class X)". Tickers without a name key on themselves.
fn company_key(ticker: &str) -> &str {
    match company_name(ticker) {
        Some(n) => match n.rfind(" (Class ") {
            Some(i) if n.ends_with(')') => &n[..i],
            _ => n,
        },
        None => ticker,
    }
}

fn median(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

fn sorted(mut v: Vec<f64>) -> Vec<f64> {
    v.sort_by(f64::total_cmp);
    v
}

/// Themes (at most `THEME_MAX`, best first) and breadth stats for the
/// groups with at least two strong movers. One pass over the quotes; every
/// sort is over a single group's few members.
pub fn detect(quotes: &[Quote], rules: &ThemeRules) -> (Vec<Theme>, Vec<ThemeStat>) {
    let mut groups: HashMap<&'static str, Vec<Entry<'_>>> = HashMap::new();
    for qt in quotes {
        let pct = qt.change_pct;
        if !pct.is_finite() {
            continue;
        }
        let Some(g) = group(&qt.symbol) else { continue };
        let ticker = qt.symbol.as_str();
        let company = company_key(ticker);
        let entries = groups.entry(g).or_default();
        match entries.iter_mut().find(|e| e.company == company) {
            // Dual share class: one company counts once, by its larger move.
            Some(e) => {
                if pct.abs() > e.pct.abs() {
                    e.ticker = ticker;
                    e.pct = pct;
                }
            }
            None => entries.push(Entry {
                ticker,
                company,
                pct,
            }),
        }
    }

    let mut ranked: Vec<(f64, usize, &str, Theme)> = Vec::new();
    let mut stats: Vec<ThemeStat> = Vec::new();
    for (g, entries) in &groups {
        let present = entries.len();

        let up_stat = entries.iter().filter(|e| e.pct >= THEME_STAT_MOVE).count();
        let down_stat = entries.iter().filter(|e| e.pct <= -THEME_STAT_MOVE).count();
        if up_stat + down_stat >= 2 {
            let side = |keep: fn(f64) -> bool| {
                let v: Vec<f64> = entries.iter().map(|e| e.pct).filter(|p| keep(*p)).collect();
                (!v.is_empty()).then(|| round2(median(&sorted(v))))
            };
            stats.push(ThemeStat {
                group: (*g).to_string(),
                present,
                up: up_stat,
                down: down_stat,
                median_up: side(|p| p >= THEME_STAT_MOVE),
                median_down: side(|p| p <= -THEME_STAT_MOVE),
            });
        }

        if present < THEME_MIN_MEMBERS {
            continue;
        }
        let up = entries.iter().filter(|e| e.pct >= rules.min_move).count();
        let down = entries.iter().filter(|e| e.pct <= -rules.min_move).count();
        let direction = match (up >= THEME_MIN_MEMBERS, down >= THEME_MIN_MEMBERS) {
            (true, false) => Direction::Up,
            (false, true) => Direction::Down,
            // Neither qualifies, or both sides do: a split group is no theme.
            _ => continue,
        };
        let mut qualifying: Vec<&Entry<'_>> = entries
            .iter()
            .filter(|e| match direction {
                Direction::Up => e.pct >= rules.min_move,
                Direction::Down => e.pct <= -rules.min_move,
            })
            .collect();
        let movers = entries.iter().filter(|e| e.pct.abs() > 0.5).count();
        if qualifying.len() * 2 < movers {
            continue;
        }
        let med = median(&sorted(qualifying.iter().map(|e| e.pct).collect()));
        if med.abs() < rules.min_median {
            continue;
        }
        qualifying.sort_by(|a, b| {
            b.pct
                .abs()
                .total_cmp(&a.pct.abs())
                .then_with(|| a.ticker.cmp(b.ticker))
        });
        let count = qualifying.len();
        let members = qualifying
            .iter()
            .take(THEME_LIST_MEMBERS)
            .map(|e| ThemeMember {
                ticker: e.ticker.to_string(),
                name: company_name(e.ticker).unwrap_or(e.ticker).to_string(),
                change_pct: e.pct,
            })
            .collect();
        ranked.push((
            med.abs() * count as f64,
            count,
            g,
            Theme {
                group: (*g).to_string(),
                group_zh: group_zh(g).to_string(),
                direction,
                members,
                more: count.saturating_sub(THEME_LIST_MEMBERS),
                median_pct: round2(med),
                reasons: Vec::new(),
            },
        ));
    }

    ranked.sort_by(|a, b| {
        b.0.total_cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.cmp(b.2))
    });
    let themes = ranked.into_iter().take(THEME_MAX).map(|r| r.3).collect();

    stats.sort_by(|a, b| {
        b.present
            .cmp(&a.present)
            .then_with(|| a.group.cmp(&b.group))
    });
    stats.truncate(THEME_STATS_MAX);
    (themes, stats)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(sym: &str, pct: f64) -> Quote {
        Quote {
            symbol: sym.into(),
            last: 100.0,
            prev_close: 100.0,
            change_pct: pct,
            volume: Some(1_000_000),
        }
    }

    fn qs(items: &[(&str, f64)]) -> Vec<Quote> {
        items.iter().map(|(s, p)| q(s, *p)).collect()
    }

    #[test]
    fn three_members_same_direction_form_a_theme() {
        let quotes = qs(&[("MU", -3.0), ("SNDK", -4.0), ("WDC", -5.0), ("STX", 0.1)]);
        let (themes, _) = detect(&quotes, &PREMARKET_RULES);
        assert_eq!(themes.len(), 1);
        let t = &themes[0];
        assert_eq!(t.group, "Memory & storage");
        assert_eq!(t.group_zh, group_zh("Memory & storage"));
        assert_eq!(t.direction, Direction::Down);
        assert_eq!(t.median_pct, -4.0);
        assert_eq!(t.more, 0);
        assert!(t.reasons.is_empty());
        let tickers: Vec<_> = t.members.iter().map(|m| m.ticker.as_str()).collect();
        assert_eq!(tickers, ["WDC", "SNDK", "MU"]);
        assert_eq!(t.members[2].name, "Micron Technology");
    }

    #[test]
    fn mixed_directions_do_not() {
        let quotes = qs(&[
            ("MU", 3.0),
            ("SNDK", 4.0),
            ("WDC", 5.0),
            ("STX", -3.0),
            ("NTAP", -4.0),
            ("P", -5.0),
        ]);
        let (themes, _) = detect(&quotes, &PREMARKET_RULES);
        assert!(themes.is_empty(), "{themes:?}");
    }

    #[test]
    fn two_members_do_not() {
        let quotes = qs(&[("MU", 4.0), ("SNDK", 5.0), ("WDC", 0.2)]);
        let (themes, _) = detect(&quotes, &PREMARKET_RULES);
        assert!(themes.is_empty());
    }

    #[test]
    fn half_rule_rejects_a_group_mostly_flat() {
        // Three semiconductors qualify; three more move mildly (beyond 0.5%).
        // 3 of 6 movers is exactly half: a theme.
        let mut items = vec![
            ("NVDA", 4.0),
            ("AMD", 4.0),
            ("AVGO", 4.0),
            ("QCOM", 1.0),
            ("TXN", -1.0),
            ("ADI", 0.8),
            // Flat members do not count as movers.
            ("INTC", 0.2),
            ("ON", -0.5),
        ];
        let (themes, _) = detect(&qs(&items), &PREMARKET_RULES);
        assert_eq!(themes.len(), 1);
        // A seventh mild mover tips 3 of 7 under half.
        items.push(("MRVL", -0.9));
        let (themes, _) = detect(&qs(&items), &PREMARKET_RULES);
        assert!(themes.is_empty(), "{themes:?}");
    }

    #[test]
    fn median_below_threshold_is_not_a_theme() {
        let quotes = qs(&[("MU", 2.0), ("SNDK", 2.2), ("WDC", 2.4)]);
        let (themes, _) = detect(&quotes, &PREMARKET_RULES);
        assert!(themes.is_empty());
        let (themes, _) = detect(
            &quotes,
            &ThemeRules {
                min_move: 2.0,
                min_median: 2.2,
            },
        );
        assert_eq!(themes.len(), 1);
    }

    #[test]
    fn themes_rank_by_median_times_members_and_cap_at_three() {
        let quotes = qs(&[
            // Memory: 3 members, median 6 -> 18
            ("MU", 6.0),
            ("SNDK", 6.0),
            ("WDC", 6.0),
            // Semis: 5 members, median 3 -> 15
            ("NVDA", 3.0),
            ("AMD", 3.0),
            ("AVGO", 3.0),
            ("QCOM", 3.0),
            ("TXN", 3.0),
            // Airlines: 3 members, median 5 -> 15, fewer members than semis
            ("DAL", 5.0),
            ("LUV", 5.0),
            ("UAL", 5.0),
            // Homebuilders: 3 members, median 3 -> 9, ranks fourth
            ("DHI", -3.0),
            ("LEN", -3.0),
            ("PHM", -3.0),
        ]);
        let (themes, _) = detect(&quotes, &PREMARKET_RULES);
        let groups: Vec<_> = themes.iter().map(|t| t.group.as_str()).collect();
        assert_eq!(groups, ["Memory & storage", "Semiconductors", "Airlines"]);
    }

    #[test]
    fn members_listed_by_move_with_more_count() {
        let quotes = qs(&[
            ("APA", 3.0),
            ("COP", 3.1),
            ("CVX", 3.2),
            ("DVN", 3.3),
            ("EOG", 3.4),
            ("EQT", 3.5),
            ("EXE", 3.6),
            ("FANG", 3.7),
            ("OXY", 1.0),
        ]);
        let (themes, _) = detect(&quotes, &PREMARKET_RULES);
        assert_eq!(themes.len(), 1);
        let t = &themes[0];
        assert_eq!(t.members.len(), THEME_LIST_MEMBERS);
        assert_eq!(t.more, 2);
        assert_eq!(t.members[0].ticker, "FANG");
        assert_eq!(t.members[5].ticker, "CVX");
        assert_eq!(t.direction, Direction::Up);
        assert_eq!(t.median_pct, 3.35);
    }

    #[test]
    fn stats_cover_groups_with_two_strong_movers_capped_at_fifteen() {
        // Two strong movers in a group: appears in stats, not a theme.
        let quotes = qs(&[("DAL", 1.6), ("LUV", -2.0), ("UAL", 0.1)]);
        let (themes, stats) = detect(&quotes, &PREMARKET_RULES);
        assert!(themes.is_empty());
        assert_eq!(stats.len(), 1);
        assert_eq!(
            stats[0],
            ThemeStat {
                group: "Airlines".into(),
                present: 3,
                up: 1,
                down: 1,
                median_up: Some(1.6),
                median_down: Some(-2.0),
            }
        );
        // A single strong mover does not.
        let (_, stats) = detect(&qs(&[("DAL", 1.6)]), &PREMARKET_RULES);
        assert!(stats.is_empty());

        // More than fifteen eligible groups are capped, largest present first.
        let mut all = Vec::new();
        for g in crate::premarket::groups() {
            let members: Vec<_> = crate::premarket::universe()
                .into_iter()
                .filter(|t| group(t) == Some(g))
                .take(2)
                .collect();
            for t in members {
                all.push(q(&t, 2.0));
            }
        }
        let (_, stats) = detect(&all, &PREMARKET_RULES);
        assert_eq!(stats.len(), THEME_STATS_MAX);
        assert!(stats.windows(2).all(|w| w[0].present >= w[1].present));
    }

    #[test]
    fn median_is_rounded() {
        let quotes = qs(&[("MU", 3.001), ("SNDK", 3.004), ("WDC", 3.01), ("STX", 3.02)]);
        let (themes, _) = detect(&quotes, &PREMARKET_RULES);
        // median of 3.001, 3.004, 3.01, 3.02 = 3.007
        assert_eq!(themes[0].median_pct, 3.01);
    }

    #[test]
    fn dual_share_classes_count_once() {
        // GOOG + GOOGL are one company: with MSFT that is two, not three.
        let quotes = qs(&[("GOOG", 4.0), ("GOOGL", 3.5), ("MSFT", 4.0)]);
        let (themes, _) = detect(&quotes, &PREMARKET_RULES);
        assert!(themes.is_empty(), "{themes:?}");
        // With a third distinct company the larger |move| class is kept.
        let quotes = qs(&[("GOOG", 4.0), ("GOOGL", 3.5), ("MSFT", 4.0), ("META", 5.0)]);
        let (themes, _) = detect(&quotes, &PREMARKET_RULES);
        assert_eq!(themes.len(), 1);
        let tickers: Vec<_> = themes[0]
            .members
            .iter()
            .map(|m| m.ticker.as_str())
            .collect();
        assert_eq!(tickers, ["META", "GOOG", "MSFT"]);
    }
}
