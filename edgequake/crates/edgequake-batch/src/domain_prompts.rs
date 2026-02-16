//! Prompt assembly from domain configuration.
//!
//! Builds system and user prompts by combining structural framing (kept in code)
//! with domain-specific content (entity types, relationship keywords, examples)
//! from a [`DomainConfig`].
//!
//! When using `DomainConfig::builtin_epstein()`, the output is character-identical
//! to the original hardcoded `EpsteinExtractionPrompts`.

use crate::domain_config::DomainConfig;
use edgequake_pipeline::prompts::{DEFAULT_COMPLETION_DELIMITER, DEFAULT_TUPLE_DELIMITER};

/// Domain-configurable extraction prompts.
pub struct DomainExtractionPrompts {
    config: DomainConfig,
    tuple_delimiter: String,
    completion_delimiter: String,
}

impl DomainExtractionPrompts {
    pub fn new(config: DomainConfig) -> Self {
        Self {
            config,
            tuple_delimiter: DEFAULT_TUPLE_DELIMITER.to_string(),
            completion_delimiter: DEFAULT_COMPLETION_DELIMITER.to_string(),
        }
    }

    /// Build the domain-specific system prompt.
    ///
    /// This prompt is > 1024 tokens to ensure OpenAI prompt caching kicks in.
    /// It's identical across all extraction requests for a given domain config.
    pub fn system_prompt(&self) -> String {
        let td = &self.tuple_delimiter;
        let cd = &self.completion_delimiter;
        let entity_types_str = self.config.entity_type_names().join(", ");

        let mut prompt = String::with_capacity(8192);

        // --- Role ---
        prompt.push_str(&format!(
            "---Role---\n{}\n",
            self.config.prompts.role_description
        ));

        // --- Entity Types ---
        prompt.push_str(&format!(
            "\n---Entity Types---\nExtract entities using ONLY these types: {}\n\nType definitions:\n",
            entity_types_str
        ));
        for (type_name, description) in &self.config.entity_types {
            prompt.push_str(&format!("- {}: {}\n", type_name, description));
        }

        // --- Instructions ---
        prompt.push_str(&format!(
            r#"
---Instructions---
1.  **Entity Extraction & Output:**
    *   **Identification:** Identify clearly defined and meaningful entities in the input text.
    *   **Entity Details:** For each identified entity, extract:
        *   `entity_name`: Use consistent title case naming. Ensure the SAME person/entity always has the SAME name across extractions.
        *   `entity_type`: One of the types above. If none apply, use `Other`.
        *   `entity_description`: Concise description based **solely on the input text** — do NOT include general knowledge about the entity beyond what the text states.
    *   **Output Format - Entities:** 4 fields delimited by `{td}`, on a single line. First field must be `entity`.
        *   Format: `entity{td}entity_name{td}entity_type{td}entity_description`

2.  **Relationship Extraction & Output:**
    *   **Identification:** Identify direct, clearly stated, and meaningful relationships between extracted entities.
    *   **N-ary Relationship Decomposition:** Decompose N-ary relationships into binary pairs.
    *   **Relationship Type Keywords:** Use these specific keywords as the FIRST keyword in `relationship_keywords` when applicable:
"#,
            td = td,
        ));

        // Relationship keywords by category
        for (category, keywords) in &self.config.relationship_keywords {
            let capitalized = capitalize_first(category);
            let keyword_list: Vec<String> = keywords
                .iter()
                .map(|kw| format!("`{}` ({})", kw.keyword, kw.description))
                .collect();
            prompt.push_str(&format!(
                "        *   **{}:** {}\n",
                capitalized,
                keyword_list.join(", ")
            ));
        }

        prompt.push_str(&format!(
            r#"    *   You may combine multiple keywords (e.g., `financial_transaction, travel_companion`) but always lead with the most specific typed keyword.
    *   **Relationship Details:** For each binary relationship:
        *   `source_entity`: Source entity name (consistent with entity extraction)
        *   `target_entity`: Target entity name (consistent with entity extraction)
        *   `relationship_keywords`: One or more typed keywords separated by comma (use vocabulary above)
        *   `relationship_description`: Concise explanation of the relationship
    *   **Output Format - Relationships:** 5 fields delimited by `{td}`, on a single line. First field must be `relation`.
        *   Format: `relation{td}source_entity{td}target_entity{td}relationship_keywords{td}relationship_description`

3.  **Delimiter Usage Protocol:**
    *   The `{td}` is an atomic field separator. Do not embed content within it.
    *   **Correct:** `entity{td}Jeffrey Epstein{td}PERSON{td}Jeffrey Epstein is a financier.`

4.  **Relationship Direction & Duplication:**
    *   Treat relationships as **undirected** unless explicitly stated otherwise.
    *   Avoid duplicate relationships.

5.  **Output Order & Prioritization:**
    *   Output all entities first, then all relationships.
    *   Prioritize relationships most significant to the document's core meaning.

6.  **Context & Objectivity:**
    *   Use third person. Avoid pronouns like "this article", "I", "you".
    *   Name subjects explicitly.

7.  **Language:** Output must be in {language}. Retain proper nouns in original form.

8.  **Entity Name Canonicalization & Alias Handling:**
    *   Use the **most complete, commonly known name** for each entity. Examples:
"#,
            td = td,
            language = self.config.domain.language,
        ));

        // Canonicalization examples
        for example in &self.config.prompts.canonicalization_examples {
            prompt.push_str(&format!("        *   {}\n", example));
        }

        prompt.push_str(
            r#"    *   Do NOT create separate entities for the same real-world entity under different names, titles, or aliases.
    *   If the text uses multiple names for the same person (maiden name, married name, legal pseudonym), choose the most commonly recognized form and append aliases to the description: "Also known as: [alias1, alias2]".
    *   Strip middle initials, honorifics (Mr., Mrs., Dr.), and legal numbering (Jane Doe No. 102) from entity names unless essential for disambiguation.
"#,
        );

        // Extra instruction sections (timestamp, email, etc.)
        for section in &self.config.prompts.extra_instructions {
            prompt.push_str(&format!("\n---{}---\n{}\n", section.title, section.content));
        }

        // Completion signal instruction (always numbered 9 for backward compatibility)
        prompt.push_str(&format!(
            "\n9.  **Completion Signal:** Output `{}` only after all entities and relationships have been completely extracted.\n",
            cd,
        ));

        // --- Examples ---
        if !self.config.examples.is_empty() {
            prompt.push_str("\n---Examples---\n");
            for (i, example) in self.config.examples.iter().enumerate() {
                let output = example.output.replace("{td}", td).replace("{cd}", cd);
                prompt.push_str(&format!(
                    "\nExample {} ({}):\n<Input Text>\n{}\n\n<Output>\n{}",
                    i + 1,
                    example.title,
                    example.input,
                    output,
                ));
            }
        }

        prompt
    }

