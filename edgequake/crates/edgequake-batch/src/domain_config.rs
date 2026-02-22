//! Domain-specific configuration for the batch extraction pipeline.
//!
//! Loads entity types, aliases, prompt content, and few-shot examples from a TOML file,
//! allowing the batch pipeline to be used for any domain without code changes.
//!
//! ## Config Resolution Order
//!
//! 1. CLI flag `--domain-config <path>`
//! 2. `EDGEQUAKE_DOMAIN_CONFIG` environment variable
//! 3. `./domain.toml` in the current directory
//! 4. `~/.edgequake/domain.toml`
//! 5. Built-in Epstein defaults (no file needed)

use edgequake_core::schema::SchemaProposal;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::info;

/// Root domain configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainConfig {
    /// Domain metadata.
    pub domain: DomainMetadata,

    /// Entity types: type_name -> description (ordered).
    pub entity_types: IndexMap<String, String>,

    /// Alias mappings: canonical_name -> [aliases].
    #[serde(default)]
    pub aliases: IndexMap<String, Vec<String>>,

    /// Prompt configuration.
    pub prompts: PromptConfig,

    /// Relationship keywords by category.
    #[serde(default)]
    pub relationship_keywords: IndexMap<String, Vec<RelationshipKeyword>>,

    /// Few-shot examples.
    #[serde(default)]
    pub examples: Vec<FewShotExample>,
}

/// Domain metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainMetadata {
    /// Short name for this domain (e.g. "epstein", "medical").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Output language (default: "English").
    #[serde(default = "default_language")]
    pub language: String,
}

fn default_language() -> String {
    "English".to_string()
}

/// Prompt configuration: role, canonicalization hints, and extra instruction sections.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptConfig {
    /// Role description paragraph for the system prompt.
    pub role_description: String,

    /// Canonicalization examples for entity names (e.g. `"Virginia Giuffre" (NOT "Virginia Roberts")`).
    #[serde(default)]
    pub canonicalization_examples: Vec<String>,

    /// Extra instruction sections appended to the system prompt (e.g. timestamp, email instructions).
    #[serde(default)]
    pub extra_instructions: Vec<InstructionSection>,

    /// Extra lines added to the user prompt instructions list.
    #[serde(default)]
    pub user_instructions: Vec<UserInstruction>,
}

/// A titled section of extra instructions for the system prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionSection {
    /// Section title (e.g. "Timestamp Instructions").
    pub title: String,
    /// Section content.
    pub content: String,
}

/// An extra line for the user prompt instructions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInstruction {
    /// Instruction content.
    pub content: String,
}

/// A relationship keyword with description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationshipKeyword {
    /// The keyword (e.g. "financial_transaction").
    pub keyword: String,
    /// Description (e.g. "payments, wire transfers").
    pub description: String,
}

/// A few-shot example for the system prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FewShotExample {
    /// Title (e.g. "Email", "Legal document").
    pub title: String,
    /// Input text.
    pub input: String,
    /// Expected output (may contain `{td}` and `{cd}` placeholders).
    pub output: String,
}

impl DomainConfig {
    /// Load domain configuration with priority chain:
    /// 1. Explicit path (CLI flag)
    /// 2. `EDGEQUAKE_DOMAIN_CONFIG` env var
    /// 3. `./domain.toml`
    /// 4. `~/.edgequake/domain.toml`
    /// 5. Built-in Epstein defaults
    pub fn load(cli_path: Option<&Path>) -> anyhow::Result<Self> {
        // 1. CLI flag
        if let Some(path) = cli_path {
            info!(path = %path.display(), "Loading domain config from CLI flag");
            return Self::from_file(path);
        }

        // 2. Environment variable
        if let Ok(path) = std::env::var("EDGEQUAKE_DOMAIN_CONFIG") {
            let path = PathBuf::from(path);
            if path.exists() {
                info!(path = %path.display(), "Loading domain config from EDGEQUAKE_DOMAIN_CONFIG");
                return Self::from_file(&path);
            }
        }

        // 3. ./domain.toml
        let local_path = PathBuf::from("domain.toml");
        if local_path.exists() {
            info!("Loading domain config from ./domain.toml");
            return Self::from_file(&local_path);
        }

        // 4. ~/.edgequake/domain.toml
        if let Some(home) = dirs::home_dir() {
            let home_path = home.join(".edgequake").join("domain.toml");
            if home_path.exists() {
                info!(path = %home_path.display(), "Loading domain config from ~/.edgequake/domain.toml");
                return Self::from_file(&home_path);
            }
        }

        // 5. Built-in default
        info!("Using built-in Epstein domain config");
        Ok(Self::builtin_epstein())
    }

