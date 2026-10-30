/// Semantic analysis for understanding game state queries in natural language
use regex::Regex;
use serde::{Deserialize, Serialize};
use strsim::jaro_winkler;

use crate::server::brp_messages::{BrpRequest, ComponentFilter, FilterOp, QueryFilter};
use crate::server::error::{Error, Result};

/// Game concepts that can be analyzed semantically
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum GameConcept {
    /// Entities that appear stuck (low velocity but have movement target)
    StuckEntities,
    /// Entities moving faster than normal
    FastMovingObjects,
    /// Entities with overlapping collision bounds
    OverlappingColliders,
    /// Entities that might be memory leaks (inactive but taking resources)
    PotentialMemoryLeaks,
    /// Entities with inconsistent state
    InconsistentState,
    /// Entities experiencing physics violations
    PhysicsViolations,
}

impl GameConcept {
    /// Get human-readable description
    #[must_use]
    pub fn description(&self) -> &'static str {
        match self {
            Self::StuckEntities => "Entities that appear stuck (have target but low velocity)",
            Self::FastMovingObjects => "Entities moving faster than expected",
            Self::OverlappingColliders => "Entities with overlapping collision boundaries",
            Self::PotentialMemoryLeaks => "Inactive entities consuming resources",
            Self::InconsistentState => "Entities with contradictory component values",
            Self::PhysicsViolations => "Entities violating physics constraints",
        }
    }
}

/// Configurable thresholds for semantic analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticThresholds {
    /// Velocity threshold below which entity is considered stuck (units/second)
    pub stuck_velocity_threshold: f32,
    /// Velocity threshold above which entity is considered fast
    pub fast_velocity_threshold: f32,
    /// Distance threshold for overlapping colliders
    pub overlap_distance_threshold: f32,
    /// Time threshold for inactive entities (seconds)
    pub inactive_time_threshold: f32,
    /// Fuzzy matching threshold for component names (0.0-1.0)
    pub fuzzy_match_threshold: f32,
}

impl Default for SemanticThresholds {
    fn default() -> Self {
        Self {
            stuck_velocity_threshold: 0.1,
            fast_velocity_threshold: 50.0,
            overlap_distance_threshold: 0.5,
            inactive_time_threshold: 60.0,
            fuzzy_match_threshold: 0.8,
        }
    }
}

/// Explanation for why an entity matches semantic criteria
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchExplanation {
    pub concept: GameConcept,
    pub reason: String,
    pub confidence: f32,
    pub relevant_components: Vec<String>,
}

/// Result of semantic analysis with explanations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticQueryResult {
    pub request: BrpRequest,
    pub explanations: Vec<MatchExplanation>,
    pub suggestions: Vec<String>,
}

/// Builder for constructing complex semantic queries
#[derive(Debug, Clone)]
pub struct SemanticQueryBuilder {
    concepts: Vec<GameConcept>,
    logic: QueryLogic,
    thresholds: SemanticThresholds,
    component_filters: Vec<ComponentFilter>,
}

/// Logic operators for combining semantic queries
#[derive(Debug, Clone, PartialEq)]
pub enum QueryLogic {
    And,
    Or,
}

impl SemanticQueryBuilder {
    /// Create a new semantic query builder
    #[must_use]
    pub fn new() -> Self {
        Self {
            concepts: Vec::new(),
            logic: QueryLogic::And,
            thresholds: SemanticThresholds::default(),
            component_filters: Vec::new(),
        }
    }

    /// Add a game concept to analyze
    #[must_use]
    pub fn with_concept(mut self, concept: GameConcept) -> Self {
        self.concepts.push(concept);
        self
    }

    /// Set query logic (AND/OR)
    #[must_use]
    pub fn with_logic(mut self, logic: QueryLogic) -> Self {
        self.logic = logic;
        self
    }

    /// Set custom thresholds
    #[must_use]
    pub fn with_thresholds(mut self, thresholds: SemanticThresholds) -> Self {
        self.thresholds = thresholds;
        self
    }
    
