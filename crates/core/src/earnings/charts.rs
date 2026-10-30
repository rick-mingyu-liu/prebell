//! GAAP-only quarterly bar charts as SVG, drawn with `plotters`.
//!
//! On `wasm32-unknown-unknown` plotters measures text through
//! `web_sys::window()`, which does not exist in a Worker and panics. So this
//! module only draws primitives (`Rectangle`, `PathElement`) and `Text` with
//! explicit anchors on a raw `DrawingArea`: never `ChartBuilder`,
//! `configure_mesh`, `titled` or `estimate_text_size`, which lay text out.

use chrono::{Datelike, NaiveDate};
use plotters::prelude::*;
use plotters::style::text_anchor::{HPos, Pos, VPos};

use crate::earnings::facts::{numbers_in_facts, EarningsFacts, Metric, QuarterPoint};
use crate::earnings::xbrl::release_already_in_trend;
use crate::facts::Field;
use crate::numbers::{check, Lang};

const W: i32 = 640;
const H: i32 = 280;
const TOP: i32 = 34;
const BOTTOM: i32 = 44;
const SIDE: i32 = 16;
/// Bars: a mid-tone blue that reads on light and dark page backgrounds.
const BAR: RGBColor = RGBColor(0x2f, 0x7d, 0xc4);
/// plotters' SVG backend writes `font-size = size / 1.24`: 16 -> ~13px.
/// Text and baseline: neutral grey with >= 3:1 contrast on white and on #141414.
const INK: RGBColor = RGBColor(0x80, 0x80, 0x80);
/// Fewer points than this is not a trend.
const MIN_BARS: usize = 2;
/// Widest slot per bar (bar plus gap), in px.
const MAX_SLOT: i32 = 96;
const RELEASE_LABEL: &str = "8-K";
/// Period ends further apart than one quarter (plus 52/53-week slack) mean
/// quarters in between are missing.
const QUARTER_DAYS: i64 = 100;
/// Typical days from a quarter's end to its earnings release.
const RELEASE_LAG_DAYS: i64 = 45;

#[derive(Debug, Clone, PartialEq)]
pub struct Chart {
    /// `revenue` or `eps`; also the file name.
    pub name: &'static str,
    pub svg: String,
    pub alt_en: String,
    pub alt_zh: String,
    /// The value labels drawn on the chart (checked against the facts).
    pub labels: Vec<String>,
}

/// One bar: its x label and GAAP value. Labels are language-neutral (one
/// SVG serves both languages): the quarter's period end as `YYYY-MM`, and
/// `8-K` for this press release's figure.
#[derive(Debug, Clone, PartialEq)]
struct Bar {
    label: String,
    value: f64,
}

fn quarter_bar(p: &QuarterPoint) -> Bar {
    let d: NaiveDate = p.period_end;
    Bar {
        label: format!("{}-{:02}", d.year(), d.month()),
        value: p.value,
    }
}

/// Revenue labels: "$102.5B" (or "$850.3M" below a billion); EPS: "$1.85".
fn value_label(name: &str, v: f64) -> String {
    let sign = if v < 0.0 { "-" } else { "" };
    let a = v.abs();
    match name {
        "eps" => format!("{sign}${a:.2}"),
        _ if a >= 1e9 => format!("{sign}${:.1}B", a / 1e9),
        _ => format!("{sign}${:.1}M", a / 1e6),
    }
}

/// Bars in time order; `None` is an empty slot where a quarter is missing
/// (consecutive period ends more than `GAP_DAYS` apart), so a missing fiscal
/// Q4 shows as a gap instead of Q3 sitting next to Q1.
fn bars(points: &[QuarterPoint], release: Option<f64>, filing_date: NaiveDate) -> Vec<Option<Bar>> {
    let mut out: Vec<Option<Bar>> = Vec::new();
    let mut prev: Option<NaiveDate> = None;
    for p in points {
        if let Some(prev) = prev {
            let missing = ((p.period_end - prev).num_days() - 1) / QUARTER_DAYS;
            out.extend(std::iter::repeat_n(None, missing.clamp(0, 3) as usize));
        }
        out.push(Some(quarter_bar(p)));
        prev = Some(p.period_end);
    }
    if let Some(v) = release {
        // The release quarter ended roughly `RELEASE_LAG_DAYS` before the filing.
        if let Some(prev) = prev {
            let release_end = filing_date - chrono::Duration::days(RELEASE_LAG_DAYS);
            let missing = ((release_end - prev).num_days() - 1) / QUARTER_DAYS;
            out.extend(std::iter::repeat_n(None, missing.clamp(0, 3) as usize));
        }
        out.push(Some(Bar {
            label: RELEASE_LABEL.into(),
            value: v,
        }));
    }
    out
}

