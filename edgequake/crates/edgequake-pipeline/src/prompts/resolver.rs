//! Post-extraction entity resolution.
//!
//! Rule-based resolver that canonicalizes entity names after LLM extraction.
//! Handles duplicates caused by:
//! - Middle initials (DONALD_J._TRUMP → DONALD_TRUMP)
//! - Legal numbering (JANE_DOE_NO._102 → JANE_DOE)
//! - Common suffixes (MAR-A-LAGO_CLUB → MAR-A-LAGO)
//! - Known aliases (VIRGINIA_ROBERTS → VIRGINIA_GIUFFRE)
//! - Fuzzy string matches (Jaro-Winkler similarity)

use std::collections::HashMap;

use crate::extractor::{ExtractedEntity, ExtractedRelationship, ExtractionResult};

/// Configuration for entity resolution.
#[derive(Debug, Clone)]
pub struct EntityResolutionConfig {
    /// Whether entity resolution is enabled.
    pub enabled: bool,
    /// Jaro-Winkler similarity threshold for fuzzy matching (0.0-1.0).
    pub string_similarity_threshold: f64,
    /// Maximum candidates to consider for fuzzy matching per entity.
    pub max_candidates: usize,
}

impl Default for EntityResolutionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            string_similarity_threshold: 0.85,
            max_candidates: 50,
        }
    }
}

/// Post-extraction entity resolver.
///
/// Resolves entity names to canonical forms using rule-based heuristics.
/// No LLM calls needed — pure string manipulation and similarity matching.
#[derive(Debug, Clone)]
pub struct EntityResolver {
    config: EntityResolutionConfig,
    /// Known alias → canonical name mappings.
    alias_map: HashMap<String, String>,
}

impl Default for EntityResolver {
    fn default() -> Self {
        Self::new(EntityResolutionConfig::default())
    }
}

impl EntityResolver {
    /// Create a new resolver with the given config.
    pub fn new(config: EntityResolutionConfig) -> Self {
        Self {
            config,
            alias_map: HashMap::new(),
        }
    }

    /// Add a known alias mapping (alias → canonical).
    pub fn add_alias(&mut self, alias: impl Into<String>, canonical: impl Into<String>) {
        self.alias_map
            .insert(alias.into().to_uppercase(), canonical.into().to_uppercase());
    }

    /// Add multiple alias mappings at once.
    pub fn add_aliases(&mut self, aliases: impl IntoIterator<Item = (String, String)>) {
        for (alias, canonical) in aliases {
            self.add_alias(alias, canonical);
        }
    }

