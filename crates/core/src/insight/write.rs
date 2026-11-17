//! The writer: one model call for every eligible mover of a post, in both
//! languages, and the parse of its reply.

use serde_json::{json, Value};

use super::check::{HOOK_CHARS_ZH, HOOK_WORDS_EN, WATCH_CHARS_ZH, WATCH_WORDS_EN};
use super::{EvidenceKind, Insight, Line, Pack};
use crate::providers::{LlmProvider, ProviderError};

pub const WRITER_VERSION: &str = "insight-writer/v6";

/// What a news or analyst-action item is to the brief: the reason it gives
/// for the move.
pub(crate) const REASON_ROLE: &str = "reason for the move";

/// The packs as the models read them: no URLs; news and analyst-action
/// items carry `"role": "reason for the move"`.
pub(crate) fn movers_json(packs: &[Pack]) -> Vec<Value> {
    packs
        .iter()
        .map(|p| {
            json!({
                "ticker": p.ticker,
                "name": p.name,
                "evidence": p.evidence.iter().map(|e| {
                    let mut item = json!({
                        "id": e.id,
                        "kind": e.kind,
                        "text": e.text,
                        "outlet": e.outlet,
                        "date": e.date.map(|d| d.to_string()),
                    });
                    if matches!(e.kind, EvidenceKind::News | EvidenceKind::AnalystAction) {
                        item["role"] = json!(REASON_ROLE);
                    }
                    item
                }).collect::<Vec<_>>(),
            })
        })
        .collect()
}

/// (system, user) for one writer call over `packs`.
pub fn prompt(packs: &[Pack]) -> (String, String) {
    let system = format!(
        r#"You write PreBell's short reading of big stock movers in a daily US market brief. For each mover you write a hook (a few words) and one sentence on what to track next for this company and why it matters. The reading is PreBell's own, in a formal, neutral register.

What to track:
- It must be specific to this company and come from its evidence: a figure or target the company gave, a stated plan or timetable, a problem the evidence names, or its report date. Never a market-wide or macro event.
- Say what would confirm or change the picture. Restating the result and adding "is worth tracking" is not an insight. In this spirit, about invented companies (do not copy): "…so whether Halvorsen Mills' freight costs keep outgrowing its sales is what decides the margin"; "…and Tarrow Bio's stated plan to start its Phase 3 trial in 2031 is the date that would test it".
- If the evidence offers nothing of that kind, leave the mover out.
- Vary how sentences close: across one reply, never use the same closing phrase twice, and end on the specific thing (the figure, the plan, the date) rather than a stock phrase such as "is the point to track". Keep it formal and one sentence. These invented lines only illustrate variety; never copy their wording or assume what they assume: "…, which puts Kestrel Foods' $910 million to $930 million sales range under test at its next report"; "…, and a return of Orvane Systems' gross margin to the 58.1% it reported would show the cost rise was temporary"; "…, so the opening of Brightwell Rail's second depot, which it set for 2031, is the milestone to watch".
- Each "news" or "analyst_action" item has "role": "reason for the move": PreBell's brief gives it as a reason the stock moved, so a hook may tie the move to it ("Margin contraction behind the drop").

People:
- A claim about a named person, including an allegation, may be repeated only when the sentence names the outlet that reported it, in a reporting frame ("Benzinga reports …", "据Benzinga报道……"), and never as PreBell's own claim. Naming an executive for what they said in their role (a CFO's guidance, a CEO's statement about preorders) is fine.

Rules:
1. Use only the evidence given for that mover. Never use what you remember about the company, the market or the news.
2. Every number and every date you write must appear in an evidence item you cite, as the same figure and precision. Write figures in digits, never in words ("10%", not "ten percent"; "70%", not "七十" or "百分之七十"). In Chinese, convert the unit naturally with 万 and 亿: "$4.2 billion" is "42亿美元" and "$79.677 million" is "7967.7万美元", never "79.677百万美元". Write a date as a calendar date from the evidence ("November 19", "11月19日"), never a weekday, a bare day or a month alone. The hook has no numbers at all.
3. A fact from a "news" or "analyst_action" item is attributed in the sentence to that item's "outlet", written exactly as given, in both languages ("Seeking Alpha reports ...", "据Seeking Alpha报道……"; for Reuters, "据路透社报道" is also fine). An item whose "outlet" is null cannot be cited. A fact from a "company_figure" or "company_guidance" item is stated as the company's ("the company guided ...", "公司预计……").
4. An analyst action may be mentioned (a named firm upgraded, downgraded or initiated coverage), but never its rating word, a price target or any figure from it, and never "analysts expect", analysts' or the Street's estimates, or the consensus ("分析师预期", "一致预期"). The thing to track must come from another item: a company figure, a stated plan or a dated event.
5. No advice of any kind (buy, sell, hold, should, recommend, position, exposure, 建议, 买入, 仓位, 配置 and the like), no prediction of where a price goes (upside, could climb, 有望上涨), no rating words. No relative time words ("next week", "this quarter", "soon", "today", "下周", "明年", "近期"): name the date from the evidence instead. A fiscal label ("Q3", "fiscal 2027", "第三季度") only when a cited item states it.
6. A report date marked "Expected report date, not confirmed by the company" is not the company's own date and must be hedged in the same clause, briefly so the line stays within its limit: in English "expected on November 17 (date not confirmed by the company)"; in Chinese never make the company the subject of 预计 or 预期 ("英伟达预计于11月17日发布财报" is wrong), write "财报日期暂定为11月17日（未经公司确认）" or "市场预计其于11月17日发布财报（未经公司确认）".
7. Limits: the English hook at most {HOOK_WORDS_EN} words, the English sentence one sentence of at most {WATCH_WORDS_EN} words; the Chinese hook at most {HOOK_CHARS_ZH} characters, the Chinese sentence one sentence of at most {WATCH_CHARS_ZH} characters. One clause chain: no semicolons. No Markdown, links or line breaks.
8. The Chinese hook and sentence say the same thing as the English ones, in natural simplified Chinese. Each language lists the ids of the evidence items its line rests on in "cites".
9. Leave a mover out when its evidence gives nothing concrete to track.

Reply with JSON only, in this shape:
{{"insights":[{{"ticker":"...","en":{{"hook":"...","watch":"...","cites":["e2"]}},"zh":{{"hook":"...","watch":"...","cites":["e2"]}}}}]}}"#
    );
    let user = json!({ "movers": movers_json(packs) }).to_string();
    (system, user)
}

