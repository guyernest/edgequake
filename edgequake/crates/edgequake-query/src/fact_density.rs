//! Fact density analysis for answer quality scoring.
//!
//! Provides regex-based extraction of proper nouns, dates, and numbers
//! from text. Used by `AnswerQuality::compute` to assess the factual
//! density of generated answers and by the `/text/score` endpoint
//! for standalone text analysis.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// Result of fact density analysis on a piece of text.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FactDensityResult {
    /// Total count of facts detected (proper_nouns + dates + numbers).
    pub fact_density: u32,
    /// Proper noun phrases detected (Title Case and ALL CAPS).
    pub proper_nouns: Vec<String>,
    /// Date expressions detected.
    pub dates: Vec<String>,
    /// Numeric expressions detected (including currency and percentages).
    pub numbers: Vec<String>,
}

/// Regex-based analyzer that detects proper nouns, dates, and numbers in text.
pub struct FactDensityAnalyzer;

// --- Compiled regex patterns (lazy-initialized singletons) ---

/// Title Case proper nouns: "Jeffrey Epstein", "New York", "Deutsche Bank"
static RE_TITLE_CASE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b[A-Z][a-z]{2,}(?:\s+[A-Z][a-z]+)*\b").unwrap()
});

/// ALL CAPS proper nouns: "JEFFREY", "EPSTEIN", "FBI" (3+ chars)
static RE_ALL_CAPS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b[A-Z]{3,}\b").unwrap()
});

/// ISO dates: 2005-03-15, 03/15/2005, 3-15-05
static RE_DATE_NUMERIC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b\d{4}[-/]\d{1,2}[-/]\d{1,2}\b|\b\d{1,2}[-/]\d{1,2}[-/]\d{2,4}\b").unwrap()
});

/// Named month dates: "March 15, 2005", "January 1", "December 25, 2023"
static RE_DATE_NAMED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b(?:January|February|March|April|May|June|July|August|September|October|November|December)\s+\d{1,2}(?:,?\s+\d{4})?\b",
    )
    .unwrap()
});

/// Currency: "$2.5 million", "$100", "$1,234.56"
static RE_CURRENCY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\d[\d,]*(?:\.\d+)?(?:\s*(?:million|billion|trillion|thousand))?").unwrap()
});

/// Percentages: "25%", "3.14%", "100%"
static RE_PERCENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b\d[\d,]*(?:\.\d+)?%").unwrap()
});

/// Standalone numbers (digits, possibly with commas/decimals): "1234", "1,000", "3.14"
/// Excludes numbers already captured by currency or percentage patterns.
static RE_STANDALONE_NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b\d[\d,]*(?:\.\d+)?\b").unwrap()
});

/// Characters that indicate a sentence boundary (preceding a match start).
fn is_sentence_starter(text: &str, match_start: usize) -> bool {
    if match_start == 0 {
        return true;
    }

    // Walk backwards from match_start to find the first non-whitespace char
    let before = &text[..match_start];
    let trimmed = before.trim_end();
    if trimmed.is_empty() {
        return true;
    }

    let last_char = trimmed.chars().last().unwrap();
    matches!(last_char, '.' | '!' | '?' | '\n')
}

impl FactDensityAnalyzer {
    /// Analyze text and extract proper nouns, dates, and numbers.
    pub fn analyze(text: &str) -> FactDensityResult {
        if text.is_empty() {
            return FactDensityResult {
                fact_density: 0,
                proper_nouns: Vec::new(),
                dates: Vec::new(),
                numbers: Vec::new(),
            };
        }

        let proper_nouns = Self::extract_proper_nouns(text);
        let dates = Self::extract_dates(text);
        let numbers = Self::extract_numbers(text, &dates);

        let fact_density = (proper_nouns.len() + dates.len() + numbers.len()) as u32;

        FactDensityResult {
            fact_density,
            proper_nouns,
            dates,
            numbers,
        }
    }

    /// Extract proper nouns (Title Case and ALL CAPS), filtering sentence starters.
    fn extract_proper_nouns(text: &str) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut result = Vec::new();

        // Title Case matches
        for m in RE_TITLE_CASE.find_iter(text) {
            if is_sentence_starter(text, m.start()) {
                continue;
            }
            let value = m.as_str().to_string();
            if seen.insert(value.clone()) {
                result.push(value);
            }
        }

        // ALL CAPS matches
        for m in RE_ALL_CAPS.find_iter(text) {
            let value = m.as_str().to_string();
            if seen.insert(value.clone()) {
                result.push(value);
            }
        }