    /// Create a resolver pre-loaded with Epstein-domain aliases.
    pub fn with_epstein_aliases(config: EntityResolutionConfig) -> Self {
        let mut resolver = Self::new(config);
        // Known aliases from the Epstein dataset
        let aliases = [
            // Virginia Giuffre variants
            ("VIRGINIA_ROBERTS", "VIRGINIA_GIUFFRE"),
            ("VIRGINIA_ROBERTS_GIUFFRE", "VIRGINIA_GIUFFRE"),
            ("VIRGINIA_L._GIUFFRE", "VIRGINIA_GIUFFRE"),
            ("VIRGINIA_L._ROBERTS", "VIRGINIA_GIUFFRE"),
            ("MS._GIUFFRE", "VIRGINIA_GIUFFRE"),
            ("MS._ROBERTS", "VIRGINIA_GIUFFRE"),
            // Donald Trump variants
            ("DONALD_J._TRUMP", "DONALD_TRUMP"),
            ("DONALD_J_TRUMP", "DONALD_TRUMP"),
            ("PRESIDENT_TRUMP", "DONALD_TRUMP"),
            ("TRUMP", "DONALD_TRUMP"),
            // Jeffrey Epstein variants
            ("J._EPSTEIN", "JEFFREY_EPSTEIN"),
            ("J_EPSTEIN", "JEFFREY_EPSTEIN"),
            ("EPSTEIN", "JEFFREY_EPSTEIN"),
            ("MR._EPSTEIN", "JEFFREY_EPSTEIN"),
            // Ghislaine Maxwell variants
            ("G._MAXWELL", "GHISLAINE_MAXWELL"),
            ("MS._MAXWELL", "GHISLAINE_MAXWELL"),
            ("MAXWELL", "GHISLAINE_MAXWELL"),
            // Bill Clinton variants
            ("WILLIAM_CLINTON", "BILL_CLINTON"),
            ("WILLIAM_J._CLINTON", "BILL_CLINTON"),
            ("WILLIAM_JEFFERSON_CLINTON", "BILL_CLINTON"),
            ("PRESIDENT_CLINTON", "BILL_CLINTON"),
            ("CLINTON", "BILL_CLINTON"),
            // Prince Andrew variants
            ("PRINCE_ANDREW", "ANDREW_DUKE_OF_YORK"),
            ("ANDREW_WINDSOR", "ANDREW_DUKE_OF_YORK"),
            ("DUKE_OF_YORK", "ANDREW_DUKE_OF_YORK"),
            ("THE_DUKE_OF_YORK", "ANDREW_DUKE_OF_YORK"),
            // Alan Dershowitz variants
            ("ALAN_M._DERSHOWITZ", "ALAN_DERSHOWITZ"),
            ("PROFESSOR_DERSHOWITZ", "ALAN_DERSHOWITZ"),
            ("DERSHOWITZ", "ALAN_DERSHOWITZ"),
            // Les Wexner variants
            ("LESLIE_WEXNER", "LES_WEXNER"),
            ("LESLIE_H._WEXNER", "LES_WEXNER"),
            ("L._WEXNER", "LES_WEXNER"),
            ("WEXNER", "LES_WEXNER"),
            // Jean-Luc Brunel variants
            ("JEAN_LUC_BRUNEL", "JEAN-LUC_BRUNEL"),
            ("BRUNEL", "JEAN-LUC_BRUNEL"),
            ("J.L._BRUNEL", "JEAN-LUC_BRUNEL"),
            ("JL_BRUNEL", "JEAN-LUC_BRUNEL"),
            // Sarah Kellen variants
            ("SARAH_KELLEN_VICKERS", "SARAH_KELLEN"),
            ("KELLEN", "SARAH_KELLEN"),
            // Nadia Marcinkova variants
            ("NADIA_MARCINKO", "NADIA_MARCINKOVA"),
            ("NADA_MARCINKOVA", "NADIA_MARCINKOVA"),
            // Location aliases
            ("MAR-A-LAGO_CLUB", "MAR-A-LAGO"),
            ("MAR-A-LAGO_RESORT", "MAR-A-LAGO"),
            ("MAR-A-LAGO_ESTATE", "MAR-A-LAGO"),
            ("LITTLE_SAINT_JAMES", "LITTLE_ST._JAMES"),
            ("LITTLE_ST_JAMES", "LITTLE_ST._JAMES"),
            ("LITTLE_SAINT_JAMES_ISLAND", "LITTLE_ST._JAMES"),
            ("LITTLE_ST._JAMES_ISLAND", "LITTLE_ST._JAMES"),
            ("ZORRO_RANCH", "ZORRO_RANCH"),
            // Organization aliases
            ("EPSTEIN_FOUNDATION", "JEFFREY_EPSTEIN_FOUNDATION"),
            ("J._EPSTEIN_&_COMPANY", "J._EPSTEIN_&_CO."),
            ("J._EPSTEIN_AND_COMPANY", "J._EPSTEIN_&_CO."),
            ("THE_WEXNER_FOUNDATION", "WEXNER_FOUNDATION"),
            // FBI / DOJ
            ("FEDERAL_BUREAU_OF_INVESTIGATION", "FBI"),
            ("F.B.I.", "FBI"),
            ("DEPARTMENT_OF_JUSTICE", "DOJ"),
            ("DEPT._OF_JUSTICE", "DOJ"),
            ("U.S._DEPARTMENT_OF_JUSTICE", "DOJ"),
            // SDNY
            ("SOUTHERN_DISTRICT_OF_NEW_YORK", "SDNY"),
            ("S.D.N.Y.", "SDNY"),
            ("U.S._ATTORNEY'S_OFFICE_SDNY", "SDNY"),
            // Palm Beach
            ("PALM_BEACH_POLICE", "PALM_BEACH_POLICE_DEPARTMENT"),
            ("PBPD", "PALM_BEACH_POLICE_DEPARTMENT"),
        ];
        for (alias, canonical) in aliases {
            resolver.add_alias(alias, canonical);
        }
        resolver
    }

