"""Summarise llm-eval results.jsonl files into the report's markdown tables.

    python3 analyze.py [--facts DIR] RESULTS.jsonl [RESULTS.jsonl ...]

Every file is one trial; a draft (post x model x language) appears once per trial.
With --facts (the saved bundles, named <post>.json), every number the verifier
rejected is traced back to the bundle's text: a number that matches a figure
anywhere in the bundle (a note's quote, a summary, a scale the whitelist does
not carry) is a verifier false positive, not a model error.
"""

import json
import os
import re
import statistics as st
import sys
from collections import Counter, defaultdict

MODELS = ["deepseek-chat", "deepseek-flash", "claude-sonnet-5-5", "claude-opus-5-5"]
# Production volume (a month of ~21 trading days, ~4.33 weeks).
BRIEFS_PER_MONTH = 3 * 21
EARNINGS_PER_MONTH = (20 * 4.33, 40 * 4.33)


def category(outcome: str) -> str:
    o = outcome
    if o == "ok":
        return "ok"
    if "budget cap" in o:
        return "budget stop"
    if o.startswith("The previous request failed") or o.startswith("llm:"):
        if "truncated" in o:
            return "provider: truncated"
        return "provider: " + o.split(":", 1)[1].strip()[:40]
    if "compare results with estimates" in o:
        return "estimate/expectation wording"
    if "mention missing figures" in o:
        return "mentions a missing figure"
    if o.startswith("These numbers are not in the facts") or not (
        o.startswith("Your reply was invalid") or o.startswith("draft:")
    ):
        return "number not in facts"
    if "heading" in o or "section" in o:
        return "sections/headings"
    return "format: " + o.split(":", 1)[-1].strip()[:50]


UNIT = {"万亿": 1e12, "亿": 1e8, "万": 1e4, "trillion": 1e12, "billion": 1e9, "million": 1e6, "thousand": 1e3}
NUM = re.compile(r"-?\d[\d,]*(?:\.\d+)?")


def value(token):
    """`$2,272 million` -> 2.272e9, `232 万` -> 2.32e6, `10%` -> 10."""
    m = NUM.search(token)
    if not m:
        return None
    v = float(m.group().replace(",", ""))
    rest = token[m.end():].strip().lower()
    for u, k in UNIT.items():
        if rest.startswith(u):
            return v * k
    return v


def fact_values(path):
    text = open(path).read()
    out = set()
    for m in NUM.finditer(text):
        try:
            v = float(m.group().replace(",", ""))
        except ValueError:
            continue
        for k in (1, 1e3, 1e6, 1e9, 1e12):
            out.add(abs(v) * k)
    return out


def traced(token, facts):
    v = value(token)
    if v is None or v == 0:
        return False
    v = abs(v)
    return any(abs(f - v) <= 0.006 * v for f in facts if f)


def rejected_numbers(outcome):
    o = outcome
    if o.startswith("These numbers are not in the facts:"):
        o = o.split(":", 1)[1]
    elif o.startswith(("Your reply", "The previous", "draft:", "llm:")) or o == "ok":
        return []
    return [t.strip() for t in re.split(r"[;,]\s(?=\D|\$|\d)", o) if t.strip()]


def pct(n, d):
    return f"{100 * n / d:.0f}% ({n}/{d})" if d else "-"


