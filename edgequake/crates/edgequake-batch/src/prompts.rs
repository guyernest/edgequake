//! Domain-specific extraction prompts for the Epstein House Oversight dataset.
//!
//! Customizes entity types, few-shot examples, and extraction instructions
//! for legal/financial/crime documents including:
//! - Email parsing (From/To/Sent headers)
//! - Timestamp extraction from all document types
//! - Crime-focused relationship extraction
//!
//! ## Prompt Caching Strategy
//!
//! The system prompt + user preamble are kept identical and > 1024 tokens
//! across all 129K requests to maximize OpenAI's automatic prompt caching.
//! Cached tokens cost 50% less, so combined with batch API (50% off),
//! the system prompt portion costs ~75% less than standard pricing.

use edgequake_pipeline::prompts::{DEFAULT_COMPLETION_DELIMITER, DEFAULT_TUPLE_DELIMITER};

/// Domain entity types for the Epstein/crime dataset.
pub const ENTITY_TYPES: &[&str] = &[
    "PERSON",
    "ORGANIZATION",
    "LOCATION",
    "LEGAL_CASE",
    "FINANCIAL_ITEM",
    "ALLEGATION",
    "COMMUNICATION",
    "DOCUMENT",
];

/// Epstein-specific extraction prompts.
pub struct EpsteinExtractionPrompts {
    tuple_delimiter: String,
    completion_delimiter: String,
}

impl EpsteinExtractionPrompts {
    pub fn new() -> Self {
        Self {
            tuple_delimiter: DEFAULT_TUPLE_DELIMITER.to_string(),
            completion_delimiter: DEFAULT_COMPLETION_DELIMITER.to_string(),
        }
    }