fn draw(bars: &[Option<Bar>], labels: &[(usize, String)]) -> Result<String, String> {
    let mut svg = String::new();
    {
        let root = SVGBackend::with_string(&mut svg, (W as u32, H as u32)).into_drawing_area();
        let err = |e: DrawingAreaErrorKind<_>| format!("chart: {e:?}");
        let values = || bars.iter().flatten().map(|b| b.value);
        let max = values().fold(0.0_f64, f64::max);
        let min = values().fold(0.0_f64, f64::min);
        let span = (max - min).max(f64::MIN_POSITIVE);
        // Room under the lowest bar for a negative value label.
        let below = if min < 0.0 { 18 } else { 0 };
        let plot_h = f64::from(H - TOP - BOTTOM - below);
        let y = |v: f64| TOP + ((max - v) / span * plot_h).round() as i32;
        let zero = y(0.0);
        // Slots at most `MAX_SLOT` wide, centred, so a short series is not a wall of fat bars.
        let slot = ((W - 2 * SIDE) / bars.len() as i32).min(MAX_SLOT);
        let left = (W - slot * bars.len() as i32) / 2;
        let gap = (slot / 4).max(2);
        let text = |size: i32, h: HPos, v: VPos| {
            ("sans-serif", size)
                .into_font()
                .color(&INK)
                .pos(Pos::new(h, v))
        };
        for (i, b) in bars.iter().enumerate() {
            let Some(b) = b else { continue };
            let x0 = left + i as i32 * slot + gap / 2;
            let x1 = x0 + slot - gap;
            let (top, bottom) = if b.value >= 0.0 {
                (y(b.value), zero)
            } else {
                (zero, y(b.value))
            };
            root.draw(&Rectangle::new(
                [(x0, top), (x1, bottom.max(top + 1))],
                BAR.filled(),
            ))
            .map_err(err)?;
            let cx = (x0 + x1) / 2;
            root.draw(&Text::new(
                b.label.clone(),
                (cx, H - BOTTOM + 10),
                text(16, HPos::Center, VPos::Top),
            ))
            .map_err(err)?;
            if let Some((_, l)) = labels.iter().find(|(j, _)| *j == i) {
                let (ly, vpos) = if b.value >= 0.0 {
                    (top - 4, VPos::Bottom)
                } else {
                    (bottom + 4, VPos::Top)
                };
                root.draw(&Text::new(
                    l.clone(),
                    (cx, ly),
                    text(17, HPos::Center, vpos),
                ))
                .map_err(err)?;
            }
        }
        root.draw(&PathElement::new(
            vec![(SIDE, zero), (W - SIDE, zero)],
            INK.stroke_width(1),
        ))
        .map_err(err)?;
        root.present().map_err(err)?;
    }
    Ok(svg)
}

/// A release bar this many times the series' largest quarter is not a
/// quarter (a full-year or year-to-date total quoted as one).
const MAX_RELEASE_RATIO: f64 = 2.5;

/// The press release's GAAP figure for a series: only when the release's
/// fiscal quarter is verified (a full-year-only release has no quarter
/// bar), the series' latest XBRL quarter is not already this release's,
/// and the figure is at most `MAX_RELEASE_RATIO` times the series max.
fn release_value(f: &EarningsFacts, m: Metric, points: &[QuarterPoint]) -> Option<f64> {
    let quarter_verified = matches!(&f.fiscal_period, Field::Value(p) if p.quarter.is_some());
    if !quarter_verified || release_already_in_trend(points, f.filing_date) {
        return None;
    }
    let series_max = points.iter().map(|p| p.value.abs()).fold(0.0_f64, f64::max);
    f.metrics
        .value(m)
        .map(|fig| fig.value)
        .filter(|v| v.is_finite())
        .filter(|v| series_max == 0.0 || v.abs() <= MAX_RELEASE_RATIO * series_max)
}