    /// Set custom thresholds from reference (memory optimized)
    #[must_use]
    pub fn with_thresholds_ref(mut self, thresholds: &SemanticThresholds) -> Self {
        self.thresholds = thresholds.clone();
        self
    }

    /// Add component filter
    #[must_use]
    pub fn with_component_filter(mut self, filter: ComponentFilter) -> Self {
        self.component_filters.push(filter);
        self
    }

    /// Build the semantic query
    pub fn build(self) -> Result<SemanticQueryResult> {
        if self.concepts.is_empty() {
            return Err(Error::Brp(
                "No concepts specified for semantic query".to_string(),
            ));
        }

        let mut with_components = Vec::new();
        let mut where_clauses = self.component_filters.clone();
        let mut explanations = Vec::new();

        for concept in &self.concepts {
            let (components, filters, explanation) = self.build_concept_query(concept)?;
            with_components.extend(components);
            where_clauses.extend(filters);
            explanations.push(explanation);
        }

        // Remove duplicates
        with_components.sort();
        with_components.dedup();

        let request = BrpRequest {
            method: crate::server::brp::builtin_methods::BRP_QUERY_METHOD.to_string(),
            id: None,
            params: Some(crate::server::brp::query_params(
                Some(&QueryFilter {
                    with: if with_components.is_empty() {
                        None
                    } else {
                        Some(with_components)
                    },
                    without: None,
                    where_clause: if where_clauses.is_empty() {
                        None
                    } else {
                        Some(where_clauses)
                    },
                }),
                true,
            )),
        };

        let suggestions = self.generate_suggestions();

        Ok(SemanticQueryResult {
            request,
            explanations,
            suggestions,
        })
    }

    fn build_concept_query(
        &self,
        concept: &GameConcept,
    ) -> Result<(Vec<String>, Vec<ComponentFilter>, MatchExplanation)> {
        match concept {
            GameConcept::StuckEntities => {
                let components = vec!["Transform".to_string(), "Velocity".to_string()];
                let filters = vec![
                    ComponentFilter {
                        component: "Velocity".to_string(),
                        field: Some("linear.x".to_string()),
                        op: FilterOp::LessThan,
                        value: serde_json::json!(self.thresholds.stuck_velocity_threshold),
                    },
                    ComponentFilter {
                        component: "Velocity".to_string(),
                        field: Some("linear.y".to_string()),
                        op: FilterOp::LessThan,
                        value: serde_json::json!(self.thresholds.stuck_velocity_threshold),
                    },
                ];
                let explanation = MatchExplanation {
                    concept: concept.clone(),
                    reason: format!(
                        "Entities with velocity magnitude below {} units/second",
                        self.thresholds.stuck_velocity_threshold
                    ),
                    confidence: 0.9,
                    relevant_components: components.clone(),
                };
                Ok((components, filters, explanation))
            }
            GameConcept::FastMovingObjects => {
                let components = vec!["Velocity".to_string()];
                let filters = vec![ComponentFilter {
                    component: "Velocity".to_string(),
                    field: Some("linear.x".to_string()),
                    op: FilterOp::GreaterThan,
                    value: serde_json::json!(self.thresholds.fast_velocity_threshold),
                }];
                let explanation = MatchExplanation {
                    concept: concept.clone(),
                    reason: format!(
                        "Entities with velocity above {} units/second",
                        self.thresholds.fast_velocity_threshold
                    ),
                    confidence: 0.85,
                    relevant_components: components.clone(),
                };
                Ok((components, filters, explanation))
            }
            GameConcept::OverlappingColliders => {
                let components = vec!["Transform".to_string(), "Collider".to_string()];
                let filters = Vec::new(); // Complex spatial analysis would require post-processing
                let explanation = MatchExplanation {
                    concept: concept.clone(),
                    reason: "Entities with colliders that may be overlapping (requires spatial analysis)".to_string(),
                    confidence: 0.7,
                    relevant_components: components.clone(),
                };
                Ok((components, filters, explanation))
            }
            GameConcept::PotentialMemoryLeaks => {
                let components = vec!["Transform".to_string()];
                let filters = Vec::new(); // Would require analysis of entity lifecycle
                let explanation = MatchExplanation {
                    concept: concept.clone(),
                    reason: "Entities that may be consuming resources without active purpose"
                        .to_string(),
                    confidence: 0.6,
                    relevant_components: components.clone(),
                };
                Ok((components, filters, explanation))
            }
            GameConcept::InconsistentState => {
                let components = vec!["Health".to_string(), "Alive".to_string()];
                let filters = vec![
                    ComponentFilter {
                        component: "Health".to_string(),
                        field: Some("current".to_string()),
                        op: FilterOp::LessThanOrEqual,
                        value: serde_json::json!(0.0),
                    },
                    ComponentFilter {
                        component: "Alive".to_string(),
                        field: None,
                        op: FilterOp::Equal,
                        value: serde_json::json!(true),
                    },
                ];
                let explanation = MatchExplanation {
                    concept: concept.clone(),
                    reason: "Entities marked as alive but with zero or negative health".to_string(),
                    confidence: 0.95,
                    relevant_components: components.clone(),
                };
                Ok((components, filters, explanation))
            }
            GameConcept::PhysicsViolations => {
                let components = vec!["Transform".to_string(), "RigidBody".to_string()];
                let filters = Vec::new(); // Would require physics constraint validation
                let explanation = MatchExplanation {
                    concept: concept.clone(),
                    reason: "Entities potentially violating physics constraints".to_string(),
                    confidence: 0.75,
                    relevant_components: components.clone(),
                };
                Ok((components, filters, explanation))
            }
        }
    }