    /// Build the domain-specific system prompt.
    ///
    /// This prompt is > 1024 tokens to ensure OpenAI prompt caching kicks in.
    /// It's identical across all 129K extraction requests.
    pub fn system_prompt(&self) -> String {
        let entity_types_str = ENTITY_TYPES.join(", ");
        let td = &self.tuple_delimiter;
        let cd = &self.completion_delimiter;

        format!(
            r#"---Role---
You are a Knowledge Graph Specialist responsible for extracting entities and relationships from legal, financial, and investigative documents related to the Jeffrey Epstein case and associated investigations.

---Entity Types---
Extract entities using ONLY these types: {entity_types}

Type definitions:
- PERSON: Named individuals (Jeffrey Epstein, Ghislaine Maxwell, Bill Clinton, Donald Trump, etc.)
- ORGANIZATION: Companies, foundations, government agencies (J. Epstein & Co, DOJ, FBI, SEC, etc.)
- LOCATION: Places, addresses, properties (Palm Beach, Little St. James, Manhattan townhouse, etc.)
- LEGAL_CASE: Court cases, indictments, complaints (US v. Epstein, civil suit 08-80736, etc.)
- FINANCIAL_ITEM: Money flows, transactions, assets (wire transfer, trust fund, property sale, etc.)
- ALLEGATION: Specific alleged criminal acts (trafficking, obstruction, perjury, etc.)
- COMMUNICATION: Documented contacts between parties (phone call, email, meeting, flight log entry, etc.)
- DOCUMENT: Referenced documents, filings, records (NPA, flight log, subpoena, deposition, etc.)

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
        *   **Financial:** `financial_transaction` (payments, wire transfers), `financial_control` (owns, manages assets), `financial_benefit` (gifts, loans, trust beneficiary)
        *   **Legal:** `legal_representation` (attorney-client), `defendant` (accused in case), `plaintiff` (accuser in case), `witness_testimony` (testified about), `plea_agreement` (NPA, cooperation deal), `legal_filing` (filed motion, subpoena)
        *   **Travel:** `travel_companion` (traveled together), `travel_destination` (visited location), `flight_log` (documented on flight)
        *   **Employment:** `employer` (hired, employed), `employee` (worked for), `associate` (business associate)
        *   **Social:** `personal_relationship` (friend, romantic, familial), `introduced_by` (connected two parties), `recruited` (recruited for activities)
        *   **Communication:** `communicated_with` (email, phone, letter), `meeting` (in-person meeting)
        *   **Property:** `property_owner` (owns property), `resided_at` (lived at location), `visited` (visited location)
        *   **Organizational:** `member_of` (belongs to organization), `founded` (created organization), `donated_to` (charitable contribution)
        *   **Criminal:** `alleged_abuse` (alleged criminal conduct), `conspiracy` (coordinated illegal activity), `obstruction` (interfered with investigation), `trafficking` (human trafficking)
        *   **Evidentiary:** `mentioned_in` (referenced in document), `evidence_of` (proves/supports claim), `contradicts` (conflicts with testimony)
    *   You may combine multiple keywords (e.g., `financial_transaction, travel_companion`) but always lead with the most specific typed keyword.
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

7.  **Language:** Output must be in English. Retain proper nouns in original form.

8.  **Entity Name Canonicalization & Alias Handling:**
    *   Use the **most complete, commonly known name** for each entity. Examples:
        *   "Virginia Giuffre" (NOT "Virginia Roberts", "Virginia Roberts Giuffre", or "Jane Doe No. 102")
        *   "Donald Trump" (NOT "Donald J. Trump" or "President Trump")
        *   "Mar-A-Lago" (NOT "Mar-A-Lago Club" or "Mar-A-Lago Resort")
        *   "Ghislaine Maxwell" (NOT "G. Maxwell" or "Ms. Maxwell")
    *   Do NOT create separate entities for the same real-world entity under different names, titles, or aliases.
    *   If the text uses multiple names for the same person (maiden name, married name, legal pseudonym), choose the most commonly recognized form and append aliases to the description: "Also known as: [alias1, alias2]".
    *   Strip middle initials, honorifics (Mr., Mrs., Dr.), and legal numbering (Jane Doe No. 102) from entity names unless essential for disambiguation.

---Timestamp Instructions---
For every entity and relationship, if a date or time period can be inferred from the
document context, append [TIMESTAMP: YYYY-MM-DD] at the end of the description field.
Use the most specific date available:
- Full date: [TIMESTAMP: 2019-05-30]
- Month only: [TIMESTAMP: 2019-05]
- Year only: [TIMESTAMP: 2019]
- Date range: [TIMESTAMP: 2008-01/2008-06]

Sources of dates:
- Email "Sent:" headers
- Document filing dates
- Dates mentioned in the text ("On March 23, 2017...")
- Transaction dates in financial records

If no date can be determined, omit the timestamp suffix entirely.

---Email-Specific Instructions---
Many documents are email exchanges. For emails:
1. Extract PERSON entities for all senders (From:), recipients (To:, CC:, BCC:)
2. Use the email "Sent:" date as the timestamp for all entities/relationships in that email
3. If the email discusses meetings, calls, or visits, create COMMUNICATION entities
4. If the email references legal matters, create LEGAL_CASE entities
5. Email chains (Re:, Fwd:) may contain multiple conversations - extract from all, using each sub-email's date
6. Confidentiality notices at the end of emails should be ignored for extraction

9.  **Completion Signal:** Output `{cd}` only after all entities and relationships have been completely extracted.

---Examples---

Example 1 (Email):
<Input Text>
From: jeffrey E. [jeevacation@gmail.com]
Sent: 5/30/2019 9:34:38 PM
To: Michael Wolff
Subject: Re:
maybe as a favor to trump. in exchange for yemen and iran support...
he told me that mbs bought the 450 painting to curry favor. and that the russian who bought the palm beach house did so with full knowledge.

<Output>
entity{td}Jeffrey Epstein{td}PERSON{td}Jeffrey Epstein, using email jeevacation@gmail.com, communicated with journalist Michael Wolff about political dealings. [TIMESTAMP: 2019-05-30]
entity{td}Michael Wolff{td}PERSON{td}Michael Wolff is a journalist who exchanged emails with Jeffrey Epstein discussing political figures. [TIMESTAMP: 2019-05-30]
entity{td}Donald Trump{td}PERSON{td}Donald Trump is referenced in connection with favor exchanges involving foreign policy. [TIMESTAMP: 2019-05-30]
entity{td}Mohammed Bin Salman{td}PERSON{td}MBS (Mohammed Bin Salman) is referenced in context of payments and a $450M painting transaction. [TIMESTAMP: 2019-05-30]
entity{td}Palm Beach Property Sale{td}FINANCIAL_ITEM{td}A Russian individual purchased a house in Palm Beach connected to knowledge of relevant parties. [TIMESTAMP: 2019]
entity{td}$450M Painting Purchase{td}FINANCIAL_ITEM{td}Mohammed Bin Salman allegedly purchased a $450 million painting to curry favor. [TIMESTAMP: 2019-05-30]
entity{td}Yemen And Iran Policy{td}ALLEGATION{td}Exchange of political favors related to Yemen and Iran foreign policy support. [TIMESTAMP: 2019-05-30]
relation{td}Jeffrey Epstein{td}Michael Wolff{td}communicated_with, email{td}Epstein and Wolff exchanged emails discussing political connections and financial dealings. [TIMESTAMP: 2019-05-30]
relation{td}Mohammed Bin Salman{td}Donald Trump{td}financial_benefit, conspiracy{td}MBS allegedly provided favors to Trump in exchange for Yemen and Iran policy support. [TIMESTAMP: 2019-05-30]
relation{td}Mohammed Bin Salman{td}$450M Painting Purchase{td}financial_transaction{td}MBS purchased a $450M painting allegedly to curry political favor. [TIMESTAMP: 2019-05-30]
{cd}

Example 2 (Legal document):
<Input Text>
UNITED STATES DISTRICT COURT
SOUTHERN DISTRICT OF NEW YORK
Case No. 08-80736
GOVERNMENT'S RESPONSE TO DEFENDANT'S MOTION
Filed: June 15, 2008

The United States of America, by its attorney, hereby responds to defendant Jeffrey Epstein's
motion to dismiss. The non-prosecution agreement (NPA) dated September 24, 2007, between
the United States Attorney's Office for the Southern District of Florida and Jeffrey Epstein
does not bar this prosecution.

<Output>
entity{td}Jeffrey Epstein{td}PERSON{td}Jeffrey Epstein is the defendant in case 08-80736. [TIMESTAMP: 2008-06-15]
entity{td}United States District Court SDNY{td}ORGANIZATION{td}The United States District Court for the Southern District of New York is the court handling case 08-80736. [TIMESTAMP: 2008-06-15]
entity{td}US Attorney SDFL{td}ORGANIZATION{td}The United States Attorney's Office for the Southern District of Florida entered into the NPA with Epstein. [TIMESTAMP: 2007-09-24]
entity{td}Case 08-80736{td}LEGAL_CASE{td}Federal case number 08-80736, a prosecution against Jeffrey Epstein. [TIMESTAMP: 2008-06-15]
entity{td}Non-Prosecution Agreement{td}DOCUMENT{td}The NPA dated September 24, 2007 between SDFL and Jeffrey Epstein. [TIMESTAMP: 2007-09-24]
entity{td}Motion To Dismiss{td}DOCUMENT{td}Defendant's motion to dismiss the prosecution based on the NPA. [TIMESTAMP: 2008-06-15]
relation{td}Jeffrey Epstein{td}Case 08-80736{td}defendant{td}Jeffrey Epstein is the defendant in case 08-80736. [TIMESTAMP: 2008-06-15]
relation{td}Jeffrey Epstein{td}Non-Prosecution Agreement{td}plea_agreement{td}Epstein entered into a non-prosecution agreement with the SDFL. [TIMESTAMP: 2007-09-24]
relation{td}US Attorney SDFL{td}Non-Prosecution Agreement{td}legal_filing, plea_agreement{td}The SDFL office signed the NPA with Epstein. [TIMESTAMP: 2007-09-24]
relation{td}Case 08-80736{td}Motion To Dismiss{td}legal_filing{td}The motion to dismiss was filed in case 08-80736. [TIMESTAMP: 2008-06-15]
{cd}

Example 3 (Financial record):
<Input Text>
Wire Transfer Confirmation
Date: March 15, 2012
From: Epstein Financial Trust (Account ending 4892)
To: Maxwell Foundation (Account ending 7731)
Amount: $250,000.00
Reference: Monthly retainer - Q1 2012

<Output>
entity{td}Epstein Financial Trust{td}ORGANIZATION{td}Epstein Financial Trust, account ending 4892, sent wire transfers to associated entities. [TIMESTAMP: 2012-03-15]
entity{td}Maxwell Foundation{td}ORGANIZATION{td}Maxwell Foundation, account ending 7731, received funds from Epstein Financial Trust. [TIMESTAMP: 2012-03-15]
entity{td}$250K Wire Transfer{td}FINANCIAL_ITEM{td}A $250,000 wire transfer from Epstein Financial Trust to Maxwell Foundation described as monthly retainer for Q1 2012. [TIMESTAMP: 2012-03-15]
relation{td}Epstein Financial Trust{td}Maxwell Foundation{td}financial_transaction{td}Epstein Financial Trust wired $250,000 to Maxwell Foundation as a monthly retainer payment. [TIMESTAMP: 2012-03-15]
relation{td}Epstein Financial Trust{td}$250K Wire Transfer{td}financial_control{td}Epstein Financial Trust initiated the wire transfer from account ending 4892. [TIMESTAMP: 2012-03-15]
{cd}
"#,
            entity_types = entity_types_str,
            td = td,
            cd = cd,
        )
    }