    /// Resolve a single entity name to its canonical form.
    ///
    /// Applies resolution rules in order:
    /// 1. Alias map lookup
    /// 2. Strip middle initials (e.g., _J._ or _J_)
    /// 3. Strip legal numbering (e.g., _NO._102)
    /// 4. Strip common suffixes (_CLUB, _PROPERTY, etc.)
    pub fn resolve_name(&self, name: &str) -> String {
        let upper = name.to_uppercase();

        // 1. Check alias map first (exact match)
        if let Some(canonical) = self.alias_map.get(&upper) {
            return canonical.clone();
        }

        let mut resolved = upper;

        // 2. Strip middle initials: _J._ or _J_ (single letter between underscores)
        let re_middle_initial =
            regex::Regex::new(r"_[A-Z]\.?_").expect("valid regex for middle initials");
        resolved = re_middle_initial.replace_all(&resolved, "_").to_string();

        // 3. Strip legal numbering: _NO._102 or _NO_102 or _NUMBER_3
        let re_legal_number =
            regex::Regex::new(r"_NO\.?_?\d+$").expect("valid regex for legal numbering");
        resolved = re_legal_number.replace(&resolved, "").to_string();

        // 4. Strip common suffixes that create duplicates
        let suffixes = ["_CLUB", "_PROPERTY", "_RESIDENCE", "_MANSION", "_ESTATE"];
        for suffix in &suffixes {
            if resolved.ends_with(suffix) && resolved.len() > suffix.len() {
                resolved = resolved[..resolved.len() - suffix.len()].to_string();
                break; // Only strip one suffix
            }
        }

        // Check alias map again after normalization
        if let Some(canonical) = self.alias_map.get(&resolved) {
            return canonical.clone();
        }

        resolved
    }

    /// Resolve all entities and relationships in an ExtractionResult.
    ///
    /// - Resolves entity names
    /// - Resolves relationship source/target names
    /// - Deduplicates entities that resolve to the same canonical name
    ///   (merges descriptions with "Also known as" notation)
    pub fn resolve_extraction(&self, mut result: ExtractionResult) -> ExtractionResult {
        if !self.config.enabled {
            return result;
        }

        // Build name resolution map: original → canonical
        let mut name_map: HashMap<String, String> = HashMap::new();
        for entity in &result.entities {
            let canonical = self.resolve_name(&entity.name);
            if canonical != entity.name {
                name_map.insert(entity.name.clone(), canonical);
            }
        }

        // Apply fuzzy matching for remaining entities
        let resolved_names: Vec<String> = result
            .entities
            .iter()
            .map(|e| name_map.get(&e.name).cloned().unwrap_or_else(|| e.name.clone()))
            .collect();
        self.apply_fuzzy_matching(&resolved_names, &mut name_map);

        // Resolve entity names and deduplicate
        let mut canonical_entities: HashMap<String, ExtractedEntity> = HashMap::new();
        for mut entity in result.entities.drain(..) {
            let canonical_name = name_map
                .get(&entity.name)
                .cloned()
                .unwrap_or_else(|| entity.name.clone());

            let original_name = entity.name.clone();
            entity.name = canonical_name.clone();

            if let Some(existing) = canonical_entities.get_mut(&canonical_name) {
                // Merge: append alias info and combine descriptions
                if original_name != canonical_name
                    && !existing.description.contains(&original_name)
                {
                    if !existing.description.contains("Also known as:") {
                        existing.description =
                            format!("{} Also known as: {}", existing.description, original_name);
                    } else {
                        existing.description =
                            format!("{}, {}", existing.description, original_name);
                    }
                }
                // Merge source chunk IDs
                for chunk_id in &entity.source_chunk_ids {
                    if !existing.source_chunk_ids.contains(chunk_id) {
                        existing.source_chunk_ids.push(chunk_id.clone());
                    }
                }
            } else {
                // Add alias note if name was changed
                if original_name != canonical_name
                    && !entity.description.contains(&original_name)
                {
                    entity.description = format!(
                        "{} Also known as: {}",
                        entity.description, original_name
                    );
                }
                canonical_entities.insert(canonical_name, entity);
            }
        }

        result.entities = canonical_entities.into_values().collect();

        // Resolve relationship endpoints
        for rel in &mut result.relationships {
            if let Some(canonical) = name_map.get(&rel.source) {
                rel.source = canonical.clone();
            }
            if let Some(canonical) = name_map.get(&rel.target) {
                rel.target = canonical.clone();
            }
        }

        // Remove self-referencing relationships that may have been created by resolution
        result
            .relationships
            .retain(|rel| rel.source != rel.target);

        // Deduplicate relationships (same source+target pair)
        let mut seen_rels: HashMap<(String, String), usize> = HashMap::new();
        let mut deduped_rels: Vec<ExtractedRelationship> = Vec::new();
        for rel in result.relationships.drain(..) {
            let key = if rel.source < rel.target {
                (rel.source.clone(), rel.target.clone())
            } else {
                (rel.target.clone(), rel.source.clone())
            };

            if let Some(&idx) = seen_rels.get(&key) {
                // Merge: append keywords from duplicate
                let existing = &mut deduped_rels[idx];
                for kw in &rel.keywords {
                    if !existing.keywords.contains(kw) && existing.keywords.len() < 5 {
                        existing.keywords.push(kw.clone());
                    }
                }
            } else {
                seen_rels.insert(key, deduped_rels.len());
                deduped_rels.push(rel);
            }
        }
        result.relationships = deduped_rels;

        result
    }