    fn generate_suggestions(&self) -> Vec<String> {
        let mut suggestions = Vec::new();

        if self.concepts.contains(&GameConcept::StuckEntities) {
            suggestions.push("Try adjusting stuck_velocity_threshold in config".to_string());
            suggestions.push("Look for entities with NavigationTarget component".to_string());
        }

        if self.concepts.contains(&GameConcept::FastMovingObjects) {
            suggestions.push("Check for runaway physics simulations".to_string());
            suggestions.push("Verify velocity damping is applied correctly".to_string());
        }

        if self.concepts.contains(&GameConcept::OverlappingColliders) {
            suggestions.push("Use spatial partitioning for collision detection".to_string());
            suggestions.push("Check collision layer configuration".to_string());
        }

        suggestions
    }
}

impl Default for SemanticQueryBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Semantic analyzer for understanding natural language queries about game state
pub struct SemanticAnalyzer {
    thresholds: SemanticThresholds,
    patterns: Vec<SemanticPattern>,
}

struct SemanticPattern {
    pattern: Regex,
    concepts: Vec<GameConcept>,
    builder_fn: fn(&SemanticAnalyzer, &regex::Captures) -> Result<SemanticQueryBuilder>,
}

impl SemanticAnalyzer {
    /// Create a new semantic analyzer with default thresholds
    /// 
    /// # Errors
    /// Returns error if regex patterns fail to compile
    pub fn new() -> Result<Self> {
        Self::with_thresholds(SemanticThresholds::default())
    }