    /// Build the user prompt for a specific chunk.
    ///
    /// The preamble is static (for prompt caching), only the chunk text varies.
    pub fn user_prompt(&self, chunk_text: &str) -> String {
        let entity_types_str = ENTITY_TYPES.join(", ");

        format!(
            r#"---Task---
Extract entities and relationships from the input text below.

---Instructions---
1. Strictly adhere to all format requirements for entity and relationship lists.
2. Output *only* the extracted list of entities and relationships. No introductory or concluding remarks.
3. Output `{cd}` as the final line after all extractions.
4. Ensure the output language is English.
5. Append [TIMESTAMP: YYYY-MM-DD] to descriptions when dates can be inferred.
6. For emails, use the Sent: date as the timestamp.

---Data to be Processed---
<Entity_types>
[{entity_types}]

<Input Text>
```
{text}
```

<Output>"#,
            cd = self.completion_delimiter,
            entity_types = entity_types_str,
            text = chunk_text
        )
    }

    /// Get the entity types for this domain.
    pub fn entity_types(&self) -> Vec<String> {
        ENTITY_TYPES.iter().map(|s| s.to_string()).collect()
    }
}

impl Default for EpsteinExtractionPrompts {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_prompt_contains_domain_types() {
        let prompts = EpsteinExtractionPrompts::new();
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
        let prompts = EpsteinExtractionPrompts::new();
        let system = prompts.system_prompt();