def main(paths):
    rows = []
    for trial, p in enumerate(paths):
        for line in open(p):
            r = json.loads(line)
            r["trial"] = trial
            rows.append(r)
    by_model = defaultdict(list)
    for r in rows:
        by_model[r["model"]].append(r)
    models = [m for m in MODELS if m in by_model] + sorted(set(by_model) - set(MODELS))

    print("## Results\n")
    print("| | " + " | ".join(models) + " |")
    print("|---|" + "---|" * len(models))

    def row(label, f):
        print(f"| {label} | " + " | ".join(f(by_model[m]) for m in models) + " |")

    def fp(rs, kind=None, lang=None):
        rs = [r for r in rs if (kind is None or r["kind"] == kind) and (lang is None or r["lang"] == lang)]
        return pct(sum(r["first_pass"] for r in rs), len(rs))

    row("Drafts", lambda rs: str(len(rs)))
    row("First-pass rate, all", lambda rs: fp(rs))
    row("First-pass, EN", lambda rs: fp(rs, lang="en"))
    row("First-pass, ZH", lambda rs: fp(rs, lang="zh"))
    row("First-pass, earnings", lambda rs: fp(rs, kind="earnings"))
    row("First-pass, briefs (premarket+recap)", lambda rs: fp([r for r in rs if r["kind"] != "earnings"]))

    def in_build(rs, lang=None, n=1):
        rs = [r for r in rs if lang is None or r["lang"] == lang]
        return pct(sum(r["published"] and r["builds"] <= n for r in rs), len(rs))

    row("Published in first build (<=2 attempts)", lambda rs: in_build(rs))
    row("ZH published in first build", lambda rs: in_build(rs, "zh"))
    row("Published within 3 builds", lambda rs: in_build(rs, n=3))
    row("Mean LLM calls per published draft", lambda rs: f"{st.mean([r['calls'] for r in rs if r['published']]):.2f}" if any(r["published"] for r in rs) else "-")
    row("Retries per published draft", lambda rs: f"{st.mean([r['calls'] - 1 for r in rs if r['published']]):.2f}" if any(r["published"] for r in rs) else "-")

    def calls(rs):
        return [a for r in rs for a in r["attempts"] if a["status"] == 200]

    row("Latency per call, median", lambda rs: f"{st.median([a['ms'] for a in calls(rs)]) / 1000:.1f} s")
    row("Latency per call, p90", lambda rs: f"{sorted(a['ms'] for a in calls(rs))[int(0.9 * (len(calls(rs)) - 1))] / 1000:.1f} s")
    row("Latency to publish a draft, mean", lambda rs: f"{st.mean([r['total_ms'] for r in rs if r['published']]) / 1000:.1f} s")
    row("Input tokens per call, mean", lambda rs: f"{st.mean([a['input_tokens'] for a in calls(rs)]):,.0f}")
    row("Output tokens per call, mean", lambda rs: f"{st.mean([a['output_tokens'] for a in calls(rs)]):,.0f}")
    row("  of which reasoning (DeepSeek)", lambda rs: f"{st.mean([a['reasoning_tokens'] for a in calls(rs)]):,.0f}")

    # Cost per post = EN + ZH drafts including retries, averaged per post.
    def per_post(rs, kinds, key="cost_usd"):
        posts = defaultdict(float)
        n = Counter()
        for r in rs:
            if r["kind"] in kinds:
                posts[(r["trial"], r["post"])] += r[key]
                n[(r["trial"], r["post"])] += 1
        vals = [v for k, v in posts.items() if n[k] == 2]
        return st.mean(vals) if vals else 0.0

    def uncached(rs, kinds):
        # DeepSeek: price the cached tokens as uncached, since production
        # hits its cache only on retries.
        total = defaultdict(float)
        for r in rs:
            if r["kind"] not in kinds:
                continue
            for a in r["attempts"]:
                c = a["cost_usd"]
                if r["model"].startswith("deepseek") and a["cached_input_tokens"]:
                    c += a["cached_input_tokens"] * (0.15 - 0.003) / 1e6
                total[(r["trial"], r["post"])] += c
        return st.mean(total.values()) if total else 0.0

    E, B = {"earnings"}, {"premarket", "recap"}
    row("Draft cost per earnings post (EN+ZH)", lambda rs: f"${uncached(rs, E):.4f}")
    row("Draft cost per brief (EN+ZH)", lambda rs: f"${uncached(rs, B):.4f}")

    def month(rs, lo=True):
        e = EARNINGS_PER_MONTH[0 if lo else 1]
        return uncached(rs, E) * e + uncached(rs, B) * BRIEFS_PER_MONTH

    row("Draft cost per month (20-40 earnings/wk + 3 briefs/day)", lambda rs: f"${month(rs):.2f}-${month(rs, False):.2f}")
    row("Eval spend (all trials)", lambda rs: f"${sum(r['cost_usd'] for r in rs):.3f}")

    print("\n## Why attempts failed (every failed attempt, all builds)\n")
    for m in models:
        c = Counter(category(a["outcome"]) for r in by_model[m] for a in r["attempts"] if a["outcome"] != "ok")
        first = Counter(category(r["attempts"][0]["outcome"]) for r in by_model[m] if r["attempts"] and not r["first_pass"])
        print(f"- **{m}**: first attempts failed: " + (", ".join(f"{k} {v}" for k, v in first.most_common()) or "none")
              + "; all failed attempts: " + (", ".join(f"{k} {v}" for k, v in c.most_common()) or "none"))

    if FACTS:
        print("\n## Rejected numbers traced to the bundle (first attempts)\n")
        print("A rejected number that matches a figure in the bundle (rounded within 0.6%, any scale) is a verifier false positive.\n")
        for m in models:
            t = Counter()
            examples = Counter()
            for r in by_model[m]:
                if r["first_pass"] or not r["attempts"]:
                    continue
                facts = fact_values(os.path.join(FACTS, r["post"] + ".json"))
                toks = rejected_numbers(r["attempts"][0]["outcome"])
                if not toks:
                    continue
                ok = all(traced(x, facts) for x in toks)
                t["all in facts" if ok else "some not in facts"] += 1
                for x in toks:
                    if not traced(x, facts):
                        examples[x] += 1
            print(f"- **{m}**: first attempts rejected for numbers: {dict(t)}; untraceable numbers: "
                  + ", ".join(f"{k}" + (f" x{v}" if v > 1 else "") for k, v in examples.most_common()))

        print("\n## Adjusted rates\n")
        print("Adjusted first-pass counts a first attempt whose only rejections were numbers traced to the bundle as passed."
              " Retry fixed: of first attempts that failed, how many the in-build retry (with the problems appended) fixed."
              " Prose-rule violations: first attempts rejected for estimate wording or for mentioning a missing figure.\n")
        for m in models:
            rs = by_model[m]

            def adj_ok(r):
                if r["first_pass"]:
                    return True
                toks = rejected_numbers(r["attempts"][0]["outcome"])
                facts = fact_values(os.path.join(FACTS, r["post"] + ".json"))
                return bool(toks) and all(traced(x, facts) for x in toks)

            zh = [r for r in rs if r["lang"] == "zh"]
            retried = [r for r in rs if not r["first_pass"] and len(r["attempts"]) > 1 and r["attempts"][1]["build"] == 1]
            rule = sum(category(r["attempts"][0]["outcome"]) in ("estimate/expectation wording", "mentions a missing figure")
                       for r in rs if not r["first_pass"] and r["attempts"])
            print(f"- **{m}**: adjusted first-pass {pct(sum(map(adj_ok, rs)), len(rs))}, ZH {pct(sum(map(adj_ok, zh)), len(zh))}; "
                  f"retry fixed {pct(sum(r['attempts'][1]['outcome'] == 'ok' for r in retried), len(retried))}; "
                  f"prose-rule violations {rule}")

    print("\n## Drafts not published within 3 builds\n")
    for r in rows:
        if not r["published"]:
            print(f"- {r['model']} {r['post']} {r['lang']} (trial {r['trial']}): {'; '.join(r['final_errors'])[:200]}")

    print("\n## Per draft (trial 0)\n")
    print("| post | lang | " + " | ".join(models) + " |")
    print("|---|---|" + "---|" * len(models))
    keyed = {(r["post"], r["lang"], r["model"]): r for r in rows if r["trial"] == 0}
    for post in sorted({r["post"] for r in rows}):
        for lang in ["en", "zh"]:
            cells = []
            for m in models:
                r = keyed.get((post, lang, m))
                if not r:
                    cells.append("-")
                    continue
                mark = "pass" if r["first_pass"] else (f"{r['calls']} calls" if r["published"] else "FAIL")
                cells.append(mark)
            print(f"| {post.split('_0')[0]} | {lang} | " + " | ".join(cells) + " |")


FACTS = None

if __name__ == "__main__":
    args = sys.argv[1:]
    if args[:1] == ["--facts"]:
        FACTS, args = args[1], args[2:]
    main(args)