    /// Build the user prompt for a specific chunk.
    ///
    /// The preamble is static (for prompt caching), only the chunk text varies.
    pub fn user_prompt(&self, chunk_text: &str) -> String {
        let entity_types_str = self.config.entity_type_names().join(", ");
        let cd = &self.completion_delimiter;

        let mut instructions = vec![
            "Strictly adhere to all format requirements for entity and relationship lists.".to_string(),
            "Output *only* the extracted list of entities and relationships. No introductory or concluding remarks.".to_string(),
            format!("Output `{}` as the final line after all extractions.", cd),
            format!("Ensure the output language is {}.", self.config.domain.language),
        ];

        // Append domain-specific user instructions
        for (i, instr) in self.config.prompts.user_instructions.iter().enumerate() {
            instructions.push(format!("{}. {}", i + 5, instr.content));
        }

        let instructions_str: String = instructions
            .iter()
            .enumerate()
            .map(|(i, instr)| {
                if i < 4 {
                    format!("{}. {}", i + 1, instr)
                } else {
                    instr.clone()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            r#"---Task---
Extract entities and relationships from the input text below.

---Instructions---
{instructions}

---Data to be Processed---
<Entity_types>
[{entity_types}]

<Input Text>
```
{text}
```

<Output>"#,
            instructions = instructions_str,
            entity_types = entity_types_str,
            text = chunk_text,
        )
    }

    /// Get the entity types for this domain.
    pub fn entity_types(&self) -> Vec<String> {
        self.config
            .entity_type_names()
            .iter()
            .map(|s| s.to_string())
            .collect()
    }
}

/// Capitalize the first letter of a string.
fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_prompts() -> DomainExtractionPrompts {
        DomainExtractionPrompts::new(DomainConfig::builtin_epstein())
    }

    #[test]
    fn test_system_prompt_contains_domain_types() {
        let prompts = make_prompts();
        let system = prompts.system_prompt();

        assert!(system.contains("PERSON"));
        assert!(system.contains("LEGAL_CASE"));
        assert!(system.contains("FINANCIAL_ITEM"));
        assert!(system.contains("ALLEGATION"));
        assert!(system.contains("COMMUNICATION"));
        assert!(system.contains("DOCUMENT"));
    }

    #[test]
    fn test_system_prompt_contains_timestamp_instructions() {
        let prompts = make_prompts();
        let system = prompts.system_prompt();

        assert!(system.contains("TIMESTAMP:"));
        assert!(system.contains("YYYY-MM-DD"));
        assert!(system.contains("Email \"Sent:\" headers"));
    }

    #[test]
    fn test_system_prompt_contains_email_instructions() {
        let prompts = make_prompts();
        let system = prompts.system_prompt();

        assert!(system.contains("Email-Specific Instructions"));
        assert!(system.contains("From:"));
        assert!(system.contains("To:"));
        assert!(system.contains("Confidentiality notices"));
    }

    #[test]
    fn test_system_prompt_contains_examples() {
        let prompts = make_prompts();
        let system = prompts.system_prompt();

        assert!(system.contains("Example 1"));
        assert!(system.contains("Example 2"));
        assert!(system.contains("Example 3"));
        assert!(system.contains("Jeffrey Epstein"));
        assert!(system.contains("Michael Wolff"));
        assert!(system.contains("Non-Prosecution Agreement"));
    }

    #[test]
    fn test_system_prompt_long_enough_for_caching() {
        let prompts = make_prompts();
        let system = prompts.system_prompt();

        // OpenAI caches prompts >= 1024 tokens
        // Rough estimate: 1 token ~ 4 chars, so need >= 4096 chars
        assert!(
            system.len() > 4096,
            "System prompt should be > 4096 chars for prompt caching, got {}",
            system.len()
        );
    }

    #[test]
    fn test_user_prompt_contains_chunk() {
        let prompts = make_prompts();
        let user = prompts.user_prompt("This is test chunk text.");

        assert!(user.contains("This is test chunk text."));
        assert!(user.contains("<|COMPLETE|>"));
        assert!(user.contains("TIMESTAMP"));
    }

    #[test]
    fn test_system_prompt_contains_delimiters() {
        let prompts = make_prompts();
        let system = prompts.system_prompt();

        assert!(system.contains("<|#|>"));
        assert!(system.contains("<|COMPLETE|>"));
    }

    #[test]
    fn test_entity_types() {
        let prompts = make_prompts();
        let types = prompts.entity_types();

        assert_eq!(types.len(), 8);
        assert!(types.contains(&"PERSON".to_string()));
        assert!(types.contains(&"LEGAL_CASE".to_string()));
    }

    #[test]
    fn test_system_prompt_crime_focus() {
        let prompts = make_prompts();
        let system = prompts.system_prompt();

        // Typed relationship keywords
        assert!(system.contains("financial_transaction"));
        assert!(system.contains("legal_representation"));
        assert!(system.contains("travel_companion"));
        assert!(system.contains("communicated_with"));
        assert!(system.contains("alleged_abuse"));
        assert!(system.contains("trafficking"));
        assert!(system.contains("plea_agreement"));
        assert!(system.contains("witness_testimony"));
    }

    #[test]
    fn test_system_prompt_contains_role() {
        let prompts = make_prompts();
        let system = prompts.system_prompt();
        assert!(system.contains("Knowledge Graph Specialist"));
        assert!(system.contains("Jeffrey Epstein case"));
    }

    #[test]
    fn test_system_prompt_contains_canonicalization() {
        let prompts = make_prompts();
        let system = prompts.system_prompt();
        assert!(system.contains("Virginia Giuffre"));
        assert!(system.contains("NOT \"Virginia Roberts\""));
    }
}
