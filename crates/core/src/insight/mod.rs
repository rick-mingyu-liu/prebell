//! Mover insights: a short hook and one "watch next" sentence per big mover,
//! in English and Chinese, written by a model from an evidence pack, checked
//! by code (`check`) and then by a judge model that must quote verbatim
//! support (`judge`). Pure logic: nothing in the Worker calls it yet.
//! Spec: `docs/superpowers/specs/2026-11-13-mover-insights-design.md`.

use serde::{Deserialize, Serialize};

pub mod check;
pub mod judge;
pub mod pack;
pub mod write;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Move,
    News,
    AnalystAction,
    CompanyFigure,
    CompanyGuidance,
    Theme,
    ReportDate,
    MacroEvent,
}

/// One item the writer and the judge may rest a line on.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Evidence {
    /// "e1", "e2", ... unique within a pack.
    pub id: String,
    pub kind: EvidenceKind,
    /// What the model reads; verbatim source text for `News`,
    /// `AnalystAction` and the `Company*` kinds.
    pub text: String,
    /// English outlet name, `News` and `AnalystAction` only.
    pub outlet: Option<String>,
    pub url: Option<String>,
    pub date: Option<chrono::NaiveDate>,
}

/// Everything the writer and the judge see for one mover.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pack {
    pub ticker: String,
    pub name: String,
    pub evidence: Vec<Evidence>,
}

/// One language's hook and watch line, with the evidence ids it rests on.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Line {
    pub hook: String,
    pub watch: String,
    pub cites: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Insight {
    pub ticker: String,
    pub en: Line,
    pub zh: Line,
}