fn line(v: Option<&Value>) -> Option<Line> {
    let v = v?;
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let cites = v
        .get("cites")?
        .as_array()?
        .iter()
        .map(|c| c.as_str().map(str::to_string))
        .collect::<Option<Vec<_>>>()?;
    Some(Line {
        hook: text("hook")?,
        watch: text("watch")?,
        cites,
    })
}

/// One entry per usable reply item; items for unknown or repeated tickers,
/// or with a missing field, become Err(reason).
pub fn parse(reply: &Value, packs: &[Pack]) -> Vec<Result<Insight, String>> {
    let Some(items) = reply.get("insights").and_then(Value::as_array) else {
        return vec![Err("the reply has no \"insights\" list".into())];
    };
    let mut seen: Vec<&str> = Vec::new();
    items
        .iter()
        .map(|it| {
            let t = it
                .get("ticker")
                .and_then(Value::as_str)
                .ok_or("an item without a ticker")?;
            let pack = packs
                .iter()
                .find(|p| p.ticker.eq_ignore_ascii_case(t.trim()))
                .ok_or(format!("{t}: not one of the movers"))?;
            // Errors name the mover as its pack does ("NVDA", not "nvda").
            let t = pack.ticker.as_str();
            if seen.contains(&t) {
                return Err(format!("{t}: repeated"));
            }
            seen.push(t);
            let en = line(it.get("en")).ok_or(format!("{t}: missing or incomplete en"))?;
            let zh = line(it.get("zh")).ok_or(format!("{t}: missing or incomplete zh"))?;
            Ok(Insight {
                ticker: pack.ticker.clone(),
                en,
                zh,
            })
        })
        .collect()
}

