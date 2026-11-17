//! The labelling page: one self-contained HTML file listing about sixty real
//! lines in random order, with the judges' verdicts hidden, for the owner to
//! mark "Fine", "Wrong fact" or "Advice or not formal".

use crate::{line_key, Record};

pub(crate) const CARDS: usize = 60;
pub(crate) const SEED: u64 = 20_261_113;

/// splitmix64: a fixed-seed shuffle needs no dependency.
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    pub(crate) fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = (self.next() % (i as u64 + 1)) as usize;
            v.swap(i, j);
        }
    }
}

/// Lines that went to the judges.
fn judged(r: &Record) -> bool {
    r.insight.is_some() && r.check.is_empty()
}

/// The lines for the page: every judged line a judge said no to or the
/// judges disagreed on, then lines every judge passed, picked at random,
/// until `n`; shuffled.
pub(crate) fn pick(records: &[Record], n: usize) -> Vec<&Record> {
    let lines: Vec<&Record> = records.iter().filter(|r| judged(r)).collect();
    let (mut must, mut rest): (Vec<&Record>, Vec<&Record>) = lines
        .into_iter()
        .partition(|r| r.published_if.values().any(|p| !p));
    let mut rng = Rng(SEED);
    rng.shuffle(&mut rest);
    let room = n.saturating_sub(must.len());
    must.extend(rest.into_iter().take(room));
    rng.shuffle(&mut must);
    must
}

/// How many lines `pick` must show whatever `n` is.
pub(crate) fn must_show(records: &[Record]) -> usize {
    records
        .iter()
        .filter(|r| judged(r) && r.published_if.values().any(|p| !p))
        .count()
}

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// One line's card; `done` is the label it already has (pre-selected).
fn card(k: usize, r: &Record, done: Option<&str>) -> String {
    let i = r.insight.as_ref().expect("judged lines have an insight");
    let mut cites: Vec<&String> = i.en.cites.iter().chain(&i.zh.cites).collect();
    cites.sort();
    cites.dedup();
    let evidence: String = r
        .pack
        .evidence
        .iter()
        .map(|e| {
            let source = e
                .outlet
                .as_deref()
                .map_or(String::new(), |o| format!(" · {}", esc(o)));
            let cited = cites.contains(&&e.id);
            let quote = e.quote.as_deref().map_or(String::new(), |q| {
                format!(" <span class=\"quote\">Source text: {}</span>", esc(q))
            });
            format!(
                "<li{}><span class=\"tag\">{}{source}{}</span> {}{quote}</li>",
                if cited { " class=\"cited\"" } else { "" },
                esc(&serde_json::to_value(e.kind)
                    .expect("kind")
                    .as_str()
                    .unwrap_or("")
                    .replace('_', " ")),
                if cited { " · cited" } else { "" },
                esc(&e.text)
            )
        })
        .collect();
    let choice = |value: &str, label: &str| {
        let on = if done == Some(value) { " checked" } else { "" };
        format!("<label><input type=\"radio\" name=\"c{k}\" value=\"{value}\"{on}> {label}</label>")
    };
    format!(
        r#"<article class="card{class}" data-post="{post}" data-ticker="{ticker}" data-key="{key}">
<header><span class="n">{n}</span> <strong>{ticker}</strong> {name} <span class="post">{post}</span></header>
<div class="lines">
<div lang="en"><p class="hook">{en_hook}</p><p>{en_watch}</p></div>
<div lang="zh"><p class="hook">{zh_hook}</p><p>{zh_watch}</p></div>
</div>
<details open><summary>Evidence the writer had (the items the line cites are marked)</summary><ul>{evidence}</ul></details>
<fieldset>{fine}{wrong}{advice}</fieldset>
</article>"#,
        n = k + 1,
        class = if done.is_some() { " done" } else { "" },
        key = line_key(&r.writer_version, i),
        post = esc(&r.post),
        ticker = esc(&r.ticker),
        name = esc(&r.pack.name),
        en_hook = esc(&i.en.hook),
        en_watch = esc(&i.en.watch),
        zh_hook = esc(&i.zh.hook),
        zh_watch = esc(&i.zh.watch),
        fine = choice("fine", "Fine"),
        wrong = choice("wrong_fact", "Wrong fact"),
        advice = choice("advice_or_informal", "Advice or not formal"),
    )
}

