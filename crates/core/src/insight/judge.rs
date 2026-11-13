//! The judge: a second model call that splits each line into claims and
//! quotes the evidence words that support each one. Code then confirms the
//! quotes are verbatim, so the judge cannot invent support. It only removes
//! lines; it never edits them.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::check::{dates_not_in, numbers_from, outlet_names, scrub, year_of};
use super::write::movers_json;
use super::{Insight, Line, Pack};
use crate::ipo::news::visible;
use crate::numbers::Lang;
use crate::providers::{LlmProvider, ProviderError};

pub const JUDGE_VERSION: &str = "insight-judge/v2";

/// Shortest support, in letters and digits, that can confirm a claim: a
/// single common word confirms nothing.
const MIN_SUPPORT: usize = 12;
/// The same for support that is mostly Chinese characters.
const MIN_SUPPORT_CJK: usize = 6;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Claim {
    pub claim: String,
    pub evidence_id: String,
    pub support: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Verdict {
    pub ticker: String,
    pub en_claims: Vec<Claim>,
    pub zh_claims: Vec<Claim>,
    pub en_supported: bool,
    pub zh_supported: bool,
    pub no_advice: bool,
    pub formal: bool,
    pub same_meaning: bool,
    /// No allegation about a named person's private life, and no person
    /// named in connection with alleged wrongdoing or an investigation.
    /// Absent (false) in verdicts from before judge v2.
    #[serde(default)]
    pub no_personal_allegation: bool,
    pub notes: String,
}

const SYSTEM: &str = r#"You check short readings of big stock movers that another writer produced for PreBell, a daily US market brief. You are checking, not rewriting: never suggest new wording.

For each mover you get its evidence items (each with an id) and the lines written about it: an English hook and sentence ("en") and a Chinese hook and sentence ("zh").

1. Split each language's hook and sentence into the factual claims they make (a figure, a date, an event, a plan, who said what). Attribution ("Reuters reports", "据路透社报道") is checked by code: do not list it as a claim. For each claim give the id of the evidence item that supports it and copy the exact words from that item's text that support it: a verbatim span, character for character, in the evidence's own language, long enough to show the claim (not a single word). If no item supports a claim, or the words say something different from the claim (another figure, another subject, another direction), set that language's "*_supported" to false.
2. Each "news" or "analyst_action" item has "role": "reason for the move": PreBell's brief gives it as a reason the stock moved, so a hook that ties the move to it ("Margin contraction behind the drop") is supported by that item. PreBell's own framing of why something matters is allowed only when it follows from a supported claim. A report date the evidence marks "not confirmed by the company" must not be stated as the company's own expectation (as in "英伟达预计于11月17日发布财报"); if it is, set that language's "*_supported" to false. A fact taken from a news item must be attributed in the line to that item's outlet; if it is not, or it is attributed to another outlet, set that language's "*_supported" to false.
3. Set "no_advice" to false if a line gives investment advice of any kind, a rating, a price target or a prediction of where the price goes.
4. Set "formal" to false if the tone is informal, promotional or sensational.
5. Set "same_meaning" to false if the Chinese lines do not say the same thing as the English ones (a different figure, date, outlet or claim).
6. Set "no_personal_allegation" to false if a line repeats an allegation about a named person's private life, or names a person in connection with alleged wrongdoing or an investigation. Stating the company-level fact ("the board opened an independent investigation") is fine, and so is naming an executive for what they said in their role.

Use only the evidence given, never what you remember. Reply with JSON only, in this shape, one verdict per mover:
{"verdicts":[{"ticker":"...","en_claims":[{"claim":"...","evidence_id":"e2","support":"exact words"}],"zh_claims":[{"claim":"...","evidence_id":"e2","support":"exact words"}],"en_supported":true,"zh_supported":true,"no_advice":true,"formal":true,"same_meaning":true,"no_personal_allegation":true,"notes":"one short sentence"}]}"#;

/// The insights that have a pack, with it.
fn pairs<'a>(packs: &'a [Pack], insights: &'a [Insight]) -> Vec<(&'a Pack, &'a Insight)> {
    insights
        .iter()
        .filter_map(|i| packs.iter().find(|p| p.ticker == i.ticker).map(|p| (p, i)))
        .collect()
}