/// One writer call over `packs`, no retry. No packs, no call.
pub async fn write(
    llm: &dyn LlmProvider,
    packs: &[Pack],
) -> Result<Vec<Result<Insight, String>>, ProviderError> {
    if packs.is_empty() {
        return Ok(vec![]);
    }
    let (system, user) = prompt(packs);
    let reply = llm.complete_json(&system, &user).await?;
    Ok(parse(&reply, packs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insight::{Evidence, EvidenceKind};
    use async_trait::async_trait;
    use std::cell::Cell;

    fn pack(ticker: &str) -> Pack {
        Pack {
            ticker: ticker.into(),
            name: format!("{ticker} Inc"),
            evidence: vec![
                Evidence {
                    id: "e1".into(),
                    kind: EvidenceKind::Move,
                    text: format!("{ticker} Inc ({ticker}) rose 4.20% in premarket trading."),
                    outlet: None,
                    url: None,
                    date: Some("2026-11-12".parse().unwrap()),
                    quote: None,
                    unconfirmed: false,
                },
                Evidence {
                    id: "e2".into(),
                    kind: EvidenceKind::News,
                    text: "The company expects revenue of $4.2 billion.".into(),
                    outlet: Some("Reuters".into()),
                    url: Some("https://www.reuters.com/secret-path".into()),
                    date: None,
                    quote: None,
                    unconfirmed: false,
                },
            ],
        }
    }

    fn item(ticker: &str) -> Value {
        json!({"ticker": ticker,
            "en": {"hook": "Guidance sets the test", "watch": "Reuters reports ...", "cites": ["e2"]},
            "zh": {"hook": "指引成为考验", "watch": "据路透社报道……", "cites": ["e2"]}})
    }

    struct Canned {
        reply: Value,
        calls: Cell<usize>,
    }
    #[async_trait(?Send)]
    impl LlmProvider for Canned {
        async fn complete_json(&self, _: &str, _: &str) -> Result<Value, ProviderError> {
            self.calls.set(self.calls.get() + 1);
            Ok(self.reply.clone())
        }
    }

    struct Panics;
    #[async_trait(?Send)]
    impl LlmProvider for Panics {
        async fn complete_json(&self, _: &str, _: &str) -> Result<Value, ProviderError> {
            panic!("no call expected")
        }
    }

    #[test]
    fn the_prompt_carries_every_pack_and_no_urls() {
        let (_, user) = prompt(&[pack("AAPL"), pack("NVDA")]);
        let v: Value = serde_json::from_str(&user).unwrap();
        let movers = v["movers"].as_array().unwrap();
        assert_eq!(movers.len(), 2);
        assert_eq!(movers[0]["ticker"], "AAPL");
        assert_eq!(movers[1]["ticker"], "NVDA");
        assert_eq!(movers[0]["name"], "AAPL Inc");
        let ev = movers[0]["evidence"].as_array().unwrap();
        assert_eq!(ev[0]["id"], "e1");
        assert_eq!(ev[0]["kind"], "move");
        assert_eq!(ev[0]["date"], "2026-11-12");
        assert_eq!(ev[1]["id"], "e2");
        assert_eq!(ev[1]["kind"], "news");
        assert_eq!(ev[1]["outlet"], "Reuters");
        assert!(ev[1].get("url").is_none());
        assert!(!user.contains("http"), "{user}");
    }

    #[test]
    fn the_system_prompt_states_the_limits() {
        let (system, _) = prompt(&[pack("AAPL")]);
        for want in [
            "8 words",
            "40 words",
            "20 characters",
            "100 characters",
            "JSON only",
            "outlet",
        ] {
            assert!(system.contains(want), "{want}");
        }
    }

    #[test]
    fn parse_keeps_good_items() {
        let packs = [pack("AAPL"), pack("NVDA")];
        let r = parse(&json!({"insights": [item("AAPL"), item("nvda")]}), &packs);
        assert_eq!(r.len(), 2);
        let a = r[0].as_ref().unwrap();
        assert_eq!(a.ticker, "AAPL");
        assert_eq!(a.en.hook, "Guidance sets the test");
        assert_eq!(a.zh.cites, ["e2"]);
        assert_eq!(r[1].as_ref().unwrap().ticker, "NVDA");
    }

    #[test]
    fn parse_rejects_unknown_and_repeated_tickers() {
        let packs = [pack("AAPL"), pack("NVDA")];
        let r = parse(
            &json!({"insights": [item("AAPL"), item("TSLA"), item("AAPL"), item("NVDA")]}),
            &packs,
        );
        assert_eq!(r.len(), 4);
        assert_eq!(r[0].as_ref().unwrap().ticker, "AAPL");
        assert!(r[1].as_ref().unwrap_err().contains("TSLA"));
        assert!(r[2].as_ref().unwrap_err().contains("repeated"));
        assert_eq!(r[3].as_ref().unwrap().ticker, "NVDA");
    }

    #[test]
    fn errors_name_the_mover_as_its_pack_does() {
        // Review L5: "nvda" in the reply is NVDA's mover.
        let packs = [pack("AAPL"), pack("NVDA")];
        let mut no_zh = item("nvda");
        no_zh.as_object_mut().unwrap().remove("zh");
        let r = parse(&json!({"insights": [no_zh, item("NVDA")]}), &packs);
        assert_eq!(r[0].as_ref().unwrap_err(), "NVDA: missing or incomplete zh");
        assert_eq!(r[1].as_ref().unwrap_err(), "NVDA: repeated");
    }

    #[test]
    fn the_people_rule_is_the_owners() {
        // Rick, 2026-11-17: "Allow it if attributed". The lead's earlier ban
        // on naming a person in connection with an allegation is gone.
        let (system, _) = prompt(&[pack("AAPL")]);
        assert!(!system.contains("Never repeat an allegation"), "{system}");
        assert!(!system.contains("never name a person in connection"));
        assert!(system.contains("including an allegation"));
    }

    #[test]
    fn the_prompt_states_the_review_rules() {
        let (system, _) = prompt(&[pack("AAPL")]);
        for want in [
            "42亿美元",
            "never in words",
            "no semicolons",
            "Expected report date, not confirmed by the company",
            "never make the company the subject of 预计",
            "never use the same closing phrase twice",
            "expected on November 17 (date not confirmed by the company)",
            "财报日期暂定为11月17日（未经公司确认）",
            "reason for the move",
            "only when the sentence names the outlet that reported it",
            "never as PreBell's own claim",
            "分析师预期",
            "never its rating word, a price target or any figure from it",
        ] {
            assert!(system.contains(want), "{want}");
        }
    }

    #[test]
    fn parse_rejects_a_missing_language() {
        let packs = [pack("AAPL"), pack("NVDA")];
        let mut no_zh = item("AAPL");
        no_zh.as_object_mut().unwrap().remove("zh");
        let mut no_cites = item("NVDA");
        no_cites["en"].as_object_mut().unwrap().remove("cites");
        let r = parse(&json!({"insights": [no_zh, no_cites, {"en": {}}]}), &packs);
        assert!(r[0].as_ref().unwrap_err().contains("zh"));
        assert!(r[1].as_ref().unwrap_err().contains("en"));
        assert!(r[2].as_ref().unwrap_err().contains("ticker"));
    }

    #[test]
    fn an_omitted_mover_is_not_an_error() {
        let packs = [pack("AAPL"), pack("NVDA")];
        let r = parse(&json!({"insights": [item("NVDA")]}), &packs);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].as_ref().unwrap().ticker, "NVDA");
        assert!(parse(&json!({"insights": []}), &packs).is_empty());
    }

    #[test]
    fn a_non_object_reply_is_one_error() {
        let packs = [pack("AAPL")];
        for reply in [json!([item("AAPL")]), json!("text"), json!({"lines": []})] {
            let r = parse(&reply, &packs);
            assert_eq!(r.len(), 1, "{reply}");
            assert!(r[0].is_err());
        }
    }

    #[tokio::test]
    async fn no_packs_means_no_call() {
        assert!(write(&Panics, &[]).await.unwrap().is_empty());
        let llm = Canned {
            reply: json!({"insights": [item("AAPL")]}),
            calls: Cell::new(0),
        };
        let r = write(&llm, &[pack("AAPL")]).await.unwrap();
        assert_eq!(llm.calls.get(), 1);
        assert!(r[0].is_ok());
    }

    /// Round 4, M4: the prompt's examples are invented, never the trial's
    /// own data (v4 used AXON's range, RKLB's Neutron and FICO's margin, and
    /// RKLB's "any further slip" followed from it).
    #[test]
    fn the_examples_are_invented() {
        let (system, _) = prompt(&[pack("AAPL")]);
        for trial in [
            "Neutron",
            "47.2",
            "3.61",
            "3.63",
            "slip",
            "Axon",
            "Rocket Lab",
            "Fair Isaac",
        ] {
            assert!(!system.contains(trial), "{trial}");
        }
        for invented in [
            "Halvorsen Mills",
            "Tarrow Bio",
            "Kestrel Foods",
            "Orvane Systems",
            "Brightwell Rail",
        ] {
            assert!(system.contains(invented), "{invented}");
        }
        assert_eq!(WRITER_VERSION, "insight-writer/v6");
    }
}