const STYLE: &str = r#"
:root { --bg: #fafaf8; --fg: #1c1c1a; --muted: #6b6b66; --card: #ffffff; --line: #e2e2dc; --accent: #2457c5; }
@media (prefers-color-scheme: dark) {
  :root { --bg: #161615; --fg: #ececea; --muted: #a0a09a; --card: #20201e; --line: #34342f; --accent: #8fb0ff; }
}
* { box-sizing: border-box; }
body { margin: 0; background: var(--bg); color: var(--fg); font: 16px/1.5 -apple-system, "Segoe UI", "PingFang SC", "Noto Sans SC", sans-serif; }
main { max-width: 760px; margin: 0 auto; padding: 16px; }
h1 { font-size: 1.4rem; margin: 8px 0; }
.intro, .post, .tag, .n { color: var(--muted); }
.post, .tag { font-size: 0.85rem; }
.bar { position: sticky; top: 0; background: var(--bg); padding: 8px 0; border-bottom: 1px solid var(--line); display: flex; gap: 12px; align-items: center; flex-wrap: wrap; z-index: 1; }
button { font: inherit; padding: 6px 14px; border-radius: 6px; border: 1px solid var(--accent); background: var(--accent); color: var(--bg); cursor: pointer; }
.card { background: var(--card); border: 1px solid var(--line); border-radius: 8px; padding: 12px 16px; margin: 16px 0; }
.card.done { border-color: var(--accent); }
.lines { display: grid; gap: 8px; }
.lines p { margin: 2px 0; }
.hook { font-weight: 600; }
ul { padding-left: 18px; margin: 6px 0; }
li { margin: 4px 0; overflow-wrap: anywhere; color: var(--muted); }
li.cited { color: var(--fg); border-left: 3px solid var(--accent); padding-left: 8px; margin-left: -11px; }
.quote { display: block; font-size: 0.85rem; color: var(--muted); }
fieldset { border: 0; padding: 8px 0 0; margin: 0; display: flex; gap: 16px; flex-wrap: wrap; }
label { cursor: pointer; }
"#;

const SCRIPT: &str = r#"
(function () {
  var KEY = "insight-trial-labels";
  var cards = Array.prototype.slice.call(document.querySelectorAll(".card"));
  // Keyed by the line's text and writer version: a new run's lines never
  // inherit choices made for older text.
  function id(c) { return c.dataset.key; }
  function load() {
    try { return JSON.parse(localStorage.getItem(KEY) || "{}"); } catch (e) { return {}; }
  }
  function store(v) {
    try { localStorage.setItem(KEY, JSON.stringify(v)); } catch (e) { /* storage unavailable: labels live until reload */ }
  }
  var saved = load();
  function count() {
    var n = 0;
    cards.forEach(function (c) {
      var on = c.querySelector("input:checked");
      c.classList.toggle("done", !!on);
      if (on) n++;
    });
    document.getElementById("count").textContent = n + " of " + cards.length + " labelled";
  }
  cards.forEach(function (c) {
    var v = saved[id(c)];
    c.querySelectorAll("input").forEach(function (input) {
      if (input.value === v) input.checked = true;
      input.addEventListener("change", function () {
        saved[id(c)] = input.value;
        store(saved);
        count();
      });
    });
  });
  count();
  document.getElementById("save").addEventListener("click", function () {
    var out = [];
    cards.forEach(function (c) {
      var on = c.querySelector("input:checked");
      if (on) out.push({ post: c.dataset.post, ticker: c.dataset.ticker, key: c.dataset.key, label: on.value });
    });
    var blob = new Blob([JSON.stringify(out, null, 1)], { type: "application/json" });
    var a = document.createElement("a");
    a.href = URL.createObjectURL(blob);
    a.download = "labels.json";
    document.body.appendChild(a);
    a.click();
    a.remove();
  });
})();
"#;

/// The page for `records`.
pub(crate) fn page(records: &[Record], n: usize) -> String {
    let picked = pick(records, n);
    let cards: String = picked
        .iter()
        .enumerate()
        .map(|(k, r)| card(k, r, None))
        .collect::<Vec<_>>()
        .join("\n");
    let intro = format!(
        "{} lines, in random order. For each, read the English and Chinese lines against the evidence and choose one: <b>Fine</b>; <b>Wrong fact</b> (a claim the evidence does not support, or the two languages differ); <b>Advice or not formal</b>. Choices are kept in this browser; \"Save labels\" downloads <code>labels.json</code>.",
        picked.len()
    );
    html(&intro, &cards)
}

/// The shadow stage's cumulative page: every line the judge answered for,
/// new ones first; lines labelled before are shown done, with their label,
/// and saved again with the new ones, so one `labels.json` holds them all.
pub(crate) fn shadow_page(lines: &[(&Record, Option<String>)]) -> String {
    let cards: String = lines
        .iter()
        .enumerate()
        .map(|(k, (r, done))| card(k, r, done.as_deref()))
        .collect::<Vec<_>>()
        .join("\n");
    let old = lines.iter().filter(|(_, d)| d.is_some()).count();
    let intro = format!(
        "Shadow stage: {} new lines to read, in random order, then {old} you labelled before (marked, at the end). For each new one, read the English and Chinese lines against the evidence and choose one: <b>Fine</b>; <b>Wrong fact</b> (a claim the evidence does not support, or the two languages differ); <b>Advice or not formal</b>. The brief and its 8:35 update may carry the same mover: both are shown. Choices are kept in this browser; \"Save labels\" downloads <code>labels.json</code> with every label, old and new.",
        lines.len() - old
    );
    html(&intro, &cards)
}

fn html(intro: &str, cards: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Insight labels</title>
<style>{STYLE}</style>
</head>
<body>
<main>
<h1>Mover insight lines</h1>
<p class="intro">{intro}</p>
<div class="bar"><button id="save" type="button">Save labels</button><span id="count"></span></div>
{cards}
</main>
<script>{SCRIPT}</script>
</body>
</html>
"#
    )
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::JudgeResult;
    use prebell_core::insight::judge::Verdict;
    use prebell_core::insight::{Evidence, EvidenceKind, Insight, Line, Pack};
    use std::collections::BTreeMap;

    /// An answered verdict that passes or not.
    pub(crate) fn verdict(ticker: &str, passed: bool) -> Verdict {
        Verdict {
            ticker: ticker.into(),
            en_claims: vec![],
            zh_claims: vec![],
            en_supported: passed,
            zh_supported: passed,
            no_advice: true,
            formal: true,
            same_meaning: true,
            personal_claims_attributed: true,
            notes: String::new(),
        }
    }

    /// A judged line; `verdicts` are (claude passed, deepseek passed).
    pub(crate) fn record(post: &str, ticker: &str, verdicts: (bool, bool)) -> Record {
        let l = |hook: &str, watch: &str| Line {
            hook: hook.into(),
            watch: watch.into(),
            cites: vec!["e2".into()],
        };
        let j = |p: bool| JudgeResult {
            verdict: Some(verdict(ticker, p)),
            passed: p,
            error: None,
            retried: false,
        };
        Record {
            post: post.into(),
            ticker: ticker.into(),
            pack: Pack {
                ticker: ticker.into(),
                name: format!("{ticker} Inc"),
                evidence: vec![Evidence {
                    id: "e2".into(),
                    kind: EvidenceKind::News,
                    text: "The company <b>expects</b> revenue of $4.2 billion, per https://example.com/x.".into(),
                    outlet: Some("Reuters".into()),
                    url: Some("https://www.reuters.com/a".into()),
                    date: None,
                    quote: None,
                    unconfirmed: false,
                }],
            },
            insight: Some(Insight {
                ticker: ticker.into(),
                en: l("Guidance sets the test", "Reuters reports revenue of $4.2 billion."),
                zh: l("指引成为考验", "据路透社报道，营收42亿美元。"),
            }),
            write_error: None,
            check: vec![],
            judges: BTreeMap::from([("claude".to_string(), j(verdicts.0)), ("deepseek".to_string(), j(verdicts.1))]),
            published_if: BTreeMap::from([("claude".to_string(), verdicts.0), ("deepseek".to_string(), verdicts.1)]),
            writer_version: "w".into(),
            judge_version: "j".into(),
        }
    }

    fn records() -> Vec<Record> {
        (0..200)
            .map(|k| {
                let v = match k % 20 {
                    0 => (true, false),
                    1 => (false, true),
                    2 => (false, false),
                    _ => (true, true),
                };
                record(
                    &format!("recap_2026-11-{:02}", k % 10 + 1),
                    &format!("T{k}"),
                    v,
                )
            })
            .collect()
    }

    #[test]
    fn the_page_hides_verdicts_and_lists_sixty_of_two_hundred() {
        let rs = records();
        let html = page(&rs, CARDS);
        assert_eq!(html.matches("<article class=\"card\"").count(), 60);
        let lower = html.to_lowercase();
        for hidden in [
            "passed",
            "claude",
            "deepseek",
            "verdict",
            "published_if",
            "judge",
        ] {
            assert!(!lower.contains(hidden), "{hidden}");
        }
        // Every line a judge said no to is on the page: 30 of the 200.
        for r in rs.iter().filter(|r| r.published_if.values().any(|p| !p)) {
            assert!(
                html.contains(&format!("data-ticker=\"{}\"", r.ticker)),
                "{}",
                r.ticker
            );
        }
        // Shuffled, and the same page every time.
        assert_eq!(html, page(&rs, CARDS));
        let order: Vec<&str> = pick(&rs, CARDS).iter().map(|r| r.ticker.as_str()).collect();
        assert_ne!(order[..5], ["T0", "T1", "T2", "T20", "T21"]);
        // Lines the checks refused are never on the page.
        let mut refused = record("recap_2026-11-01", "BAD", (false, false));
        refused.check = vec!["en: refused wording: buy".into()];
        assert!(!page(&[refused], CARDS).contains("BAD"));
        // Fewer lines than sixty: all of them.
        assert_eq!(
            page(&rs[..7], CARDS)
                .matches("<article class=\"card\"")
                .count(),
            7
        );
    }

    #[test]
    fn the_page_is_self_contained() {
        let rs = records();
        let html = page(&rs, CARDS);
        assert!(!html.contains("<script src"));
        assert!(!html.contains("<link"));
        // The only URLs are inside evidence text (escaped, never a link).
        let evidence = "per https://example.com/x.";
        let without = html.replace(evidence, "");
        assert!(!without.contains("http://") && !without.contains("https://"));
        assert!(!html.contains("<b>expects</b>"));
        assert!(html.contains("&lt;b&gt;expects&lt;/b&gt;"));
        for want in [
            "Fine",
            "Wrong fact",
            "Advice or not formal",
            "Save labels",
            "labels.json",
            "localStorage",
            "try {",
        ] {
            assert!(html.contains(want), "{want}");
        }
    }

    /// Review M7, M5 and L4.
    #[test]
    fn the_page_shows_the_whole_pack_and_keys_choices_by_line() {
        let mut r = record("recap_2026-11-12", "AAPL", (true, false));
        r.pack.evidence.push(Evidence {
            id: "e3".into(),
            kind: EvidenceKind::CompanyFigure,
            text: "Revenue: $18.52 billion, reported for Q1 of fiscal 2027.".into(),
            outlet: None,
            url: None,
            date: None,
            quote: Some("Total revenue 18,517 14,883".into()),
            unconfirmed: false,
        });
        let html = page(std::slice::from_ref(&r), CARDS);
        // The uncited item is shown too, and only the cited one is marked.
        assert!(html.contains("Revenue: $18.52 billion"), "{html}");
        assert!(html.contains("Source text: Total revenue 18,517 14,883"));
        assert_eq!(html.matches("<li class=\"cited\">").count(), 1);
        // Saved choices are keyed by the line, so a new line starts blank.
        let key = line_key(&r.writer_version, r.insight.as_ref().unwrap());
        assert!(html.contains(&format!("data-key=\"{key}\"")));
        assert!(html.contains("return c.dataset.key;"));
        assert!(html.contains("key: c.dataset.key"));
        // More lines that must be shown than cards: all are, and main warns.
        let many: Vec<Record> = (0..70)
            .map(|k| record("recap_2026-11-12", &format!("T{k}"), (false, true)))
            .collect();
        assert_eq!(must_show(&many), 70);
        assert_eq!(
            page(&many, CARDS)
                .matches("<article class=\"card\"")
                .count(),
            70
        );
    }
}
