use crate::server::brp_messages::{BrpRequest, QueryFilter};
use crate::server::error::{Error, Result};
/// Query parsing for natural language to BRP conversion
use regex::Regex;

/// A parsed query: the BRP request to send, and how many of the matching entities to return.
#[derive(Debug, Clone)]
pub struct ParsedQuery {
    /// The request the game answers.
    pub request: BrpRequest,
    /// At most this many result rows, for `find N entities ...`. `world.query` has no limit of
    /// its own, so whoever sends `request` cuts the rows.
    pub limit: Option<usize>,
}

impl From<BrpRequest> for ParsedQuery {
    fn from(request: BrpRequest) -> Self {
        Self { request, limit: None }
    }
}

/// Trait for parsing natural language queries into BRP requests
pub trait QueryParser {
    /// Parse a natural language query into a BRP request and a row limit
    ///
    /// # Errors
    /// Returns error if query cannot be parsed or is invalid
    fn parse(&self, query: &str) -> Result<ParsedQuery>;

    /// Get help text for supported query patterns
    fn help(&self) -> &str;
}

/// Basic pattern-based query parser using regex
pub struct RegexQueryParser {
    patterns: Vec<QueryPattern>,
    /// One line per pattern, built from the patterns so it lists exactly what `parse` accepts.
    help: String,
}

struct QueryPattern {
    pattern: Regex,
    builder: fn(&regex::Captures) -> Result<ParsedQuery>,
    description: &'static str,
}

// ---------------------------------------------------------------------------
// Request builders for the real Bevy 0.19 BRP method names
// ---------------------------------------------------------------------------

/// Build a `world.query` request from an optional filter.
fn make_query_request(filter: Option<QueryFilter>) -> BrpRequest {
    BrpRequest {
        method: crate::server::brp::builtin_methods::BRP_QUERY_METHOD.to_string(),
        id: None,
        params: Some(crate::server::brp::query_params(filter.as_ref(), true)),
    }
}

/// One component's registered type path: letters, digits, `_` and `::`, plus the generic arguments
/// some paths end in (`bevy_time::time::Time<bevy_time::real::Real>`, `Time<()>`). A digit is part
/// of the name: `bevy_mesh::components::Mesh3d`, `avian3d::...`. No whitespace, so `A B` is not one
/// name.
const PATH: &str = r"[A-Za-z0-9_:<>()]+";

/// A comma-separated list of [`PATH`]s. Whitespace is allowed, because a list reads `A, B` and a
/// generic argument list reads `Pair<A, B>`.
const PATH_LIST: &str = r"[A-Za-z0-9_:<>(),\s]+";