    /// Apply Jaro-Winkler fuzzy matching to find additional duplicates.
    fn apply_fuzzy_matching(
        &self,
        resolved_names: &[String],
        name_map: &mut HashMap<String, String>,
    ) {
        if self.config.string_similarity_threshold >= 1.0 {
            return; // Disabled
        }

        let unique_names: Vec<&String> = resolved_names
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        // Compare pairs (O(n^2) but capped by max_candidates)
        let limit = unique_names.len().min(self.config.max_candidates);
        for i in 0..limit {
            for j in (i + 1)..limit {
                let a = unique_names[i];
                let b = unique_names[j];

                // Skip if already mapped to same canonical
                let canonical_a = name_map.get(a).unwrap_or(a);
                let canonical_b = name_map.get(b).unwrap_or(b);
                if canonical_a == canonical_b {
                    continue;
                }

                let similarity = strsim::jaro_winkler(a, b);
                if similarity >= self.config.string_similarity_threshold {
                    // Map the shorter name to the longer (more complete) name
                    let (alias, canonical) = if a.len() >= b.len() { (b, a) } else { (a, b) };
                    name_map.insert(alias.clone(), canonical.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entity(name: &str, entity_type: &str, desc: &str) -> ExtractedEntity {
        ExtractedEntity::new(name, entity_type, desc)
    }

    fn make_result(
        entities: Vec<ExtractedEntity>,
        relationships: Vec<ExtractedRelationship>,
    ) -> ExtractionResult {
        let mut result = ExtractionResult::new("test-chunk");
        result.entities = entities;
        result.relationships = relationships;
        result
    }

    // =========================================================================
    // Alias map tests
    // =========================================================================

    #[test]
    fn test_alias_map_resolution() {
        let resolver = EntityResolver::with_epstein_aliases(EntityResolutionConfig::default());

        assert_eq!(
            resolver.resolve_name("VIRGINIA_ROBERTS"),
            "VIRGINIA_GIUFFRE"
        );
        assert_eq!(resolver.resolve_name("DONALD_J._TRUMP"), "DONALD_TRUMP");
        assert_eq!(resolver.resolve_name("MAR-A-LAGO_CLUB"), "MAR-A-LAGO");
    }

    #[test]
    fn test_expanded_alias_map() {
        let resolver = EntityResolver::with_epstein_aliases(EntityResolutionConfig::default());

        // Clinton variants
        assert_eq!(resolver.resolve_name("WILLIAM_CLINTON"), "BILL_CLINTON");
        assert_eq!(
            resolver.resolve_name("WILLIAM_J._CLINTON"),
            "BILL_CLINTON"
        );
        assert_eq!(resolver.resolve_name("PRESIDENT_CLINTON"), "BILL_CLINTON");

        // Prince Andrew variants
        assert_eq!(
            resolver.resolve_name("PRINCE_ANDREW"),
            "ANDREW_DUKE_OF_YORK"
        );
        assert_eq!(
            resolver.resolve_name("DUKE_OF_YORK"),
            "ANDREW_DUKE_OF_YORK"
        );

        // Dershowitz variants
        assert_eq!(
            resolver.resolve_name("ALAN_M._DERSHOWITZ"),
            "ALAN_DERSHOWITZ"
        );
        assert_eq!(
            resolver.resolve_name("PROFESSOR_DERSHOWITZ"),
            "ALAN_DERSHOWITZ"
        );

        // Wexner variants
        assert_eq!(resolver.resolve_name("LESLIE_WEXNER"), "LES_WEXNER");
        assert_eq!(resolver.resolve_name("LESLIE_H._WEXNER"), "LES_WEXNER");

        // Brunel variants
        assert_eq!(
            resolver.resolve_name("JEAN_LUC_BRUNEL"),
            "JEAN-LUC_BRUNEL"
        );

        // Kellen/Marcinkova
        assert_eq!(
            resolver.resolve_name("SARAH_KELLEN_VICKERS"),
            "SARAH_KELLEN"
        );
        assert_eq!(
            resolver.resolve_name("NADIA_MARCINKO"),
            "NADIA_MARCINKOVA"
        );

        // Maxwell solo
        assert_eq!(resolver.resolve_name("MAXWELL"), "GHISLAINE_MAXWELL");

        // Agencies
        assert_eq!(
            resolver.resolve_name("FEDERAL_BUREAU_OF_INVESTIGATION"),
            "FBI"
        );
        assert_eq!(
            resolver.resolve_name("DEPARTMENT_OF_JUSTICE"),
            "DOJ"
        );
        assert_eq!(
            resolver.resolve_name("SOUTHERN_DISTRICT_OF_NEW_YORK"),
            "SDNY"
        );
    }

    // =========================================================================
    // Middle initial stripping
    // =========================================================================

    #[test]
    fn test_strip_middle_initials() {
        let resolver = EntityResolver::new(EntityResolutionConfig::default());

        assert_eq!(resolver.resolve_name("DONALD_J._TRUMP"), "DONALD_TRUMP");
        assert_eq!(resolver.resolve_name("DONALD_J_TRUMP"), "DONALD_TRUMP");
        assert_eq!(resolver.resolve_name("JOHN_F._KENNEDY"), "JOHN_KENNEDY");
    }

    #[test]
    fn test_no_false_positive_middle_initial() {
        let resolver = EntityResolver::new(EntityResolutionConfig::default());

        // Two-letter segments should NOT be stripped
        assert_eq!(resolver.resolve_name("NEW_YORK"), "NEW_YORK");
        // Name without middle initial stays the same
        assert_eq!(resolver.resolve_name("JEFFREY_EPSTEIN"), "JEFFREY_EPSTEIN");
    }

    // =========================================================================
    // Legal numbering stripping
    // =========================================================================

    #[test]
    fn test_strip_legal_numbering() {
        let resolver = EntityResolver::new(EntityResolutionConfig::default());

        assert_eq!(resolver.resolve_name("JANE_DOE_NO._102"), "JANE_DOE");
        assert_eq!(resolver.resolve_name("JANE_DOE_NO_3"), "JANE_DOE");
        assert_eq!(resolver.resolve_name("JOHN_DOE_NO._1"), "JOHN_DOE");
    }

    // =========================================================================
    // Suffix stripping
    // =========================================================================

    #[test]
    fn test_strip_common_suffixes() {
        let resolver = EntityResolver::new(EntityResolutionConfig::default());

        assert_eq!(resolver.resolve_name("MAR-A-LAGO_CLUB"), "MAR-A-LAGO");
        assert_eq!(
            resolver.resolve_name("PALM_BEACH_MANSION"),
            "PALM_BEACH"
        );
        assert_eq!(
            resolver.resolve_name("TOWNHOUSE_PROPERTY"),
            "TOWNHOUSE"
        );
        assert_eq!(
            resolver.resolve_name("ZORRO_RANCH_ESTATE"),
            "ZORRO_RANCH"
        );
    }

    #[test]
    fn test_suffix_not_stripped_if_nothing_left() {
        let resolver = EntityResolver::new(EntityResolutionConfig::default());

        // "CLUB" alone should not be stripped to empty
        assert_eq!(resolver.resolve_name("CLUB"), "CLUB");
    }

    // =========================================================================
    // Full extraction resolution
    // =========================================================================

    #[test]
    fn test_resolve_extraction_deduplicates_entities() {
        let resolver = EntityResolver::with_epstein_aliases(EntityResolutionConfig {
            string_similarity_threshold: 1.0, // Disable fuzzy for this test
            ..Default::default()
        });

        let result = make_result(
            vec![
                make_entity("VIRGINIA_ROBERTS", "PERSON", "A victim"),
                make_entity("VIRGINIA_GIUFFRE", "PERSON", "A witness"),
            ],
            vec![ExtractedRelationship::new(
                "VIRGINIA_ROBERTS",
                "JEFFREY_EPSTEIN",
                "ACCUSED",
            )],
        );

        let resolved = resolver.resolve_extraction(result);

        // Should merge into one entity
        assert_eq!(resolved.entities.len(), 1);
        assert_eq!(resolved.entities[0].name, "VIRGINIA_GIUFFRE");
        assert!(resolved.entities[0]
            .description
            .contains("VIRGINIA_ROBERTS"));

        // Relationship should point to canonical name
        assert_eq!(resolved.relationships[0].source, "VIRGINIA_GIUFFRE");
    }

    #[test]
    fn test_resolve_extraction_removes_self_refs_after_resolution() {
        let resolver = EntityResolver::with_epstein_aliases(EntityResolutionConfig {
            string_similarity_threshold: 1.0,
            ..Default::default()
        });

        let result = make_result(
            vec![
                make_entity("DONALD_J._TRUMP", "PERSON", "A president"),
                make_entity("DONALD_TRUMP", "PERSON", "A businessman"),
            ],
            vec![
                ExtractedRelationship::new("DONALD_J._TRUMP", "DONALD_TRUMP", "SAME_AS"),
                ExtractedRelationship::new("DONALD_TRUMP", "JEFFREY_EPSTEIN", "KNOWS"),
            ],
        );

        let resolved = resolver.resolve_extraction(result);

        // Self-referencing relationship should be removed
        assert_eq!(resolved.relationships.len(), 1);
        assert_eq!(resolved.relationships[0].relation_type, "KNOWS");
    }

    #[test]
    fn test_resolve_extraction_disabled() {
        let resolver = EntityResolver::new(EntityResolutionConfig {
            enabled: false,
            ..Default::default()
        });

        let result = make_result(
            vec![
                make_entity("VIRGINIA_ROBERTS", "PERSON", "A victim"),
                make_entity("VIRGINIA_GIUFFRE", "PERSON", "A witness"),
            ],
            vec![],
        );

        let resolved = resolver.resolve_extraction(result);

        // Should NOT resolve — both entities remain
        assert_eq!(resolved.entities.len(), 2);
    }

    // =========================================================================
    // Fuzzy matching tests
    // =========================================================================

    #[test]
    fn test_fuzzy_matching() {
        let resolver = EntityResolver::new(EntityResolutionConfig {
            string_similarity_threshold: 0.85,
            ..Default::default()
        });

        // These are very similar strings that should match
        let result = make_result(
            vec![
                make_entity("JEFFREY_EPSTEIN", "PERSON", "Financier"),
                make_entity("JEFFERY_EPSTEIN", "PERSON", "A financier"), // typo
            ],
            vec![],
        );

        let resolved = resolver.resolve_extraction(result);
        assert_eq!(resolved.entities.len(), 1);
    }

    #[test]
    fn test_fuzzy_no_false_positives() {
        let resolver = EntityResolver::new(EntityResolutionConfig {
            string_similarity_threshold: 0.85,
            ..Default::default()
        });

        let result = make_result(
            vec![
                make_entity("GHISLAINE_MAXWELL", "PERSON", "An associate"),
                make_entity("ROBERT_MAXWELL", "PERSON", "A publisher"),
            ],
            vec![],
        );

        let resolved = resolver.resolve_extraction(result);
        // These should NOT be merged — different people with same surname
        assert_eq!(resolved.entities.len(), 2);
    }
}