        assert!(system.contains("TIMESTAMP:"));
        assert!(system.contains("YYYY-MM-DD"));
        assert!(system.contains("Email \"Sent:\" headers"));
    }

    #[test]
    fn test_system_prompt_contains_email_instructions() {
        let prompts = EpsteinExtractionPrompts::new();
        let system = prompts.system_prompt();

        assert!(system.contains("Email-Specific Instructions"));
        assert!(system.contains("From:"));
        assert!(system.contains("To:"));
        assert!(system.contains("Confidentiality notices"));
    }

    #[test]
    fn test_system_prompt_contains_examples() {
        let prompts = EpsteinExtractionPrompts::new();
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
        let prompts = EpsteinExtractionPrompts::new();
        let system = prompts.system_prompt();

        // OpenAI caches prompts >= 1024 tokens
        // Rough estimate: 1 token ≈ 4 chars, so need >= 4096 chars
        assert!(
            system.len() > 4096,
            "System prompt should be > 4096 chars for prompt caching, got {}",
            system.len()
        );
    }

    #[test]
    fn test_user_prompt_contains_chunk() {
        let prompts = EpsteinExtractionPrompts::new();
        let user = prompts.user_prompt("This is test chunk text.");

        assert!(user.contains("This is test chunk text."));
        assert!(user.contains("<|COMPLETE|>"));
        assert!(user.contains("TIMESTAMP"));
    }

    #[test]
    fn test_system_prompt_contains_delimiters() {
        let prompts = EpsteinExtractionPrompts::new();
        let system = prompts.system_prompt();

        assert!(system.contains("<|#|>"));
        assert!(system.contains("<|COMPLETE|>"));
    }

    #[test]
    fn test_entity_types() {
        let prompts = EpsteinExtractionPrompts::new();
        let types = prompts.entity_types();

        assert_eq!(types.len(), 8);
        assert!(types.contains(&"PERSON".to_string()));
        assert!(types.contains(&"LEGAL_CASE".to_string()));
    }

    #[test]
    fn test_system_prompt_crime_focus() {
        let prompts = EpsteinExtractionPrompts::new();
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
}
