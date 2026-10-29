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
use crate::facts::Field;
use crate::numbers::{check, Lang};

const W: i32 = 640;
const H: i32 = 280;
const TOP: i32 = 34;
const BOTTOM: i32 = 44;
const SIDE: i32 = 16;
/// Bars: a mid-tone blue that reads on light and dark page backgrounds.
const BAR: RGBColor = RGBColor(0x2f, 0x7d, 0xc4);
/// Text and baseline: neutral grey with >= 3:1 contrast on white and on #141414.
const INK: RGBColor = RGBColor(0x80, 0x80, 0x80);
/// Fewer points than this is not a trend.
const MIN_BARS: usize = 2;
const RELEASE_LABEL: &str = "8-K";

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

fn bars(points: &[QuarterPoint], release: Option<f64>) -> Vec<Bar> {
    let mut out: Vec<Bar> = points.iter().map(quarter_bar).collect();
    if let Some(v) = release {
        out.push(Bar {
            label: RELEASE_LABEL.into(),
            value: v,
        });
    }
    out
}

fn draw(bars: &[Bar], labels: &[(usize, String)]) -> Result<String, String> {
    let mut svg = String::new();
    {
        let root = SVGBackend::with_string(&mut svg, (W as u32, H as u32)).into_drawing_area();
        let err = |e: DrawingAreaErrorKind<_>| format!("chart: {e:?}");
        let max = bars.iter().map(|b| b.value).fold(0.0_f64, f64::max);
        let min = bars.iter().map(|b| b.value).fold(0.0_f64, f64::min);
        let span = (max - min).max(f64::MIN_POSITIVE);
        let plot_h = f64::from(H - TOP - BOTTOM);
        let y = |v: f64| TOP + ((max - v) / span * plot_h).round() as i32;
        let zero = y(0.0);
        let slot = (W - 2 * SIDE) / bars.len() as i32;
        let gap = (slot / 4).max(2);
        let text = |size: i32, h: HPos, v: VPos| {
            ("sans-serif", size)
                .into_font()
                .color(&INK)
                .pos(Pos::new(h, v))
        };
        for (i, b) in bars.iter().enumerate() {
            let x0 = SIDE + i as i32 * slot + gap / 2;
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
                text(12, HPos::Center, VPos::Top),
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
                    text(13, HPos::Center, vpos),
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

fn release_value(f: &EarningsFacts, m: Metric) -> Option<f64> {
    if f.release_in_trend {
        return None;
    }
    f.metrics.value(m).map(|fig| fig.value)
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
            release_value(f, Metric::Revenue),
            "quarterly revenue (GAAP)",
            "季度营收（GAAP）",
        ),
        (
            "eps",
            &trend.eps_diluted,
            release_value(f, Metric::EpsGaap),
            "quarterly diluted EPS (GAAP)",
            "季度摊薄每股收益（GAAP）",
        ),
    ];
    let mut out = Vec::new();
    for (name, points, release, what_en, what_zh) in series {
        let b = bars(points, release);
        if b.len() < MIN_BARS {
            continue;
        }
        let ends = [0, b.len() - 1];
        let labels: Vec<(usize, String)> = ends
            .iter()
            .map(|&i| (i, value_label(name, b[i].value)))
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
    fn release_in_trend_is_not_added_twice() {
        let mut f = facts();
        f.release_in_trend = true;
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
        f.release_in_trend = true;
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
        let svg = draw(&b, &[]).unwrap();
        assert!(svg.contains("&lt;b&gt;&amp;") && !svg.contains("<b>"));
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