/// One chart per series with at least two GAAP points. Value labels go on
/// the first and last bars only; a chart whose labels fail the number check
/// is dropped.
pub fn charts(f: &EarningsFacts) -> Vec<Chart> {
    let Field::Value(trend) = &f.trend else {
        return vec![];
    };
    let allowed = numbers_in_facts(f);
    let series = [
        (
            "revenue",
            &trend.revenue,
            release_value(f, Metric::Revenue, &trend.revenue),
            "quarterly revenue (GAAP)",
            "季度营收（GAAP）",
        ),
        (
            "eps",
            &trend.eps_diluted,
            release_value(f, Metric::EpsGaap, &trend.eps_diluted),
            "quarterly diluted EPS (GAAP)",
            "季度摊薄每股收益（GAAP）",
        ),
    ];
    let mut out = Vec::new();
    for (name, points, release, what_en, what_zh) in series {
        let b = bars(points, release, f.filing_date);
        if b.iter().flatten().count() < MIN_BARS {
            continue;
        }
        // The first and last slots always hold bars.
        let labels: Vec<(usize, String)> = [0, b.len() - 1]
            .iter()
            .filter_map(|&i| b[i].as_ref().map(|bar| (i, value_label(name, bar.value))))
            .collect();
        if labels
            .iter()
            .any(|(_, l)| check(l, Lang::En, &allowed).is_err())
        {
            continue;
        }
        let Ok(svg) = draw(&b, &labels) else {
            continue;
        };
        let latest = if release.is_some() {
            (
                "; the last bar is this 8-K's figure",
                "；最后一栏为本次8-K数据",
            )
        } else {
            ("", "")
        };
        out.push(Chart {
            name,
            svg,
            alt_en: format!("{} {what_en} by fiscal quarter end{}", f.company, latest.0),
            alt_zh: format!("{}{what_zh}，按财季截止月份{}", f.company, latest.1),
            labels: labels.into_iter().map(|(_, l)| l).collect(),
        });
    }
    out
}

