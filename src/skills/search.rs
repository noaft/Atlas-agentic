//! Local BM25 ranking. No model requests or remote search index.
use super::{SkillScope, SkillSearchResult, SkillStore};
use std::collections::{BTreeMap, BTreeSet};

fn tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .map(str::to_lowercase)
        .collect()
}

impl SkillStore {
    pub fn search(&self, query: &str, scope: SkillScope) -> Result<Vec<SkillSearchResult>, String> {
        if query.chars().count() > 512 {
            return Err("Skill search is limited to 512 characters.".into());
        }
        let terms: BTreeSet<_> = tokens(query).into_iter().take(32).collect();
        let catalog = self.list()?;
        let mut documents = Vec::new();
        for skill in catalog
            .skills
            .into_iter()
            .filter(|skill| skill.scope == scope)
        {
            let text = if terms.is_empty() {
                String::new()
            } else {
                let Ok(document) = self.read(&skill.id, None) else {
                    continue;
                };
                format!(
                    "{} {} {} {}",
                    skill.name, skill.title, skill.description, document.content
                )
            };
            let mut counts = BTreeMap::<String, usize>::new();
            let words = tokens(&text);
            let length = words.len();
            for term in words {
                *counts.entry(term).or_default() += 1;
            }
            documents.push((skill, counts, length));
        }
        if documents.is_empty() {
            return Ok(Vec::new());
        }
        let count = documents.len() as f64;
        let average_length =
            (documents.iter().map(|(_, _, len)| *len).sum::<usize>() as f64 / count).max(1.0);
        let frequencies: BTreeMap<_, _> = terms
            .iter()
            .map(|term| {
                (
                    term,
                    documents
                        .iter()
                        .filter(|(_, counts, _)| counts.contains_key(term))
                        .count() as f64,
                )
            })
            .collect();
        let mut results = Vec::new();
        for (skill, counts, length) in documents {
            let mut score = 0.0;
            for term in &terms {
                let frequency = *counts.get(term).unwrap_or(&0) as f64;
                if frequency == 0.0 {
                    continue;
                }
                let document_frequency = frequencies[term];
                let idf =
                    (1.0 + (count - document_frequency + 0.5) / (document_frequency + 0.5)).ln();
                score += idf * (frequency * 2.2)
                    / (frequency + 1.2 * (0.25 + 0.75 * length as f64 / average_length));
            }
            if terms.is_empty() || score > 0.0 {
                results.push(SkillSearchResult { skill, score });
            }
        }
        results.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.skill.id.cmp(&right.skill.id))
        });
        Ok(results)
    }
}
