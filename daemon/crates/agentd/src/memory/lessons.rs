use rusqlite::{Connection, params};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LessonOutcome {
    Success,
    Failure,
    Partial,
}

impl LessonOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Partial => "partial",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "success" => Some(Self::Success),
            "failure" => Some(Self::Failure),
            "partial" => Some(Self::Partial),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DecayBand {
    Fresh,
    Week,
    Month,
}

#[derive(Debug, Clone)]
pub struct LessonWrite {
    pub decision: String,
    pub rationale: String,
    pub outcome: LessonOutcome,
    pub confidence_score: f64,
    pub pattern: String,
    pub agent_source: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LessonRecord {
    pub id: i64,
    pub decision: String,
    pub rationale: String,
    pub outcome: String,
    pub confidence_score: f64,
    pub effective_confidence: f64,
    pub pattern: String,
    pub agent_source: String,
    pub created_at: String,
}

pub fn insert_lesson(conn: &Connection, lesson: &LessonWrite) -> anyhow::Result<()> {
    conn.execute(
        "INSERT INTO lessons (decision, rationale, outcome, confidence_score, pattern, agent_source)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            lesson.decision,
            lesson.rationale,
            lesson.outcome.as_str(),
            lesson.confidence_score.clamp(0.0, 1.0),
            lesson.pattern,
            lesson.agent_source
        ],
    )?;
    Ok(())
}

pub fn query_lessons(
    conn: &Connection,
    outcome: Option<&str>,
    agent_source: Option<&str>,
    pattern_like: Option<&str>,
) -> anyhow::Result<Vec<LessonRecord>> {
    let mut stmt = conn.prepare(
        "SELECT id, decision, rationale, outcome, confidence_score, pattern, agent_source, created_at
         FROM lessons
         WHERE (?1 IS NULL OR outcome = ?1)
           AND (?2 IS NULL OR agent_source = ?2)
           AND (?3 IS NULL OR pattern LIKE '%' || ?3 || '%' OR decision LIKE '%' || ?3 || '%' OR rationale LIKE '%' || ?3 || '%')
         ORDER BY id DESC
         LIMIT 500",
    )?;

    let rows = stmt.query_map(params![outcome, agent_source, pattern_like], |row| {
        let created_at = row.get::<_, String>(7)?;
        let confidence_score = row.get::<_, f64>(4)?;
        let effective_conf = effective_confidence(confidence_score, &created_at);
        Ok(LessonRecord {
            id: row.get::<_, i64>(0)?,
            decision: row.get::<_, String>(1)?,
            rationale: row.get::<_, String>(2)?,
            outcome: row.get::<_, String>(3)?,
            confidence_score,
            effective_confidence: effective_conf,
            pattern: row.get::<_, String>(5)?,
            agent_source: row.get::<_, String>(6)?,
            created_at,
        })
    })?;

    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn effective_confidence(confidence_score: f64, created_at: &str) -> f64 {
    confidence_score * decay_multiplier(created_at)
}

pub fn decay_multiplier(created_at: &str) -> f64 {
    let parsed = chrono::NaiveDateTime::parse_from_str(created_at, "%Y-%m-%d %H:%M:%S");
    let Ok(created) = parsed else {
        return 1.0;
    };
    let now = chrono::Utc::now().naive_utc();
    let age_days = (now - created).num_days();
    match decay_band(age_days) {
        DecayBand::Fresh => 1.0,
        DecayBand::Week => 0.9,
        DecayBand::Month => 0.5,
    }
}

fn decay_band(age_days: i64) -> DecayBand {
    if age_days >= 30 {
        DecayBand::Month
    } else if age_days >= 7 {
        DecayBand::Week
    } else {
        DecayBand::Fresh
    }
}

pub fn extract_self_reported_lessons(content: &str, agent_source: &str) -> Vec<LessonWrite> {
    let mut out = Vec::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("# LESSON:") {
            continue;
        }
        let payload = trimmed.trim_start_matches("# LESSON:").trim();
        let mut outcome = LessonOutcome::Partial;
        let mut confidence = 0.8_f64;
        let mut pattern = String::from("general");
        let mut decision = String::new();
        let mut rationale = String::new();

        for pair in payload.split(';') {
            let mut parts = pair.splitn(2, '=');
            let key = parts.next().unwrap_or("").trim().to_ascii_lowercase();
            let value = parts.next().unwrap_or("").trim().to_string();
            match key.as_str() {
                "outcome" => {
                    if let Some(parsed) = LessonOutcome::parse(&value) {
                        outcome = parsed;
                    }
                }
                "confidence" => {
                    if let Ok(v) = value.parse::<f64>() {
                        confidence = v.clamp(0.0, 1.0);
                    }
                }
                "pattern" => pattern = value,
                "decision" => decision = value,
                "rationale" => rationale = value,
                _ => {}
            }
        }

        if !decision.is_empty() && !rationale.is_empty() {
            out.push(LessonWrite {
                decision,
                rationale,
                outcome,
                confidence_score: confidence,
                pattern,
                agent_source: agent_source.to_string(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{decay_multiplier, extract_self_reported_lessons};

    #[test]
    fn decays_confidence_after_7_days() {
        assert_eq!(decay_multiplier("2000-01-01 00:00:00"), 0.5);
    }

    #[test]
    fn parses_self_reported_lesson_line() {
        let content = "# LESSON: outcome=failure; confidence=0.7; pattern=auth; decision=jwt failed; rationale=token drift";
        let lessons = extract_self_reported_lessons(content, "claude");
        assert_eq!(lessons.len(), 1);
        assert_eq!(lessons[0].pattern, "auth");
    }
}