/// R2 key for a chart SVG.
pub fn r2_key(f: &EarningsFacts, name: &str) -> String {
    format!(
        "charts/earnings/{}/{}/{name}.svg",
        f.ticker.replace('.', "-"),
        f.accession
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::earnings::facts::tests::facts;
    use crate::earnings::facts::{Figure, Trend, Unit};

    #[test]
    fn revenue_and_eps_charts() {
        let c = charts(&facts());
        assert_eq!(
            c.iter().map(|c| c.name).collect::<Vec<_>>(),
            ["revenue", "eps"]
        );
        let rev = &c[0];
        assert!(rev.svg.starts_with("<svg"));
        // Two XBRL quarters + this release.
        assert_eq!(rev.svg.matches("<rect").count(), 3);
        assert!(
            rev.svg.contains(">\n8-K\n<") || rev.svg.contains(">8-K<"),
            "{}",
            rev.svg
        );
        assert!(rev.svg.contains("2026-06"));
        assert_eq!(rev.labels, ["$100.4B", "$102.5B"]);
        assert_eq!(c[1].labels, ["$1.80", "$1.85"]);
        assert!(rev.alt_en.contains("this 8-K"));
        assert!(rev.alt_zh.contains("本次8-K"));
    }

    #[test]
    fn full_year_release_is_not_a_quarter_bar() {
        // A full-year-only release (no quarter): its revenue is a year's.
        let mut f = facts();
        if let Field::Value(p) = &mut f.fiscal_period {
            p.quarter = None;
        }
        let c = charts(&f);
        assert!(c.iter().all(|c| !c.svg.contains("8-K")), "no release bar");
        assert_eq!(c[0].svg.matches("<rect").count(), 2);
        // An unverified fiscal period: no release bar either.
        let mut f = facts();
        f.fiscal_period = Field::Missing { reason: "x".into() };
        assert!(charts(&f).iter().all(|c| !c.svg.contains("8-K")));
    }

    #[test]
    fn release_far_above_the_series_is_dropped() {
        // A full-year total quoted as the quarter: 4x the quarterly bars.
        let mut f = facts();
        if let Field::Value(fig) = &mut f.metrics.revenue {
            fig.value = 401.6e9;
        }
        let rev = charts(&f)
            .into_iter()
            .find(|c| c.name == "revenue")
            .unwrap();
        assert!(!rev.svg.contains("8-K"));
        // Up to 2.5x the series max is kept.
        let mut f = facts();
        if let Field::Value(fig) = &mut f.metrics.revenue {
            fig.value = 2.5 * 100.4e9;
        }
        assert_eq!(
            release_value(&f, Metric::Revenue, &trend_of(&f).revenue),
            Some(2.5 * 100.4e9)
        );
    }

    fn trend_of(f: &EarningsFacts) -> Trend {
        match &f.trend {
            Field::Value(t) => t.clone(),
            Field::Missing { .. } => Trend::default(),
        }
    }

    #[test]
    fn release_in_trend_is_not_added_twice() {
        let mut f = facts();
        // The 10-Q for the release quarter was filed with the 8-K.
        f.filing_date = "2026-07-31".parse().unwrap();
        let c = charts(&f);
        assert!(!c[0].svg.contains("8-K"));
        assert_eq!(c[0].svg.matches("<rect").count(), 2);
    }

    #[test]
    fn non_gaap_eps_never_charted() {
        let mut f = facts();
        f.metrics.eps_gaap = Field::Missing { reason: "x".into() };
        f.metrics.eps_non_gaap = Field::Value(Figure {
            value: 2.22,
            unit: Unit::UsdPerShare,
            amount: "$2.22".into(),
            scale: 1.0,
            quote: "non-GAAP diluted earnings per share of $2.22".into(),
        });
        let c = charts(&f);
        let eps = c.iter().find(|c| c.name == "eps").unwrap();
        assert!(!eps.svg.contains("2.22"));
        assert!(!eps.svg.contains("8-K"));
    }

    #[test]
    fn too_few_points_no_chart() {
        let mut f = facts();
        f.metrics.revenue = Field::Missing { reason: "x".into() };
        if let Field::Value(t) = &mut f.trend {
            t.revenue.truncate(1);
        }
        assert!(charts(&f).iter().all(|c| c.name != "revenue"));
        f.trend = Field::Value(Trend::default());
        assert!(charts(&f).is_empty());
    }

    #[test]
    fn negative_values_and_labels() {
        assert_eq!(value_label("eps", -0.25), "-$0.25");
        assert_eq!(value_label("revenue", 850.3e6), "$850.3M");
        let b = vec![
            Bar {
                label: "a".into(),
                value: 1.0,
            },
            Bar {
                label: "b".into(),
                value: -0.5,
            },
        ];
        let b: Vec<Option<Bar>> = b.into_iter().map(Some).collect();
        let svg = draw(&b, &[(1, "-$0.50".into())]).unwrap();
        assert_eq!(svg.matches("<rect").count(), 2);
        assert!(svg.contains("-$0.50"));
    }

    #[test]
    fn text_is_escaped() {
        let b = vec![
            Bar {
                label: "<b>&".into(),
                value: 1.0,
            },
            Bar {
                label: "y".into(),
                value: 2.0,
            },
        ];
        let b: Vec<Option<Bar>> = b.into_iter().map(Some).collect();
        let svg = draw(&b, &[]).unwrap();
        assert!(svg.contains("&lt;b&gt;&amp;") && !svg.contains("<b>"));
    }

    #[test]
    fn missing_quarters_are_gaps() {
        let qp = |end: &str, v: f64| QuarterPoint {
            period_end: end.parse().unwrap(),
            value: v,
            form: "10-Q".into(),
            filed: "2026-08-01".parse().unwrap(),
        };
        // Q3 (Jun) then Q1 (Dec): the Sep quarter is missing.
        let pts = [
            qp("2025-06-28", 1.0),
            qp("2025-12-27", 2.0),
            qp("2026-03-28", 3.0),
        ];
        let b = bars(&pts, Some(4.0), "2026-07-30".parse().unwrap());
        let shape: Vec<bool> = b.iter().map(Option::is_some).collect();
        assert_eq!(shape, [true, false, true, true, true]);
        // A release long after the last XBRL quarter leaves a gap before it.
        let b = bars(&pts[..1], Some(4.0), "2026-04-30".parse().unwrap());
        assert_eq!(b.iter().filter(|x| x.is_none()).count(), 2);
    }

    #[test]
    fn eps_series_checked_on_its_own() {
        // Revenue history is missing (e.g. a bank); EPS has the release quarter already.
        let mut f = facts();
        f.filing_date = "2026-07-31".parse().unwrap();
        if let Field::Value(t) = &mut f.trend {
            t.revenue.clear();
        }
        let c = charts(&f);
        let eps = c.iter().find(|c| c.name == "eps").unwrap();
        assert!(!eps.svg.contains("8-K"));
    }

    #[test]
    fn non_finite_release_is_ignored() {
        let mut f = facts();
        if let Field::Value(fig) = &mut f.metrics.revenue {
            fig.value = f64::INFINITY;
        }
        let rev = charts(&f)
            .into_iter()
            .find(|c| c.name == "revenue")
            .unwrap();
        assert!(!rev.svg.contains("8-K"));
    }

    #[test]
    fn key_format() {
        let mut f = facts();
        f.ticker = "BRK.B".into();
        assert_eq!(
            r2_key(&f, "eps"),
            "charts/earnings/BRK-B/0000320193-26-000070/eps.svg"
        );
    }
}