/// Split `list` at the commas that separate components. A comma inside `<..>` or `(..)` belongs to
/// one path's generic arguments (`Pair<a::A, b::B>`), so it is not a separator.
fn split_component_list(list: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (at, ch) in list.char_indices() {
        match ch {
            '<' | '(' => depth += 1,
            '>' | ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                names.push(&list[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    names.push(&list[start..]);
    names.into_iter().map(str::trim).filter(|name| !name.is_empty()).map(str::to_string).collect()
}

/// Build a `world.get_components` request.
fn make_get_request(entity: u64, components: Option<Vec<String>>) -> BrpRequest {
    BrpRequest {
        method: crate::server::brp::builtin_methods::BRP_GET_COMPONENTS_METHOD.to_string(),
        id: None,
        params: Some(serde_json::json!({
            "entity": entity,
            "components": components.unwrap_or_default(),
            "strict": false
        })),
    }
}

/// Build a `world.list_components` request.
pub(crate) fn make_list_components_request() -> BrpRequest {
    BrpRequest {
        method: crate::server::brp::builtin_methods::BRP_LIST_COMPONENTS_METHOD.to_string(),
        id: None,
        params: None,
    }
}

impl RegexQueryParser {
    /// Create a new regex-based query parser
    /// 
    /// # Errors
    /// Returns error if any regex pattern fails to compile
    pub fn new() -> Result<Self> {
        let patterns = vec![
            // List all entities
            QueryPattern {
                pattern: Regex::new(r"^(?i)list\s+all\s+entities?$")
                    .map_err(|e| Error::Validation(format!("Invalid regex pattern: {}", e)))?,
                builder: |_| Ok(make_query_request(None).into()),
                description: "list all entities - List all entities in the game",
            },

            // Show specific entity
            QueryPattern {
                pattern: Regex::new(r"^(?i)show\s+entity\s+(\d+)$")
                    .map_err(|e| Error::Validation(format!("Invalid regex pattern: {}", e)))?,
                builder: |caps| {
                    let entity_id = caps[1].parse::<u64>()
                        .map_err(|_| Error::Brp("Invalid entity ID".to_string()))?;
                    Ok(make_get_request(entity_id, None).into())
                },
                description: "show entity X - Show details for entity with ID X",
            },

            // Find entities with component
            QueryPattern {
                pattern: Regex::new(&format!(r"^(?i)find\s+entities\s+with\s+component\s+({PATH})$"))
                    .map_err(|e| Error::Validation(format!("Invalid regex pattern: {}", e)))?,
                builder: |caps| {
                    let component_type = caps[1].to_string();
                    Ok(make_query_request(Some(QueryFilter {
                        with: Some(vec![component_type]),
                        without: None,
                        where_clause: None,
                    })).into())
                },
                description: "find entities with component Y - Find all entities that have component Y",
            },

            // Find entities without component
            QueryPattern {
                pattern: Regex::new(&format!(r"^(?i)find\s+entities\s+without\s+component\s+({PATH})$"))
                    .map_err(|e| Error::Validation(format!("Invalid regex pattern: {}", e)))?,
                builder: |caps| {
                    let component_type = caps[1].to_string();
                    Ok(make_query_request(Some(QueryFilter {
                        with: None,
                        without: Some(vec![component_type]),
                        where_clause: None,
                    })).into())
                },
                description: "find entities without component Y - Find all entities that don't have component Y",
            },

            // List component types
            QueryPattern {
                pattern: Regex::new(r"^(?i)list\s+components?$")
                    .map_err(|e| Error::Validation(format!("Invalid regex pattern: {}", e)))?,
                builder: |_| Ok(make_list_components_request().into()),
                description: "list components - List all available component types",
            },

            // Find entities with multiple components
            QueryPattern {
                pattern: Regex::new(&format!(r"^(?i)find\s+entities\s+with\s+(?:components?\s+)?({PATH_LIST})$"))
                    .map_err(|e| Error::Validation(format!("Invalid regex pattern: {}", e)))?,
                builder: |caps| {
                    let components = split_component_list(&caps[1]);

                    if components.is_empty() {
                        return Err(Error::Brp("No components specified".to_string()));
                    }

                    Ok(make_query_request(Some(QueryFilter {
                        with: Some(components),
                        without: None,
                        where_clause: None,
                    })).into())
                },
                description: "find entities with A, B, C - Find entities that have all specified components",
            },

            // Get entity with specific components only
            QueryPattern {
                pattern: Regex::new(&format!(r"^(?i)show\s+entity\s+(\d+)\s+components?\s+({PATH_LIST})$"))
                    .map_err(|e| Error::Validation(format!("Invalid regex pattern: {}", e)))?,
                builder: |caps| {
                    let entity_id = caps[1].parse::<u64>()
                        .map_err(|_| Error::Brp("Invalid entity ID".to_string()))?;
                    let components = split_component_list(&caps[2]);

                    Ok(make_get_request(
                        entity_id,
                        if components.is_empty() { None } else { Some(components) },
                    ).into())
                },
                description: "show entity X components A, B - Show specific components of entity X",
            },

            // Query with limit
            QueryPattern {
                pattern: Regex::new(&format!(r"^(?i)find\s+(\d+)\s+entities\s+with\s+component\s+({PATH})$"))
                    .map_err(|e| Error::Validation(format!("Invalid regex pattern: {}", e)))?,
                builder: |caps| {
                    let limit = caps[1].parse::<usize>()
                        .ok()
                        .filter(|limit| *limit > 0)
                        .ok_or_else(|| Error::Brp(format!("Invalid limit '{}': ask for at least 1 entity", &caps[1])))?;
                    let component_type = caps[2].to_string();
                    Ok(ParsedQuery {
                        request: make_query_request(Some(QueryFilter {
                            with: Some(vec![component_type]),
                            without: None,
                            where_clause: None,
                        })),
                        limit: Some(limit),
                    })
                },
                description: "find N entities with component Y - Find up to N entities with component Y",
            },
        ];

        let mut help = String::from("Supported query patterns:");
        for pattern in &patterns {
            help.push_str("\n- ");
            help.push_str(pattern.description);
        }
        Ok(Self { patterns, help })
    }
}

impl Default for RegexQueryParser {
    fn default() -> Self {
        Self::new().expect("Default regex patterns should be valid")
    }
}

impl QueryParser for RegexQueryParser {
    fn parse(&self, query: &str) -> Result<ParsedQuery> {
        let query = query.trim();

        if query.is_empty() {
            return Err(Error::Brp("Empty query".to_string()));
        }

        for pattern in &self.patterns {
            if let Some(captures) = pattern.pattern.captures(query) {
                return (pattern.builder)(&captures);
            }
        }

        Err(Error::Brp(format!(
            "Unrecognized query pattern: '{}'. {}",
            query,
            self.help()
        )))
    }

    fn help(&self) -> &str {
        &self.help
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_query_parsing() {
        let parser = RegexQueryParser::new().unwrap();

        // Test list all entities
        let result = parser.parse("list all entities").unwrap().request;
        assert_eq!(result.method, crate::server::brp::builtin_methods::BRP_QUERY_METHOD);

        // Test show entity
        let result = parser.parse("show entity 123").unwrap().request;
        assert_eq!(result.method, crate::server::brp::builtin_methods::BRP_GET_COMPONENTS_METHOD);
        let entity = result
            .params
            .as_ref()
            .and_then(|p| p.get("entity"))
            .and_then(|e| e.as_u64())
            .unwrap();
        assert_eq!(entity, 123);

        // Test find with component
        let result = parser
            .parse("find entities with component Transform")
            .unwrap()
            .request;
        assert_eq!(result.method, crate::server::brp::builtin_methods::BRP_QUERY_METHOD);
        let with = result
            .params
            .as_ref()
            .and_then(|p| p.get("filter"))
            .and_then(|f| f.get("with"))
            .and_then(|w| w.as_array())
            .unwrap();
        assert_eq!(with, &vec![serde_json::json!("Transform")]);
    }

    #[test]
    fn test_case_insensitive_parsing() {
        let parser = RegexQueryParser::new().unwrap();

        let result1 = parser.parse("LIST ALL ENTITIES").unwrap().request;
        let result2 = parser.parse("list all entities").unwrap().request;

        assert_eq!(result1.method, crate::server::brp::builtin_methods::BRP_QUERY_METHOD);
        assert_eq!(result2.method, crate::server::brp::builtin_methods::BRP_QUERY_METHOD);
    }

    #[test]
    fn test_invalid_queries() {
        let parser = RegexQueryParser::new().unwrap();

        assert!(parser.parse("").is_err());
        assert!(parser.parse("invalid query").is_err());
        assert!(parser.parse("show entity abc").is_err()); // Invalid entity ID
        assert!(parser.parse("find 0 entities with component Transform").is_err());
    }

    #[test]
    fn test_multiple_components_query() {
        let parser = RegexQueryParser::new().unwrap();

        let result = parser
            .parse("find entities with Transform, Velocity, Health")
            .unwrap()
            .request;
        assert_eq!(result.method, crate::server::brp::builtin_methods::BRP_QUERY_METHOD);
        let with = result
            .params
            .as_ref()
            .and_then(|p| p.get("filter"))
            .and_then(|f| f.get("with"))
            .and_then(|w| w.as_array())
            .unwrap();
        assert_eq!(
            with,
            &vec![
                serde_json::json!("Transform"),
                serde_json::json!("Velocity"),
                serde_json::json!("Health"),
            ]
        );
    }

    /// The component names a parsed query puts in the filter's `with` and `without` lists.
    fn filter_names(query: &str) -> Vec<String> {
        let parser = RegexQueryParser::new().unwrap();
        let request = parser.parse(query).unwrap_or_else(|e| panic!("{query}: {e}")).request;
        let params = request.params.expect("a filter query carries params");
        ["with", "without"]
            .iter()
            .flat_map(|kind| params["filter"][*kind].as_array().cloned().unwrap_or_default())
            .map(|name| name.as_str().expect("names are strings").to_string())
            .collect()
    }

    /// Bevy's own `Mesh3d`, `Camera3d` and `Mesh2d`, and every component of crates such as
    /// `avian3d`, have a digit in their registered path; the observe error tells the agent to use
    /// exactly that path, so the parser has to accept it (issue #14).
    #[test]
    fn component_paths_with_digits_parse() {
        for (query, expected) in [
            ("find entities with component bevy_mesh::components::Mesh3d", "bevy_mesh::components::Mesh3d"),
            (
                "find 2 entities with component avian3d::dynamics::rigid_body::LinearVelocity",
                "avian3d::dynamics::rigid_body::LinearVelocity",
            ),
            (
                "find entities without component bevy_camera::components::Camera3d",
                "bevy_camera::components::Camera3d",
            ),
            ("find entities with component Mesh3d", "Mesh3d"),
        ] {
            assert_eq!(filter_names(query), [expected], "{query}");
        }

        let parser = RegexQueryParser::new().unwrap();
        let shown = parser
            .parse("show entity 7 components bevy_mesh::components::Mesh3d")
            .expect("a digit in a path is allowed after `show entity N components`")
            .request;
        assert_eq!(shown.params.unwrap()["components"], serde_json::json!(["bevy_mesh::components::Mesh3d"]));
    }

    /// A registered path can end in generic arguments, `Time<bevy_time::real::Real>` or the unit
    /// `Time<()>`. A comma inside the angle brackets belongs to the path; a comma outside them
    /// separates two components.
    #[test]
    fn generic_component_paths_parse_and_only_top_level_commas_separate() {
        let real = "bevy_time::time::Time<bevy_time::real::Real>";
        assert_eq!(filter_names(&format!("find entities with component {real}")), [real]);
        assert_eq!(filter_names("find entities with component bevy_time::time::Time<()>"), ["bevy_time::time::Time<()>"]);

        let two = "some::Pair<a::A, b::B>";
        assert_eq!(filter_names(&format!("find entities with component {two}")), [two]);
        assert_eq!(
            filter_names(&format!("find entities with {two}, bevy_mesh::components::Mesh3d, {real}")),
            [two, "bevy_mesh::components::Mesh3d", real]
        );
    }
}