    /// Load from a TOML file.
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            anyhow::anyhow!(
                "Failed to read domain config from {}: {}",
                path.display(),
                e
            )
        })?;
        let config: Self = toml::from_str(&content).map_err(|e| {
            anyhow::anyhow!(
                "Failed to parse domain config from {}: {}",
                path.display(),
                e
            )
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Build a `DomainConfig` from an approved `SchemaProposal`.
    ///
    /// Maps entity types (with descriptions) and relation types (as relationship
    /// keywords) from the schema proposal into the prompt configuration structure.
    pub fn from_schema(proposal: &SchemaProposal, namespace_name: &str) -> Self {
        debug_assert!(
            !proposal.entity_types.is_empty(),
            "from_schema requires non-empty entity types"
        );

        // 1. Entity types: name -> description (ordered)
        let mut entity_types = IndexMap::new();
        for et in &proposal.entity_types {
            entity_types.insert(et.name.clone(), et.description.clone());
        }

        // 2. Relationship keywords from relation types
        let mut relationship_keywords = IndexMap::new();
        let keywords: Vec<RelationshipKeyword> = proposal
            .relation_types
            .iter()
            .map(|rt| RelationshipKeyword {
                keyword: rt.name.clone(),
                description: rt.description.clone(),
            })
            .collect();
        if !keywords.is_empty() {
            relationship_keywords.insert("domain".to_string(), keywords);
        }

        // 3. Domain metadata
        let domain = DomainMetadata {
            name: namespace_name.to_string(),
            description: format!("Schema-derived config for namespace '{}'", namespace_name),
            language: "English".to_string(),
        };

        // 4. Prompts with generic role description
        let prompts = PromptConfig {
            role_description: format!(
                "You are a Knowledge Graph Specialist responsible for extracting entities \
                 and relationships from documents in the '{}' domain.",
                namespace_name
            ),
            canonicalization_examples: vec![],
            extra_instructions: vec![],
            user_instructions: vec![],
        };

        DomainConfig {
            domain,
            entity_types,
            aliases: IndexMap::new(),
            prompts,
            relationship_keywords,
            examples: vec![],
        }
    }

    /// Validate the configuration.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.entity_types.is_empty() {
            anyhow::bail!("domain config must define at least one entity type");
        }
        if self.prompts.role_description.is_empty() {
            anyhow::bail!("domain config must have a role_description");
        }
        Ok(())
    }

    /// Get entity type names in order.
    pub fn entity_type_names(&self) -> Vec<&str> {
        self.entity_types.keys().map(|s| s.as_str()).collect()
    }

    /// Get alias pairs as (alias, canonical) for EntityResolver.
    pub fn alias_pairs(&self) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        for (canonical, aliases) in &self.aliases {
            for alias in aliases {
                pairs.push((alias.clone(), canonical.clone()));
            }
        }
        pairs
    }

    /// Built-in Epstein domain configuration matching the original hardcoded prompts.
    pub fn builtin_epstein() -> Self {
        let mut entity_types = IndexMap::new();
        entity_types.insert("PERSON".to_string(), "Named individuals (Jeffrey Epstein, Ghislaine Maxwell, Bill Clinton, Donald Trump, etc.)".to_string());
        entity_types.insert(
            "ORGANIZATION".to_string(),
            "Companies, foundations, government agencies (J. Epstein & Co, DOJ, FBI, SEC, etc.)"
                .to_string(),
        );
        entity_types.insert("LOCATION".to_string(), "Places, addresses, properties (Palm Beach, Little St. James, Manhattan townhouse, etc.)".to_string());
        entity_types.insert(
            "LEGAL_CASE".to_string(),
            "Court cases, indictments, complaints (US v. Epstein, civil suit 08-80736, etc.)"
                .to_string(),
        );
        entity_types.insert(
            "FINANCIAL_ITEM".to_string(),
            "Money flows, transactions, assets (wire transfer, trust fund, property sale, etc.)"
                .to_string(),
        );
        entity_types.insert(
            "ALLEGATION".to_string(),
            "Specific alleged criminal acts (trafficking, obstruction, perjury, etc.)".to_string(),
        );
        entity_types.insert("COMMUNICATION".to_string(), "Documented contacts between parties (phone call, email, meeting, flight log entry, etc.)".to_string());
        entity_types.insert(
            "DOCUMENT".to_string(),
            "Referenced documents, filings, records (NPA, flight log, subpoena, deposition, etc.)"
                .to_string(),
        );

        let mut aliases = IndexMap::new();
        // Virginia Giuffre
        aliases.insert(
            "VIRGINIA_GIUFFRE".to_string(),
            vec![
                "VIRGINIA_ROBERTS".to_string(),
                "VIRGINIA_ROBERTS_GIUFFRE".to_string(),
                "VIRGINIA_L._GIUFFRE".to_string(),
                "VIRGINIA_L._ROBERTS".to_string(),
                "MS._GIUFFRE".to_string(),
                "MS._ROBERTS".to_string(),
            ],
        );
        // Donald Trump
        aliases.insert(
            "DONALD_TRUMP".to_string(),
            vec![
                "DONALD_J._TRUMP".to_string(),
                "DONALD_J_TRUMP".to_string(),
                "PRESIDENT_TRUMP".to_string(),
                "TRUMP".to_string(),
            ],
        );
        // Jeffrey Epstein
        aliases.insert(
            "JEFFREY_EPSTEIN".to_string(),
            vec![
                "J._EPSTEIN".to_string(),
                "J_EPSTEIN".to_string(),
                "EPSTEIN".to_string(),
                "MR._EPSTEIN".to_string(),
            ],
        );
        // Ghislaine Maxwell
        aliases.insert(
            "GHISLAINE_MAXWELL".to_string(),
            vec![
                "G._MAXWELL".to_string(),
                "MS._MAXWELL".to_string(),
                "MAXWELL".to_string(),
            ],
        );
        // Bill Clinton
        aliases.insert(
            "BILL_CLINTON".to_string(),
            vec![
                "WILLIAM_CLINTON".to_string(),
                "WILLIAM_J._CLINTON".to_string(),
                "WILLIAM_JEFFERSON_CLINTON".to_string(),
                "PRESIDENT_CLINTON".to_string(),
                "CLINTON".to_string(),
            ],
        );
        // Prince Andrew
        aliases.insert(
            "ANDREW_DUKE_OF_YORK".to_string(),
            vec![
                "PRINCE_ANDREW".to_string(),
                "ANDREW_WINDSOR".to_string(),
                "DUKE_OF_YORK".to_string(),
                "THE_DUKE_OF_YORK".to_string(),
            ],
        );
        // Alan Dershowitz
        aliases.insert(
            "ALAN_DERSHOWITZ".to_string(),
            vec![
                "ALAN_M._DERSHOWITZ".to_string(),
                "PROFESSOR_DERSHOWITZ".to_string(),
                "DERSHOWITZ".to_string(),
            ],
        );
        // Les Wexner
        aliases.insert(
            "LES_WEXNER".to_string(),
            vec![
                "LESLIE_WEXNER".to_string(),
                "LESLIE_H._WEXNER".to_string(),
                "L._WEXNER".to_string(),
                "WEXNER".to_string(),
            ],
        );
        // Jean-Luc Brunel
        aliases.insert(
            "JEAN-LUC_BRUNEL".to_string(),
            vec![
                "JEAN_LUC_BRUNEL".to_string(),
                "BRUNEL".to_string(),
                "J.L._BRUNEL".to_string(),
                "JL_BRUNEL".to_string(),
            ],
        );
        // Sarah Kellen
        aliases.insert(
            "SARAH_KELLEN".to_string(),
            vec!["SARAH_KELLEN_VICKERS".to_string(), "KELLEN".to_string()],
        );
        // Nadia Marcinkova
        aliases.insert(
            "NADIA_MARCINKOVA".to_string(),
            vec!["NADIA_MARCINKO".to_string(), "NADA_MARCINKOVA".to_string()],
        );
        // Location aliases
        aliases.insert(
            "MAR-A-LAGO".to_string(),
            vec![
                "MAR-A-LAGO_CLUB".to_string(),
                "MAR-A-LAGO_RESORT".to_string(),
                "MAR-A-LAGO_ESTATE".to_string(),
            ],
        );
        aliases.insert(
            "LITTLE_ST._JAMES".to_string(),
            vec![
                "LITTLE_SAINT_JAMES".to_string(),
                "LITTLE_ST_JAMES".to_string(),
                "LITTLE_SAINT_JAMES_ISLAND".to_string(),
                "LITTLE_ST._JAMES_ISLAND".to_string(),
            ],
        );
        aliases.insert("ZORRO_RANCH".to_string(), vec!["ZORRO_RANCH".to_string()]);
        // Organization aliases
        aliases.insert(
            "JEFFREY_EPSTEIN_FOUNDATION".to_string(),
            vec!["EPSTEIN_FOUNDATION".to_string()],
        );
        aliases.insert(
            "J._EPSTEIN_&_CO.".to_string(),
            vec![
                "J._EPSTEIN_&_COMPANY".to_string(),
                "J._EPSTEIN_AND_COMPANY".to_string(),
            ],
        );
        aliases.insert(
            "WEXNER_FOUNDATION".to_string(),
            vec!["THE_WEXNER_FOUNDATION".to_string()],
        );
        // FBI / DOJ / SDNY / Palm Beach PD
        aliases.insert(
            "FBI".to_string(),
            vec![
                "FEDERAL_BUREAU_OF_INVESTIGATION".to_string(),
                "F.B.I.".to_string(),
            ],
        );
        aliases.insert(
            "DOJ".to_string(),
            vec![
                "DEPARTMENT_OF_JUSTICE".to_string(),
                "DEPT._OF_JUSTICE".to_string(),
                "U.S._DEPARTMENT_OF_JUSTICE".to_string(),
            ],
        );
        aliases.insert(
            "SDNY".to_string(),
            vec![
                "SOUTHERN_DISTRICT_OF_NEW_YORK".to_string(),
                "S.D.N.Y.".to_string(),
                "U.S._ATTORNEY'S_OFFICE_SDNY".to_string(),
            ],
        );
        aliases.insert(
            "PALM_BEACH_POLICE_DEPARTMENT".to_string(),
            vec!["PALM_BEACH_POLICE".to_string(), "PBPD".to_string()],
        );

        let mut relationship_keywords = IndexMap::new();
        relationship_keywords.insert(
            "financial".to_string(),
            vec![
                RelationshipKeyword {
                    keyword: "financial_transaction".to_string(),
                    description: "payments, wire transfers".to_string(),
                },
                RelationshipKeyword {
                    keyword: "financial_control".to_string(),
                    description: "owns, manages assets".to_string(),
                },
                RelationshipKeyword {
                    keyword: "financial_benefit".to_string(),
                    description: "gifts, loans, trust beneficiary".to_string(),
                },
            ],
        );
        relationship_keywords.insert(
            "legal".to_string(),
            vec![
                RelationshipKeyword {
                    keyword: "legal_representation".to_string(),
                    description: "attorney-client".to_string(),
                },
                RelationshipKeyword {
                    keyword: "defendant".to_string(),
                    description: "accused in case".to_string(),
                },
                RelationshipKeyword {
                    keyword: "plaintiff".to_string(),
                    description: "accuser in case".to_string(),
                },
                RelationshipKeyword {
                    keyword: "witness_testimony".to_string(),
                    description: "testified about".to_string(),
                },
                RelationshipKeyword {
                    keyword: "plea_agreement".to_string(),
                    description: "NPA, cooperation deal".to_string(),
                },
                RelationshipKeyword {
                    keyword: "legal_filing".to_string(),
                    description: "filed motion, subpoena".to_string(),
                },
            ],
        );
        relationship_keywords.insert(
            "travel".to_string(),
            vec![
                RelationshipKeyword {
                    keyword: "travel_companion".to_string(),
                    description: "traveled together".to_string(),
                },
                RelationshipKeyword {
                    keyword: "travel_destination".to_string(),
                    description: "visited location".to_string(),
                },
                RelationshipKeyword {
                    keyword: "flight_log".to_string(),
                    description: "documented on flight".to_string(),
                },
            ],
        );
        relationship_keywords.insert(
            "employment".to_string(),
            vec![
                RelationshipKeyword {
                    keyword: "employer".to_string(),
                    description: "hired, employed".to_string(),
                },
                RelationshipKeyword {
                    keyword: "employee".to_string(),
                    description: "worked for".to_string(),
                },
                RelationshipKeyword {
                    keyword: "associate".to_string(),
                    description: "business associate".to_string(),
                },
            ],
        );
        relationship_keywords.insert(
            "social".to_string(),
            vec![
                RelationshipKeyword {
                    keyword: "personal_relationship".to_string(),
                    description: "friend, romantic, familial".to_string(),
                },
                RelationshipKeyword {
                    keyword: "introduced_by".to_string(),
                    description: "connected two parties".to_string(),
                },
                RelationshipKeyword {
                    keyword: "recruited".to_string(),
                    description: "recruited for activities".to_string(),
                },
            ],
        );
        relationship_keywords.insert(
            "communication".to_string(),
            vec![
                RelationshipKeyword {
                    keyword: "communicated_with".to_string(),
                    description: "email, phone, letter".to_string(),
                },
                RelationshipKeyword {
                    keyword: "meeting".to_string(),
                    description: "in-person meeting".to_string(),
                },
            ],
        );
        relationship_keywords.insert(
            "property".to_string(),
            vec![
                RelationshipKeyword {
                    keyword: "property_owner".to_string(),
                    description: "owns property".to_string(),
                },
                RelationshipKeyword {
                    keyword: "resided_at".to_string(),
                    description: "lived at location".to_string(),
                },
                RelationshipKeyword {
                    keyword: "visited".to_string(),
                    description: "visited location".to_string(),
                },
            ],
        );
        relationship_keywords.insert(
            "organizational".to_string(),
            vec![
                RelationshipKeyword {
                    keyword: "member_of".to_string(),
                    description: "belongs to organization".to_string(),
                },
                RelationshipKeyword {
                    keyword: "founded".to_string(),
                    description: "created organization".to_string(),
                },
                RelationshipKeyword {
                    keyword: "donated_to".to_string(),
                    description: "charitable contribution".to_string(),
                },
            ],
        );
        relationship_keywords.insert(
            "criminal".to_string(),
            vec![
                RelationshipKeyword {
                    keyword: "alleged_abuse".to_string(),
                    description: "alleged criminal conduct".to_string(),
                },
                RelationshipKeyword {
                    keyword: "conspiracy".to_string(),
                    description: "coordinated illegal activity".to_string(),
                },
                RelationshipKeyword {
                    keyword: "obstruction".to_string(),
                    description: "interfered with investigation".to_string(),
                },
                RelationshipKeyword {
                    keyword: "trafficking".to_string(),
                    description: "human trafficking".to_string(),
                },
            ],
        );
        relationship_keywords.insert(
            "evidentiary".to_string(),
            vec![
                RelationshipKeyword {
                    keyword: "mentioned_in".to_string(),
                    description: "referenced in document".to_string(),
                },
                RelationshipKeyword {
                    keyword: "evidence_of".to_string(),
                    description: "proves/supports claim".to_string(),
                },
                RelationshipKeyword {
                    keyword: "contradicts".to_string(),
                    description: "conflicts with testimony".to_string(),
                },
            ],
        );

        let examples = vec![
            FewShotExample {
                title: "Email".to_string(),
                input: r#"From: jeffrey E. [jeevacation@gmail.com]
Sent: 5/30/2019 9:34:38 PM
To: Michael Wolff
Subject: Re:
maybe as a favor to trump. in exchange for yemen and iran support...
he told me that mbs bought the 450 painting to curry favor. and that the russian who bought the palm beach house did so with full knowledge."#.to_string(),
                output: r#"entity{td}Jeffrey Epstein{td}PERSON{td}Jeffrey Epstein, using email jeevacation@gmail.com, communicated with journalist Michael Wolff about political dealings. [TIMESTAMP: 2019-05-30]
entity{td}Michael Wolff{td}PERSON{td}Michael Wolff is a journalist who exchanged emails with Jeffrey Epstein discussing political figures. [TIMESTAMP: 2019-05-30]
entity{td}Donald Trump{td}PERSON{td}Donald Trump is referenced in connection with favor exchanges involving foreign policy. [TIMESTAMP: 2019-05-30]
entity{td}Mohammed Bin Salman{td}PERSON{td}MBS (Mohammed Bin Salman) is referenced in context of payments and a $450M painting transaction. [TIMESTAMP: 2019-05-30]
entity{td}Palm Beach Property Sale{td}FINANCIAL_ITEM{td}A Russian individual purchased a house in Palm Beach connected to knowledge of relevant parties. [TIMESTAMP: 2019]
entity{td}$450M Painting Purchase{td}FINANCIAL_ITEM{td}Mohammed Bin Salman allegedly purchased a $450 million painting to curry favor. [TIMESTAMP: 2019-05-30]
entity{td}Yemen And Iran Policy{td}ALLEGATION{td}Exchange of political favors related to Yemen and Iran foreign policy support. [TIMESTAMP: 2019-05-30]
relation{td}Jeffrey Epstein{td}Michael Wolff{td}communicated_with, email{td}Epstein and Wolff exchanged emails discussing political connections and financial dealings. [TIMESTAMP: 2019-05-30]
relation{td}Mohammed Bin Salman{td}Donald Trump{td}financial_benefit, conspiracy{td}MBS allegedly provided favors to Trump in exchange for Yemen and Iran policy support. [TIMESTAMP: 2019-05-30]
relation{td}Mohammed Bin Salman{td}$450M Painting Purchase{td}financial_transaction{td}MBS purchased a $450M painting allegedly to curry political favor. [TIMESTAMP: 2019-05-30]
{cd}"#.to_string(),
            },
            FewShotExample {
                title: "Legal document".to_string(),
                input: r#"UNITED STATES DISTRICT COURT
SOUTHERN DISTRICT OF NEW YORK
Case No. 08-80736
GOVERNMENT'S RESPONSE TO DEFENDANT'S MOTION
Filed: June 15, 2008

The United States of America, by its attorney, hereby responds to defendant Jeffrey Epstein's
motion to dismiss. The non-prosecution agreement (NPA) dated September 24, 2007, between
the United States Attorney's Office for the Southern District of Florida and Jeffrey Epstein
does not bar this prosecution."#.to_string(),
                output: r#"entity{td}Jeffrey Epstein{td}PERSON{td}Jeffrey Epstein is the defendant in case 08-80736. [TIMESTAMP: 2008-06-15]
entity{td}United States District Court SDNY{td}ORGANIZATION{td}The United States District Court for the Southern District of New York is the court handling case 08-80736. [TIMESTAMP: 2008-06-15]
entity{td}US Attorney SDFL{td}ORGANIZATION{td}The United States Attorney's Office for the Southern District of Florida entered into the NPA with Epstein. [TIMESTAMP: 2007-09-24]
entity{td}Case 08-80736{td}LEGAL_CASE{td}Federal case number 08-80736, a prosecution against Jeffrey Epstein. [TIMESTAMP: 2008-06-15]
entity{td}Non-Prosecution Agreement{td}DOCUMENT{td}The NPA dated September 24, 2007 between SDFL and Jeffrey Epstein. [TIMESTAMP: 2007-09-24]
entity{td}Motion To Dismiss{td}DOCUMENT{td}Defendant's motion to dismiss the prosecution based on the NPA. [TIMESTAMP: 2008-06-15]
relation{td}Jeffrey Epstein{td}Case 08-80736{td}defendant{td}Jeffrey Epstein is the defendant in case 08-80736. [TIMESTAMP: 2008-06-15]
relation{td}Jeffrey Epstein{td}Non-Prosecution Agreement{td}plea_agreement{td}Epstein entered into a non-prosecution agreement with the SDFL. [TIMESTAMP: 2007-09-24]
relation{td}US Attorney SDFL{td}Non-Prosecution Agreement{td}legal_filing, plea_agreement{td}The SDFL office signed the NPA with Epstein. [TIMESTAMP: 2007-09-24]
relation{td}Case 08-80736{td}Motion To Dismiss{td}legal_filing{td}The motion to dismiss was filed in case 08-80736. [TIMESTAMP: 2008-06-15]
{cd}"#.to_string(),
            },
            FewShotExample {
                title: "Financial record".to_string(),
                input: r#"Wire Transfer Confirmation
Date: March 15, 2012
From: Epstein Financial Trust (Account ending 4892)
To: Maxwell Foundation (Account ending 7731)
Amount: $250,000.00
Reference: Monthly retainer - Q1 2012"#.to_string(),
                output: r#"entity{td}Epstein Financial Trust{td}ORGANIZATION{td}Epstein Financial Trust, account ending 4892, sent wire transfers to associated entities. [TIMESTAMP: 2012-03-15]
entity{td}Maxwell Foundation{td}ORGANIZATION{td}Maxwell Foundation, account ending 7731, received funds from Epstein Financial Trust. [TIMESTAMP: 2012-03-15]
entity{td}$250K Wire Transfer{td}FINANCIAL_ITEM{td}A $250,000 wire transfer from Epstein Financial Trust to Maxwell Foundation described as monthly retainer for Q1 2012. [TIMESTAMP: 2012-03-15]
relation{td}Epstein Financial Trust{td}Maxwell Foundation{td}financial_transaction{td}Epstein Financial Trust wired $250,000 to Maxwell Foundation as a monthly retainer payment. [TIMESTAMP: 2012-03-15]
relation{td}Epstein Financial Trust{td}$250K Wire Transfer{td}financial_control{td}Epstein Financial Trust initiated the wire transfer from account ending 4892. [TIMESTAMP: 2012-03-15]
{cd}"#.to_string(),
            },
        ];

        DomainConfig {
            domain: DomainMetadata {
                name: "epstein".to_string(),
                description: "Jeffrey Epstein criminal investigation dataset".to_string(),
                language: "English".to_string(),
            },
            entity_types,
            aliases,
            prompts: PromptConfig {
                role_description: "You are a Knowledge Graph Specialist responsible for extracting entities and relationships from legal, financial, and investigative documents related to the Jeffrey Epstein case and associated investigations.".to_string(),
                canonicalization_examples: vec![
                    "\"Virginia Giuffre\" (NOT \"Virginia Roberts\", \"Virginia Roberts Giuffre\", or \"Jane Doe No. 102\")".to_string(),
                    "\"Donald Trump\" (NOT \"Donald J. Trump\" or \"President Trump\")".to_string(),
                    "\"Mar-A-Lago\" (NOT \"Mar-A-Lago Club\" or \"Mar-A-Lago Resort\")".to_string(),
                    "\"Ghislaine Maxwell\" (NOT \"G. Maxwell\" or \"Ms. Maxwell\")".to_string(),
                ],
                extra_instructions: vec![
                    InstructionSection {
                        title: "Timestamp Instructions".to_string(),
                        content: r#"For every entity and relationship, if a date or time period can be inferred from the
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

If no date can be determined, omit the timestamp suffix entirely."#.to_string(),
                    },
                    InstructionSection {
                        title: "Email-Specific Instructions".to_string(),
                        content: r#"Many documents are email exchanges. For emails:
1. Extract PERSON entities for all senders (From:), recipients (To:, CC:, BCC:)
2. Use the email "Sent:" date as the timestamp for all entities/relationships in that email
3. If the email discusses meetings, calls, or visits, create COMMUNICATION entities
4. If the email references legal matters, create LEGAL_CASE entities
5. Email chains (Re:, Fwd:) may contain multiple conversations - extract from all, using each sub-email's date
6. Confidentiality notices at the end of emails should be ignored for extraction"#.to_string(),
                    },
                ],
                user_instructions: vec![
                    UserInstruction { content: "Append [TIMESTAMP: YYYY-MM-DD] to descriptions when dates can be inferred.".to_string() },
                    UserInstruction { content: "For emails, use the Sent: date as the timestamp.".to_string() },
                ],
            },
            relationship_keywords,
            examples,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_epstein_has_all_entity_types() {
        let config = DomainConfig::builtin_epstein();
        let types = config.entity_type_names();
        assert_eq!(types.len(), 8);
        assert!(types.contains(&"PERSON"));
        assert!(types.contains(&"LEGAL_CASE"));
        assert!(types.contains(&"FINANCIAL_ITEM"));
        assert!(types.contains(&"ALLEGATION"));
        assert!(types.contains(&"COMMUNICATION"));
        assert!(types.contains(&"DOCUMENT"));
    }

    #[test]
    fn test_builtin_epstein_validates() {
        let config = DomainConfig::builtin_epstein();
        config.validate().expect("builtin config should be valid");
    }

    #[test]
    fn test_alias_pairs_match_resolver_data() {
        let config = DomainConfig::builtin_epstein();
        let pairs = config.alias_pairs();

        // Check a few key alias pairs
        assert!(pairs.contains(&(
            "VIRGINIA_ROBERTS".to_string(),
            "VIRGINIA_GIUFFRE".to_string()
        )));
        assert!(pairs.contains(&("DONALD_J._TRUMP".to_string(), "DONALD_TRUMP".to_string())));
        assert!(pairs.contains(&(
            "FEDERAL_BUREAU_OF_INVESTIGATION".to_string(),
            "FBI".to_string()
        )));
        assert!(pairs.contains(&("WILLIAM_CLINTON".to_string(), "BILL_CLINTON".to_string())));
    }

    #[test]
    fn test_entity_type_order_preserved() {
        let config = DomainConfig::builtin_epstein();
        let types = config.entity_type_names();
        assert_eq!(types[0], "PERSON");
        assert_eq!(types[1], "ORGANIZATION");
        assert_eq!(types[7], "DOCUMENT");
    }

    #[test]
    fn test_roundtrip_toml_serialization() {
        let config = DomainConfig::builtin_epstein();
        let toml_str = toml::to_string_pretty(&config).expect("should serialize to TOML");
        let parsed: DomainConfig = toml::from_str(&toml_str).expect("should parse back from TOML");
        assert_eq!(parsed.entity_type_names(), config.entity_type_names());
        assert_eq!(parsed.alias_pairs().len(), config.alias_pairs().len());
    }

    #[test]
    fn test_validate_rejects_empty_entity_types() {
        let mut config = DomainConfig::builtin_epstein();
        config.entity_types.clear();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_rejects_empty_role() {
        let mut config = DomainConfig::builtin_epstein();
        config.prompts.role_description.clear();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_load_falls_back_to_builtin() {
        // With no files present and no env var, should return builtin
        let config = DomainConfig::load(None).expect("should fall back to builtin");
        assert_eq!(config.domain.name, "epstein");
    }
}