    /// Create a semantic analyzer with custom thresholds
    /// 
    /// # Errors
    /// Returns error if regex patterns fail to compile
    pub fn with_thresholds(thresholds: SemanticThresholds) -> Result<Self> {
        let patterns = vec![
            SemanticPattern {
                pattern: Regex::new(r"(?i)find\s+stuck\s+entities?")
                    .map_err(|e| Error::Validation(format!("Invalid regex pattern: {}", e)))?,
                concepts: vec![GameConcept::StuckEntities],
                builder_fn: |analyzer, _| {
                    Ok(SemanticQueryBuilder::new()
                        .with_concept(GameConcept::StuckEntities)
                        .with_thresholds_ref(&analyzer.thresholds))
                },
            },
            SemanticPattern {
                pattern: Regex::new(r"(?i)(?:show|find)\s+fast\s+moving\s+(?:objects?|entities?)")
                    .map_err(|e| Error::Validation(format!("Invalid regex pattern: {}", e)))?,
                concepts: vec![GameConcept::FastMovingObjects],
                builder_fn: |analyzer, _| {
                    Ok(SemanticQueryBuilder::new()
                        .with_concept(GameConcept::FastMovingObjects)
                        .with_thresholds_ref(&analyzer.thresholds))
                },
            },
            SemanticPattern {
                pattern: Regex::new(r"(?i)find\s+overlapping\s+colliders?")
                    .map_err(|e| Error::Validation(format!("Invalid regex pattern: {}", e)))?,
                concepts: vec![GameConcept::OverlappingColliders],
                builder_fn: |analyzer, _| {
                    Ok(SemanticQueryBuilder::new()
                        .with_concept(GameConcept::OverlappingColliders)
                        .with_thresholds_ref(&analyzer.thresholds))
                },
            },
            SemanticPattern {
                pattern: Regex::new(r"(?i)find\s+(?:memory\s+leaks?|leaked\s+entities?)")
                    .map_err(|e| Error::Validation(format!("Invalid regex pattern: {}", e)))?,
                concepts: vec![GameConcept::PotentialMemoryLeaks],
                builder_fn: |analyzer, _| {
                    Ok(SemanticQueryBuilder::new()
                        .with_concept(GameConcept::PotentialMemoryLeaks)
                        .with_thresholds_ref(&analyzer.thresholds))
                },
            },
            SemanticPattern {
                pattern: Regex::new(r"(?i)find\s+(?:inconsistent|invalid)\s+(?:state|entities?)")
                    .map_err(|e| Error::Validation(format!("Invalid regex pattern: {}", e)))?,
                concepts: vec![GameConcept::InconsistentState],
                builder_fn: |analyzer, _| {
                    Ok(SemanticQueryBuilder::new()
                        .with_concept(GameConcept::InconsistentState)
                        .with_thresholds_ref(&analyzer.thresholds))
                },
            },
            SemanticPattern {
                pattern: Regex::new(r"(?i)find\s+physics\s+violations?")
                    .map_err(|e| Error::Validation(format!("Invalid regex pattern: {}", e)))?,
                concepts: vec![GameConcept::PhysicsViolations],
                builder_fn: |analyzer, _| {
                    Ok(SemanticQueryBuilder::new()
                        .with_concept(GameConcept::PhysicsViolations)
                        .with_thresholds_ref(&analyzer.thresholds))
                },
            },
            // Compound queries
            SemanticPattern {
                pattern: Regex::new(r"(?i)find\s+stuck\s+(?:and|or)\s+fast\s+entities?")
                    .map_err(|e| Error::Validation(format!("Invalid regex pattern: {}", e)))?,
                concepts: vec![GameConcept::StuckEntities, GameConcept::FastMovingObjects],
                builder_fn: |analyzer, caps| {
                    let logic = if caps[0].to_lowercase().contains("and") {
                        QueryLogic::And
                    } else {
                        QueryLogic::Or
                    };
                    Ok(SemanticQueryBuilder::new()
                        .with_concept(GameConcept::StuckEntities)
                        .with_concept(GameConcept::FastMovingObjects)
                        .with_logic(logic)
                        .with_thresholds_ref(&analyzer.thresholds))
                },
            },
        ];

        Ok(Self {
            thresholds,
            patterns,
        })
    }

    /// Analyze a semantic query and return structured query result
    ///
    /// # Errors
    /// Returns error if query cannot be parsed or analyzed
    pub fn analyze(&self, query: &str) -> Result<SemanticQueryResult> {
        let query = query.trim();

        if query.is_empty() {
            return Err(Error::Brp("Empty semantic query".to_string()));
        }

        // Try to match against known semantic patterns
        for pattern in &self.patterns {
            if let Some(captures) = pattern.pattern.captures(query) {
                let builder = (pattern.builder_fn)(self, &captures)?;
                return builder.build();
            }
        }

        // Try fuzzy matching for component names
        if let Some(suggestion) = self.suggest_component_query(query) {
            return Err(Error::Brp(format!(
                "Unrecognized semantic query: '{query}'. Did you mean: '{suggestion}'?"
            )));
        }

        Err(Error::Brp(format!(
            "Unrecognized semantic query: '{}'. Available concepts: {}",
            query,
            self.available_concepts().join(", ")
        )))
    }