/// (system, user) for one judge call over `insights`.
pub fn prompt(packs: &[Pack], insights: &[Insight]) -> (String, String) {
    let movers: Vec<Value> = pairs(packs, insights)
        .into_iter()
        .map(|(p, i)| {
            let mut m = movers_json(std::slice::from_ref(p)).remove(0);
            m["lines"] = json!({
                "en": {"hook": i.en.hook, "watch": i.en.watch},
                "zh": {"hook": i.zh.hook, "watch": i.zh.watch},
            });
            m
        })
        .collect();
    (SYSTEM.to_string(), json!({ "movers": movers }).to_string())
}

fn claims(v: &Value, key: &str) -> Vec<Claim> {
    let s = |c: &Value, k: &str| c.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|c| Claim {
                    claim: s(c, "claim"),
                    evidence_id: s(c, "evidence_id"),
                    support: s(c, "support"),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// One verdict per mover of `insights` the reply covers, first one wins. A
/// missing flag reads as "no" and a malformed claim as unsupported.
pub fn parse(reply: &Value, insights: &[Insight]) -> Vec<Verdict> {
    let items = reply
        .get("verdicts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out: Vec<Verdict> = Vec::new();
    for it in &items {
        let Some(t) = it.get("ticker").and_then(Value::as_str) else {
            continue;
        };
        let Some(i) = insights
            .iter()
            .find(|i| i.ticker.eq_ignore_ascii_case(t.trim()))
        else {
            continue;
        };
        if out.iter().any(|v| v.ticker == i.ticker) {
            continue;
        }
        let flag = |k: &str| it.get(k).and_then(Value::as_bool).unwrap_or(false);
        out.push(Verdict {
            ticker: i.ticker.clone(),
            en_claims: claims(it, "en_claims"),
            zh_claims: claims(it, "zh_claims"),
            en_supported: flag("en_supported"),
            zh_supported: flag("zh_supported"),
            no_advice: flag("no_advice"),
            formal: flag("formal"),
            same_meaning: flag("same_meaning"),
            no_personal_allegation: flag("no_personal_allegation"),
            notes: it
                .get("notes")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        });
    }
    out
}

/// Without invisible characters, whitespace runs as one space, ASCII lowercase.
fn norm(s: &str) -> String {
    visible(s)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

/// Words that confirm nothing on their own ("the company said").
const GENERIC: &[&str] = &[
    "the",
    "a",
    "an",
    "of",
    "to",
    "in",
    "on",
    "for",
    "and",
    "or",
    "with",
    "as",
    "by",
    "at",
    "from",
    "it",
    "its",
    "that",
    "this",
    "is",
    "was",
    "are",
    "were",
    "be",
    "been",
    "will",
    "would",
    "has",
    "have",
    "had",
    "said",
    "says",
    "say",
    "company",
    "companies",
    "according",
    "reported",
    "reports",
    "also",
    "which",
    "who",
    "after",
    "before",
    "than",
    "their",
    "they",
];

fn long_enough(support: &str) -> bool {
    let alnum = support.chars().filter(|c| c.is_alphanumeric()).count();
    let cjk = support
        .chars()
        .filter(|c| matches!(c, '\u{3400}'..='\u{4dbf}' | '\u{4e00}'..='\u{9fff}'))
        .count();
    if cjk * 2 > alnum {
        return alnum >= MIN_SUPPORT_CJK;
    }
    // At least two words that carry meaning, or a figure.
    let telling = support
        .split(|c: char| !c.is_alphanumeric() && c != '$' && c != '%')
        .filter(|w| !w.is_empty() && !GENERIC.contains(w))
        .count();
    alnum >= MIN_SUPPORT && (telling >= 2 || support.chars().any(|c| c.is_ascii_digit()))
}

/// Whether a claim only states the attribution: its support is a name of
/// the named item's outlet ("StockStory", "据StockStory报道"). The outlet is
/// a field, not in the item's text, and code checks attribution itself, so
/// such a claim is ignored rather than failed.
fn attribution_claim(pack: &Pack, c: &Claim) -> bool {
    let Some(outlet) = pack
        .evidence
        .iter()
        .find(|e| e.id == c.evidence_id)
        .and_then(|e| e.outlet.as_deref())
    else {
        return false;
    };
    let support = c
        .support
        .trim()
        .trim_matches(|ch: char| {
            ch.is_ascii_punctuation() && ch != '\'' || "，。、（）".contains(ch)
        })
        .trim();
    let support = support
        .strip_prefix('据')
        .and_then(|s| s.strip_suffix("报道"))
        .unwrap_or(support);
    let lower = support.to_ascii_lowercase();
    let support = lower
        .strip_suffix(" reports")
        .or_else(|| lower.strip_suffix(" reported"))
        .unwrap_or(&lower)
        .trim();
    [Lang::En, Lang::Zh]
        .iter()
        .flat_map(|l| outlet_names(outlet, *l))
        .any(|n| n.to_ascii_lowercase() == support)
}

/// The claims that state a fact, not only the attribution.
fn fact_claims<'a>(pack: &Pack, claims: &'a [Claim]) -> Vec<&'a Claim> {
    claims
        .iter()
        .filter(|c| !attribution_claim(pack, c))
        .collect()
}

/// A span shorter than the minimum that still holds a figure or date the
/// claim itself states ("for $4B" for "The sale price is $4B"): exact, so
/// it confirms. A short span with no such figure does not.
fn pins_a_stated_figure(pack: &Pack, support: &str, claim: &str) -> bool {
    let values = |t: &str| -> Vec<f64> {
        crate::numbers::extract(t, Lang::En)
            .iter()
            .map(|n| n.value.abs())
            .collect()
    };
    let (s, c) = (values(support), values(claim));
    let figure = s.iter().any(|a| {
        c.iter()
            .any(|b| a.max(*b) > 0.0 && (a - b).abs() / a.max(*b) < 0.006)
    });
    let date = year_of(pack).is_some_and(|y| {
        let d = super::check::dates_in(claim, Lang::En, y);
        super::check::dates_in(support, Lang::En, y)
            .iter()
            .any(|x| d.contains(x))
    });
    figure || date
}

/// True when every claim's `support` is found verbatim in the named
/// evidence item of this pack. A claim that only states the attribution is
/// ignored.
pub fn support_confirmed(pack: &Pack, claims: &[Claim]) -> bool {
    fact_claims(pack, claims).iter().all(|c| {
        let support = norm(&c.support);
        (long_enough(&support) || pins_a_stated_figure(pack, &c.support, &c.claim))
            && pack
                .evidence
                .iter()
                .find(|e| e.id == c.evidence_id)
                .is_some_and(|e| norm(&e.text).contains(&support))
    })
}

/// Whether every number and date in `line` appears in the evidence the
/// judge confirmed for it: in its quoted spans, or in the full text (and
/// date) of an item a confirmed claim names, so a span that leaves out the
/// item's period ("For the fourth quarter of 2026, …") still counts. One
/// easy claim cannot carry a line whose figures its item never states. The
/// names of the line's cited outlets ("24/7 Wall St.") are left out.
fn figures_supported(pack: &Pack, line: &Line, claims: &[Claim], lang: Lang) -> bool {
    let claims = fact_claims(pack, claims);
    let named: Vec<&super::Evidence> = pack
        .evidence
        .iter()
        .filter(|e| claims.iter().any(|c| c.evidence_id == e.id))
        .collect();
    let outlets: Vec<String> = pack
        .evidence
        .iter()
        .filter(|e| line.cites.contains(&e.id) || named.iter().any(|n| n.id == e.id))
        .filter_map(|e| e.outlet.as_deref())
        .flat_map(|o| outlet_names(o, lang))
        .collect();
    let text = scrub(&format!("{} {}", line.hook, line.watch), &outlets);
    let mut texts: Vec<&str> = claims.iter().map(|c| c.support.as_str()).collect();
    texts.extend(named.iter().map(|e| e.text.as_str()));
    let dates: Vec<chrono::NaiveDate> = named.iter().filter_map(|e| e.date).collect();
    numbers_from(&text, lang, &texts).is_ok()
        && dates_not_in(&text, year_of(pack), &dates, &texts).is_empty()
}

/// The final answer for one mover: every flag true, at least one claim of
/// fact per language, support confirmed for both, and every number and date
/// of each line within the evidence its confirmed claims name.
pub fn passed(pack: &Pack, insight: &Insight, v: &Verdict) -> bool {
    v.ticker == pack.ticker
        && insight.ticker == pack.ticker
        && v.en_supported
        && v.zh_supported
        && v.no_advice
        && v.formal
        && v.same_meaning
        && v.no_personal_allegation
        && !fact_claims(pack, &v.en_claims).is_empty()
        && !fact_claims(pack, &v.zh_claims).is_empty()
        && support_confirmed(pack, &v.en_claims)
        && support_confirmed(pack, &v.zh_claims)
        && figures_supported(pack, &insight.en, &v.en_claims, Lang::En)
        && figures_supported(pack, &insight.zh, &v.zh_claims, Lang::Zh)
}

/// One judge call over the insights that have a pack, no retry. None, no call.
pub async fn judge(
    llm: &dyn LlmProvider,
    packs: &[Pack],
    insights: &[Insight],
) -> Result<Vec<Verdict>, ProviderError> {
    if pairs(packs, insights).is_empty() {
        return Ok(vec![]);
    }
    let (system, user) = prompt(packs, insights);
    let reply = llm.complete_json(&system, &user).await?;
    Ok(parse(&reply, insights))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::{Evidence, EvidenceKind, Insight, Line, Pack};
    use async_trait::async_trait;
    use serde_json::{json, Value};

    fn pack(text: &str) -> Pack {
        Pack {
            ticker: "AAPL".into(),
            name: "Apple".into(),
            evidence: vec![
                Evidence {
                    id: "e1".into(),
                    kind: EvidenceKind::Move,
                    text: "Apple (AAPL) rose 4.20% in premarket trading.".into(),
                    outlet: None,
                    url: None,
                    date: Some("2026-11-12".parse().unwrap()),
                    quote: None,
                    unconfirmed: false,
                },
                Evidence {
                    id: "e2".into(),
                    kind: EvidenceKind::News,
                    text: text.into(),
                    outlet: Some("Reuters".into()),
                    url: Some("https://www.reuters.com/x".into()),
                    date: None,
                    quote: None,
                    unconfirmed: false,
                },
            ],
        }
    }

    const TEXT: &str =
        "Apple reported revenue of $4.2 billion and announced a $1.0 billion buyback.";

    fn insight(ticker: &str) -> Insight {
        let l = |hook: &str, watch: &str| Line {
            hook: hook.into(),
            watch: watch.into(),
            cites: vec!["e2".into()],
        };
        Insight {
            ticker: ticker.into(),
            en: l(
                "Buyback in focus",
                "Reuters reports a $1.0 billion buyback.",
            ),
            zh: l("回购受关注", "据路透社报道，公司宣布10亿美元回购。"),
        }
    }

    fn claim(claim: &str, id: &str, support: &str) -> Claim {
        Claim {
            claim: claim.into(),
            evidence_id: id.into(),
            support: support.into(),
        }
    }

    fn verdict() -> Verdict {
        Verdict {
            ticker: "AAPL".into(),
            en_claims: vec![claim(
                "a $1.0 billion buyback",
                "e2",
                "announced a $1.0 billion buyback",
            )],
            zh_claims: vec![claim("10亿美元回购", "e2", "a $1.0 billion buyback")],
            en_supported: true,
            zh_supported: true,
            no_advice: true,
            formal: true,
            same_meaning: true,
            no_personal_allegation: true,
            notes: String::new(),
        }
    }

    fn reply_item(ticker: &str) -> Value {
        json!({"ticker": ticker,
            "en_claims": [{"claim": "a $1.0 billion buyback", "evidence_id": "e2", "support": "announced a $1.0 billion buyback"}],
            "zh_claims": [{"claim": "10亿美元回购", "evidence_id": "e2", "support": "a $1.0 billion buyback"}],
            "en_supported": true, "zh_supported": true,
            "no_advice": true, "formal": true, "same_meaning": true,
            "no_personal_allegation": true, "notes": "ok"})
    }

    struct Canned(Value);
    #[async_trait(?Send)]
    impl crate::providers::LlmProvider for Canned {
        async fn complete_json(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Value, crate::providers::ProviderError> {
            Ok(self.0.clone())
        }
    }

    struct Panics;
    #[async_trait(?Send)]
    impl crate::providers::LlmProvider for Panics {
        async fn complete_json(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Value, crate::providers::ProviderError> {
            panic!("no call expected")
        }
    }

    #[test]
    fn the_prompt_carries_the_packs_and_the_lines() {
        let (system, user) = prompt(&[pack(TEXT)], &[insight("AAPL")]);
        let v: Value = serde_json::from_str(&user).unwrap();
        let m = &v["movers"][0];
        assert_eq!(m["ticker"], "AAPL");
        assert_eq!(m["evidence"][1]["text"], TEXT);
        assert_eq!(
            m["lines"]["en"]["watch"],
            "Reuters reports a $1.0 billion buyback."
        );
        assert_eq!(m["lines"]["zh"]["hook"], "回购受关注");
        assert!(!user.contains("http"));
        for want in [
            "not rewriting",
            "exact words",
            "same_meaning",
            "JSON only",
            // Review L7: a missing attribution has a flag to set.
            "attributed to another outlet, set that language's \"*_supported\" to false",
        ] {
            assert!(system.contains(want), "{want}");
        }
    }

    #[test]
    fn parse_reads_a_full_verdict() {
        let mut want = verdict();
        want.notes = "ok".into();
        assert_eq!(
            parse(
                &json!({"verdicts": [reply_item("AAPL")]}),
                &[insight("AAPL")]
            ),
            [want]
        );
        // A missing flag reads as "no".
        let mut item = reply_item("AAPL");
        item.as_object_mut().unwrap().remove("formal");
        let v = parse(&json!({"verdicts": [item]}), &[insight("AAPL")]);
        assert!(!v[0].formal);
    }

    #[test]
    fn a_mover_missing_from_the_reply_has_no_verdict() {
        let insights = [insight("AAPL"), insight("NVDA")];
        let v = parse(
            &json!({"verdicts": [reply_item("NVDA"), reply_item("TSLA")]}),
            &insights,
        );
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].ticker, "NVDA");
        assert!(parse(&json!({"nothing": []}), &insights).is_empty());
        // A second verdict for one mover is ignored.
        let mut second = reply_item("NVDA");
        second["formal"] = json!(false);
        let v = parse(
            &json!({"verdicts": [reply_item("NVDA"), second]}),
            &insights,
        );
        assert_eq!(v.len(), 1);
        assert!(v[0].formal);
    }

    #[test]
    fn support_must_be_verbatim_in_the_named_item() {
        let p = pack(TEXT);
        assert!(support_confirmed(
            &p,
            &[claim("buyback", "e2", "announced a $1.0 billion buyback")]
        ));
        assert!(!support_confirmed(
            &p,
            &[claim(
                "buyback",
                "e2",
                "said it would buy back $1.0 billion of stock"
            )]
        ));
        assert!(!support_confirmed(
            &p,
            &[claim("buyback", "e1", "announced a $1.0 billion buyback")]
        ));
        assert!(!support_confirmed(
            &p,
            &[claim("buyback", "e9", "announced a $1.0 billion buyback")]
        ));
        // Every claim must be confirmed.
        assert!(!support_confirmed(
            &p,
            &[
                claim("buyback", "e2", "announced a $1.0 billion buyback"),
                claim("revenue", "e2", "revenue of $5.0 billion"),
            ]
        ));
    }

    #[test]
    fn support_ignores_invisible_characters_and_spacing() {
        let p = pack("Apple will list on \u{200b}the  Nasdaq on November 19");
        assert!(support_confirmed(
            &p,
            &[claim("listing", "e2", "on the Nasdaq on November 19")]
        ));
        assert!(support_confirmed(
            &p,
            &[claim("listing", "e2", "ON THE NASDAQ  on\nNovember 19")]
        ));
    }

    #[test]
    fn a_too_short_support_is_not_confirmed() {
        let p = pack(TEXT);
        assert!(!support_confirmed(&p, &[claim("buyback", "e2", "buyback")]));
        assert!(!support_confirmed(&p, &[claim("buyback", "e2", "")]));
        let zh = pack("公司宣布十亿美元回购计划并上调全年指引");
        assert!(!support_confirmed(&zh, &[claim("回购", "e2", "回购计划")]));
        assert!(support_confirmed(
            &zh,
            &[claim("回购", "e2", "十亿美元回购计划")]
        ));
    }

    #[test]
    fn passed_needs_every_flag_and_confirmed_support_in_both_languages() {
        let p = pack(TEXT);
        assert!(passed(&p, &insight("AAPL"), &verdict()));
        let flags: [fn(&mut Verdict); 6] = [
            |v| v.en_supported = false,
            |v| v.zh_supported = false,
            |v| v.no_advice = false,
            |v| v.formal = false,
            |v| v.same_meaning = false,
            |v| v.no_personal_allegation = false,
        ];
        for f in flags {
            let mut v = verdict();
            f(&mut v);
            assert!(!passed(&p, &insight("AAPL"), &v), "{v:?}");
        }
        let mut v = verdict();
        v.zh_claims.clear();
        assert!(!passed(&p, &insight("AAPL"), &v));
        let mut v = verdict();
        v.en_claims[0].support = "a $1.0 billion share repurchase".into();
        assert!(!passed(&p, &insight("AAPL"), &v));
        let mut v = verdict();
        v.ticker = "NVDA".into();
        assert!(!passed(&p, &insight("AAPL"), &v));
    }

    /// Review Focus 1: a rewritten support for a swapped meaning fails
    /// here. A long verbatim span about the other figure ("revenue of
    /// $4.2 billion" for "a $4.2 billion buyback") is confirmed: catching
    /// that is the judge's job, which the seeded `number_swap` lines measure.
    #[test]
    fn a_short_or_rewritten_support_for_a_swapped_meaning_fails() {
        let p = pack(TEXT);
        let mut v = verdict();
        // A short span holding the claim's own figure is exact and
        // confirms (trial run 4, NRG's "for $4B"): the swapped meaning in it
        // is the judge's to catch.
        v.en_claims = vec![claim("a $4.2 billion buyback", "e2", "$4.2 billion")];
        assert!(support_confirmed(&p, &v.en_claims));
        v.en_claims = vec![claim(
            "a $4.2 billion buyback",
            "e2",
            "a $4.2 billion buyback",
        )];
        assert!(!support_confirmed(&p, &v.en_claims));
        assert!(!passed(&p, &insight("AAPL"), &v));
        let judges_job = [claim(
            "a $4.2 billion buyback",
            "e2",
            "revenue of $4.2 billion",
        )];
        assert!(support_confirmed(&p, &judges_job));
    }

    /// Review M4: one easy claim cannot carry a line whose figures it never
    /// quoted, and a generic span confirms nothing.
    #[test]
    fn every_figure_in_a_line_needs_quoted_support() {
        let p = pack(TEXT);
        let easy = claim("Apple rose", "e1", "Apple (AAPL) rose 4.20%");
        let mut v = verdict();
        v.en_claims = vec![easy.clone()];
        v.zh_claims = vec![easy.clone()];
        assert!(support_confirmed(&p, &v.en_claims));
        assert!(!passed(&p, &insight("AAPL"), &v));
        // The same claim passes a line whose only figure it quotes.
        let mut i = insight("AAPL");
        i.en.watch = "Reuters reports the shares rose 4.2% before the buyback news.".into();
        i.zh.watch = "据路透社报道，股价上涨4.2%。".into();
        assert!(passed(&p, &i, &v));
        // A date in the line must be quoted too.
        let mut i = insight("AAPL");
        i.en.watch =
            "Reuters reports a $1.0 billion buyback, to be completed by November 19.".into();
        assert!(!passed(&p, &i, &verdict()));
        // A generic span confirms nothing, however long.
        let q = pack("Apple plans to open a factory in Texas next year, the company said.");
        assert!(!support_confirmed(
            &q,
            &[claim("Apple will triple revenue", "e2", "the company said")]
        ));
    }

    #[test]
    fn padding_and_rewording_do_not_confirm_support() {
        let p = pack("Apple said it expects third quarter revenue of $4.2 billion and a $1.0 billion buyback.");
        let c = |s: &str, id: &str| [claim("rev", id, s)];
        assert!(!support_confirmed(
            &p,
            &c(&format!("{}buyback", "\u{200b}".repeat(30)), "e2")
        ));
        assert!(!support_confirmed(&p, &c(" .... , , buyback ;;; ", "e2")));
        assert!(!support_confirmed(
            &p,
            &c("Apple forecast revenue of $4.2 billion", "e2")
        ));
        assert!(!support_confirmed(
            &p,
            &c("third quarter revenue of $4.2 billion", "e1")
        ));
        for space in ["\u{2028}", "\u{a0}", "\u{3000}"] {
            let s = format!("third quarter{space}revenue of $4.2 billion");
            assert!(support_confirmed(&p, &c(&s, "e2")), "{s:?}");
        }
    }

    #[tokio::test]
    async fn no_insights_means_no_call() {
        assert!(judge(&Panics, &[pack(TEXT)], &[]).await.unwrap().is_empty());
        // An insight without a pack is not sent either.
        assert!(judge(&Panics, &[pack(TEXT)], &[insight("NVDA")])
            .await
            .unwrap()
            .is_empty());
        let v = judge(
            &Canned(json!({"verdicts": [reply_item("AAPL")]})),
            &[pack(TEXT)],
            &[insight("AAPL")],
        )
        .await
        .unwrap();
        assert_eq!(v.len(), 1);
    }

    /// Round 2, L3: a line naming the day of the move passes when the
    /// judge quotes the move, which now holds the date.
    #[test]
    fn a_line_may_name_the_day_of_the_move() {
        let mut p = pack(TEXT);
        p.evidence[0].text =
            "Apple (AAPL) rose 4.20% in premarket trading on November 12, 2026.".into();
        let mut i = insight("AAPL");
        i.en.watch = "Reuters reports a $1.0 billion buyback after the November 12 move.".into();
        i.zh.watch = "据路透社报道，公司宣布10亿美元回购，股价于11月12日上涨。".into();
        let mut v = verdict();
        let moved = claim(
            "the move",
            "e1",
            "rose 4.20% in premarket trading on November 12, 2026",
        );
        v.en_claims.push(moved.clone());
        v.zh_claims.push(moved);
        assert!(passed(&p, &i, &v));
        v.en_claims.pop();
        assert!(!passed(&p, &i, &v));
    }

    /// A stored trial record: its "post ticker", pack, lines and each
    /// judge's verdict.
    type RunRecord = (
        String,
        Pack,
        Insight,
        std::collections::BTreeMap<String, Option<Verdict>>,
    );

    /// Records from trial run 2 (2026-11-13), as stored: pack, lines and
    /// each judge's verdict.
    fn run_two() -> Vec<RunRecord> {
        include_str!("testdata/run2_records.jsonl")
            .lines()
            .map(|l| {
                let v: Value = serde_json::from_str(l).unwrap();
                let verdicts = v["judges"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(name, j)| {
                        let mut verdict: Option<Verdict> =
                            serde_json::from_value(j["verdict"].clone()).unwrap();
                        // Run 2 predates the personal-allegation flag.
                        if let Some(v) = verdict.as_mut() {
                            v.no_personal_allegation = true;
                        }
                        (name.clone(), verdict)
                    })
                    .collect();
                (
                    format!(
                        "{} {}",
                        v["post"].as_str().unwrap(),
                        v["ticker"].as_str().unwrap()
                    ),
                    serde_json::from_value(v["pack"].clone()).unwrap(),
                    serde_json::from_value(v["insight"].clone()).unwrap(),
                    verdicts,
                )
            })
            .collect()
    }

    /// Trial run 2: true lines whose every flag the judge set true, failed
    /// by the confirmation code. Attribution listed as a claim (FICO, NRG),
    /// the digits of "24/7 Wall St." (SKYD), a span that leaves out the
    /// item's period (AMD), and "6座" for "six" (NRG) now pass.
    #[test]
    fn the_trial_run_two_verdicts_are_confirmed() {
        let records = run_two();
        let get = |key: &str, judge: &str| {
            let (_, p, i, vs) = records
                .iter()
                .find(|r| r.0 == key)
                .unwrap_or_else(|| panic!("{key}"));
            (p.clone(), i.clone(), vs[judge].clone().unwrap())
        };
        for (key, judge) in [
            ("premarket_2026-11-05 FICO", "deepseek"),
            ("premarket_2026-11-09-update NRG", "deepseek"),
            ("premarket_2026-11-09-update NRG", "claude"),
            ("recap_2026-11-10 SKYD", "claude"),
            ("recap_2026-11-10 SKYD", "deepseek"),
            ("recap_2026-11-06 AMD", "deepseek"),
        ] {
            let (p, i, v) = get(key, judge);
            assert!(passed(&p, &i, &v), "{key} {judge}");
        }
        // The attribution claim is ignored, not failed.
        let (p, _, v) = get("premarket_2026-11-05 FICO", "deepseek");
        let attribution = v
            .en_claims
            .iter()
            .find(|c| c.support == "StockStory")
            .unwrap();
        assert!(attribution_claim(&p, attribution));
        assert!(support_confirmed(&p, std::slice::from_ref(attribution)));
        // A verdict of attribution claims only has no claim of fact.
        let mut only = v.clone();
        only.en_claims.retain(|c| attribution_claim(&p, c));
        assert!(!passed(
            &p,
            &get("premarket_2026-11-05 FICO", "deepseek").1,
            &only
        ));
        // A number in no item a claim names still fails: AMD's line with a
        // figure the guidance item never states.
        let (p, mut i, v) = get("recap_2026-11-06 AMD", "deepseek");
        i.en.watch = i.en.watch.replace("$17 billion", "$19 billion");
        assert!(!passed(&p, &i, &v));
        // A judge "no" stays a no: Claude's on FICO (it said nothing linked
        // the margin to the drop) and on the affair line.
        let (p, i, v) = get("premarket_2026-11-05 FICO", "claude");
        assert!(!passed(&p, &i, &v));
        let (p, i, mut v) = get("premarket_2026-11-12-update NVDA", "deepseek");
        assert!(passed(&p, &i, &v));
        v.no_personal_allegation = false;
        assert!(!passed(&p, &i, &v));
    }

    #[test]
    fn the_v2_prompts_carry_the_run_two_rules() {
        let (system, user) = prompt(&[pack(TEXT)], &[insight("AAPL")]);
        for want in [
            "do not list it as a claim",
            "reason for the move",
            "no_personal_allegation",
            "英伟达预计于11月17日发布财报",
        ] {
            assert!(system.contains(want), "{want}");
        }
        let v: Value = serde_json::from_str(&user).unwrap();
        assert_eq!(v["movers"][0]["evidence"][1]["role"], "reason for the move");
        assert!(v["movers"][0]["evidence"][0].get("role").is_none());
        // A missing flag reads as "no".
        let v = parse(
            &json!({"verdicts": [{"ticker": "AAPL", "no_advice": true}]}),
            &[insight("AAPL")],
        );
        assert!(!v[0].no_personal_allegation);
    }

    /// Final run (2026-11-13): DeepSeek supported NRG's "The sale price is
    /// $4B" with "for $4B", exact but under the minimum. A short span is
    /// accepted when it holds a figure or date the claim states.
    #[test]
    fn a_short_exact_span_with_the_claims_figure_confirms() {
        let p = pack("after saying it agreed to sell six natural gas generation assets to independent power company LS Power for $4B");
        assert!(support_confirmed(
            &p,
            &[claim("The sale price is $4B", "e2", "for $4B")]
        ));
        assert!(support_confirmed(
            &p,
            &[claim("出售价格为40亿美元", "e2", "for $4B")]
        ));
        // Still verbatim in the named item, and still the claim's figure.
        assert!(!support_confirmed(
            &p,
            &[claim("The sale price is $4B", "e1", "for $4B")]
        ));
        assert!(!support_confirmed(
            &p,
            &[claim("The sale price is $5B", "e2", "for $4B")]
        ));
        // A short span with no figure stays refused.
        assert!(!support_confirmed(
            &p,
            &[claim("NRG is selling assets", "e2", "to sell")]
        ));
        // A short date span the claim states.
        let q = pack("Apple will list on the Nasdaq on November 19");
        assert!(support_confirmed(
            &q,
            &[claim("It lists on November 19", "e2", "November 19")]
        ));
    }
}