        result
    }

    /// Extract date expressions.
    fn extract_dates(text: &str) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut result = Vec::new();

        for re in [&*RE_DATE_NAMED, &*RE_DATE_NUMERIC] {
            for m in re.find_iter(text) {
                let value = m.as_str().to_string();
                if seen.insert(value.clone()) {
                    result.push(value);
                }
            }
        }

        result
    }

    /// Extract numeric expressions (currency, percentages, standalone numbers).
    /// Excludes numbers that are part of date expressions.
    fn extract_numbers(text: &str, dates: &[String]) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut result = Vec::new();

        // Collect character positions covered by date matches to avoid double-counting
        let mut date_positions: HashSet<usize> = HashSet::new();
        for date_str in dates {
            // Find all occurrences of this date in the text
            let mut search_start = 0;
            while let Some(pos) = text[search_start..].find(date_str) {
                let abs_pos = search_start + pos;
                for i in abs_pos..abs_pos + date_str.len() {
                    date_positions.insert(i);
                }
                search_start = abs_pos + 1;
            }
        }

        // Currency
        for m in RE_CURRENCY.find_iter(text) {
            let value = m.as_str().to_string();
            if seen.insert(value.clone()) {
                result.push(value);
            }
        }

        // Percentages
        for m in RE_PERCENT.find_iter(text) {
            let value = m.as_str().to_string();
            if seen.insert(value.clone()) {
                result.push(value);
            }
        }

        // Standalone numbers (skip those inside dates or already captured)
        for m in RE_STANDALONE_NUMBER.find_iter(text) {
            // Skip if this number overlaps with a date position
            if date_positions.contains(&m.start()) {
                continue;
            }
            // Skip if preceded by '$' (already captured as currency)
            if m.start() > 0 && text.as_bytes()[m.start() - 1] == b'$' {
                continue;
            }
            // Skip if followed by '%' (already captured as percentage)
            if m.end() < text.len() && text.as_bytes()[m.end()] == b'%' {
                continue;
            }
            let value = m.as_str().to_string();
            if seen.insert(value.clone()) {
                result.push(value);
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fact_density_full_sentence() {
        // Note: "Jeffrey Epstein" is at sentence start so gets filtered as sentence starter.
        // Mid-sentence proper nouns like "Deutsche Bank" and "New York" are detected.
        let result = FactDensityAnalyzer::analyze(
            "Records show Jeffrey Epstein met with Deutsche Bank in New York on March 15, 2005 for $2.5 million",
        );

        assert!(
            result.proper_nouns.iter().any(|n| n.contains("Epstein")),
            "Should detect 'Jeffrey Epstein', got: {:?}",
            result.proper_nouns
        );
        assert!(
            result.proper_nouns.iter().any(|n| n.contains("Deutsche")),
            "Should detect 'Deutsche Bank', got: {:?}",
            result.proper_nouns
        );
        assert!(
            result.dates.iter().any(|d| d.contains("March 15, 2005")),
            "Should detect 'March 15, 2005', got: {:?}",
            result.dates
        );
        assert!(
            result.numbers.iter().any(|n| n.contains("2.5")),
            "Should detect '$2.5 million', got: {:?}",
            result.numbers
        );
        assert!(
            result.fact_density >= 4,
            "Expected fact_density >= 4, got {}",
            result.fact_density
        );
    }

    #[test]
    fn test_fact_density_empty_text() {
        let result = FactDensityAnalyzer::analyze("");
        assert_eq!(result.fact_density, 0);
        assert!(result.proper_nouns.is_empty());
        assert!(result.dates.is_empty());
        assert!(result.numbers.is_empty());
    }

    #[test]
    fn test_fact_density_all_caps_detection() {
        let result = FactDensityAnalyzer::analyze("JEFFREY EPSTEIN and GHISLAINE MAXWELL");
        assert!(
            result.proper_nouns.len() >= 2,
            "Should detect at least JEFFREY and EPSTEIN as ALL CAPS proper nouns, got: {:?}",
            result.proper_nouns
        );
    }

    #[test]
    fn test_fact_density_sentence_starter_filtered() {
        // "The" at start of sentence should not be a proper noun
        let result = FactDensityAnalyzer::analyze("The quick brown fox. The lazy dog.");
        // "The" matches title case but should be filtered as sentence starter
        assert!(
            !result.proper_nouns.iter().any(|n| n == "The"),
            "Sentence-starting 'The' should be filtered, got: {:?}",
            result.proper_nouns
        );
    }

    #[test]
    fn test_fact_density_mid_sentence_proper_noun() {
        let result = FactDensityAnalyzer::analyze("I met Jeffrey Epstein in New York last week.");
        assert!(
            result.proper_nouns.iter().any(|n| n.contains("Jeffrey")),
            "Mid-sentence proper nouns should be detected, got: {:?}",
            result.proper_nouns
        );
    }

    #[test]
    fn test_fact_density_date_formats() {
        let result = FactDensityAnalyzer::analyze("Events on 2005-03-15 and January 1, 2020 and 12/25/2023");
        assert!(
            result.dates.len() >= 3,
            "Should detect all three date formats, got: {:?}",
            result.dates
        );
    }

    #[test]
    fn test_fact_density_number_formats() {
        let result = FactDensityAnalyzer::analyze("He paid $100 and earned 25% return on 1,000 shares.");
        assert!(
            result.numbers.iter().any(|n| n.contains("100")),
            "Should detect $100, got: {:?}",
            result.numbers
        );
        assert!(
            result.numbers.iter().any(|n| n.contains("25%")),
            "Should detect 25%, got: {:?}",
            result.numbers
        );
    }
}