    fn suggest_component_query(&self, query: &str) -> Option<String> {
        let known_concepts = [
            "stuck entities",
            "fast moving objects",
            "overlapping colliders",
            "memory leaks",
            "inconsistent state",
            "physics violations",
        ];

        let mut best_match = None;
        let mut best_score = 0.0;

        for concept in &known_concepts {
            let score = jaro_winkler(query, concept);
            if score > f64::from(self.thresholds.fuzzy_match_threshold) && score > best_score {
                best_score = score;
                best_match = Some(*concept);
            }
        }

        best_match.map(|s| format!("find {s}"))
    }

    fn available_concepts(&self) -> Vec<String> {
        vec![
            "stuck entities".to_string(),
            "fast moving objects".to_string(),
            "overlapping colliders".to_string(),
            "memory leaks".to_string(),
            "inconsistent state".to_string(),
            "physics violations".to_string(),
        ]
    }

    /// Get current thresholds
    #[must_use]
    pub fn thresholds(&self) -> &SemanticThresholds {
        &self.thresholds
    }

    /// Update thresholds
    pub fn set_thresholds(&mut self, thresholds: SemanticThresholds) {
        self.thresholds = thresholds;
    }
}

impl Default for SemanticAnalyzer {
    fn default() -> Self {
        Self::new().expect("Default semantic analyzer should initialize successfully")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semantic_query_builder() {
        let builder = SemanticQueryBuilder::new()
            .with_concept(GameConcept::StuckEntities)
            .with_logic(QueryLogic::And);

        let result = builder.build().unwrap();
        assert_eq!(result.explanations.len(), 1);
        assert_eq!(result.explanations[0].concept, GameConcept::StuckEntities);
    }

    #[test]
    fn test_semantic_analyzer_stuck_entities() {
        let analyzer = SemanticAnalyzer::new().unwrap();
        let result = analyzer.analyze("find stuck entities").unwrap();

        assert_eq!(result.explanations.len(), 1);
        assert_eq!(result.explanations[0].concept, GameConcept::StuckEntities);
        assert!(!result.suggestions.is_empty());
    }

    #[test]
    fn test_semantic_analyzer_fast_moving() {
        let analyzer = SemanticAnalyzer::new().unwrap();
        let result = analyzer.analyze("show fast moving objects").unwrap();

        assert_eq!(result.explanations.len(), 1);
        assert_eq!(
            result.explanations[0].concept,
            GameConcept::FastMovingObjects
        );
    }

    #[test]
    fn test_compound_query() {
        let analyzer = SemanticAnalyzer::new().unwrap();
        let result = analyzer.analyze("find stuck and fast entities").unwrap();

        assert_eq!(result.explanations.len(), 2);
    }

    #[test]
    fn test_fuzzy_matching() {
        let analyzer = SemanticAnalyzer::new().unwrap();
        let result = analyzer.analyze("find stuk entities"); // Typo

        assert!(result.is_err());
        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("stuck entities"));
    }

    #[test]
    fn test_custom_thresholds() {
        let custom_thresholds = SemanticThresholds {
            stuck_velocity_threshold: 0.05,
            fast_velocity_threshold: 100.0,
            ..Default::default()
        };

        let analyzer = SemanticAnalyzer::with_thresholds(custom_thresholds).unwrap();
        let result = analyzer.analyze("find stuck entities").unwrap();

        assert!(result.explanations[0].reason.contains("0.05"));
    }

    #[test]
    fn test_game_concepts() {
        assert_eq!(
            GameConcept::StuckEntities.description(),
            "Entities that appear stuck (have target but low velocity)"
        );
        assert_eq!(
            GameConcept::FastMovingObjects.description(),
            "Entities moving faster than expected"
        );
    }
}
