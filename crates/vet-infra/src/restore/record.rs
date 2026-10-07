//! What a restore of a kept tree did, written to `meta/restore-<language>.json`
//! once it is done: the versions of the tools, each step's argv, environment and
//! outcome, and whether a donor tree supplied the result. A crash before the
//! write leaves no record, so the next use restores again.
//!
//! The record is reused only if the restore it would make now has the same
//! `inputs`. Its [`fingerprint`](Record::fingerprint) hashes all of it, outcomes
//! included, and is what keys answers that depend on the restored dependencies.

use std::path::Path;

use serde_json::{Value, json};
use vet_core::content::Hash;

/// Bumped when the record's shape changes: older records are then ignored.
const FORMAT: u64 = 1;

/// A hash over fields, each length-prefixed so that no two field lists give one input.
#[derive(Default)]
pub(super) struct Digest(Vec<u8>);

impl Digest {
    pub(super) fn field(&mut self, text: &str) -> &mut Self {
        self.0.extend((text.len() as u64).to_le_bytes());
        self.0.extend(text.as_bytes());
        self
    }

    pub(super) fn finish(&self) -> Hash {
        Hash::of(&self.0)
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) enum Outcome {
    Ran,
    /// Another tree's `node_modules`, linked in; `inputs` is what of the donor that depends on.
    Linked {
        donor: String,
        inputs: Hash,
    },
    /// The restore output of another tree, copied.
    Copied,
    /// Failed, or not run because an earlier step of the project failed.
    Failed(String),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct StepRecord {
    pub project: String,
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub outcome: Outcome,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub(super) struct Record {
    /// What the restore depends on: see `ToolRestorer::inputs`.
    pub inputs: Hash,
    /// `(program, version line)` of the tools the steps run.
    pub tools: Vec<(String, String)>,
    pub steps: Vec<StepRecord>,
}

impl Record {
    pub(super) fn fingerprint(&self) -> Hash {
        let mut digest = Digest::default();
        digest.field(&self.inputs.to_hex());
        for (program, version) in &self.tools {
            digest.field(program).field(version);
        }
        for step in &self.steps {
            digest.field(&step.project).field(&step.program);
            step.args.iter().for_each(|a| {
                digest.field(a);
            });
            step.env.iter().for_each(|(k, v)| {
                digest.field(k).field(v);
            });
            match &step.outcome {
                Outcome::Ran => digest.field("ran"),
                Outcome::Linked { donor, inputs } => digest.field("linked").field(donor).field(&inputs.to_hex()),
                Outcome::Copied => digest.field("copied"),
                Outcome::Failed(message) => digest.field("failed").field(message),
            };
        }
        digest.finish()
    }

    /// Whether every step succeeded: a record of a failure is kept for its
    /// fingerprint, but the restore runs again, since the failure may have been transient.
    pub(super) fn succeeded(&self) -> bool {
        self.steps.iter().all(|s| !matches!(s.outcome, Outcome::Failed(_)))
    }

    pub(super) fn read(path: &Path) -> Option<Self> {
        let value: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        if value.get("format")?.as_u64()? != FORMAT {
            return None;
        }
        let strings = |v: &Value| -> Option<Vec<String>> {
            v.as_array()?.iter().map(|s| s.as_str().map(str::to_owned)).collect()
        };
        let pairs = |v: &Value| -> Option<Vec<(String, String)>> {
            v.as_array()?
                .iter()
                .map(|p| {
                    let p = strings(p)?;
                    let [k, v] = <[String; 2]>::try_from(p).ok()?;
                    Some((k, v))
                })
                .collect()
        };
        let text = |v: &Value, key: &str| v.get(key)?.as_str().map(str::to_owned);
        let steps = value
            .get("steps")?
            .as_array()?
            .iter()
            .map(|step| {
                let outcome = step.get("outcome")?;
                let outcome = match outcome.get("kind")?.as_str()? {
                    "ran" => Outcome::Ran,
                    "copied" => Outcome::Copied,
                    "linked" => Outcome::Linked {
                        donor: text(outcome, "donor")?,
                        inputs: Hash::from_hex(&text(outcome, "inputs")?)?,
                    },
                    "failed" => Outcome::Failed(text(outcome, "message")?),
                    _ => return None,
                };
                Some(StepRecord {
                    project: text(step, "project")?,
                    program: text(step, "program")?,
                    args: strings(step.get("args")?)?,
                    env: pairs(step.get("env")?)?,
                    outcome,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            inputs: Hash::from_hex(&text(&value, "inputs")?)?,
            tools: pairs(value.get("tools")?)?,
            steps,
        })
    }

    /// Writes the record in one step: a reader sees the old one or the new one.
    pub(super) fn write(&self, path: &Path) -> std::io::Result<()> {
        let outcome = |o: &Outcome| match o {
            Outcome::Ran => json!({"kind": "ran"}),
            Outcome::Copied => json!({"kind": "copied"}),
            Outcome::Linked { donor, inputs } => json!({"kind": "linked", "donor": donor, "inputs": inputs.to_hex()}),
            Outcome::Failed(message) => json!({"kind": "failed", "message": message}),
        };
        let value = json!({
            "format": FORMAT,
            "inputs": self.inputs.to_hex(),
            "tools": self.tools.iter().map(|(k, v)| [k, v]).collect::<Vec<_>>(),
            "steps": self.steps.iter().map(|s| json!({
                "project": s.project,
                "program": s.program,
                "args": s.args,
                "env": s.env.iter().map(|(k, v)| [k, v]).collect::<Vec<_>>(),
                "outcome": outcome(&s.outcome),
            })).collect::<Vec<_>>(),
        });
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_string_pretty(&value)?)?;
        std::fs::rename(temporary, path)
    }
}
