//! Helper utilities for Gremlin query construction.
//!
//! This module provides helper functions for building Gremlin traversals
//! and converting between EdgeQuake types and Gremlin types.

use serde_json::Value as JsonValue;
use std::collections::HashMap;

/// Gremlin query builder for common graph operations.
pub struct GremlinQueryBuilder {
    namespace: String,
}

impl GremlinQueryBuilder {
    /// Create a new query builder for a namespace.
    pub fn new(namespace: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
        }
    }

    /// Build a query to add or update a vertex.
    ///
    /// Uses `fold().coalesce()` pattern for upsert semantics.
    pub fn upsert_vertex(&self, id: &str, properties: &HashMap<String, JsonValue>) -> String {
        let props = self.properties_chain(properties);

        format!(
            "g.V().has('id', '{}').has('namespace', '{}').fold().coalesce(\
                unfold().sideEffect(__.properties().drop()){},\
                addV('Entity').property('id', '{}'){})",
            id, self.namespace, props, id, props
        )
    }

    /// Build a query to add an edge between two vertices.
    ///
    /// Ensures both vertices exist before creating edge.
    pub fn upsert_edge(
        &self,
        source: &str,
        target: &str,
        properties: &HashMap<String, JsonValue>,
    ) -> String {
        let props = self.properties_chain(properties);

        format!(
            "g.V().has('id', '{}').has('namespace', '{}').addE('relates_to')\
                .to(g.V().has('id', '{}').has('namespace', '{}')){}",
            source, self.namespace, target, self.namespace, props
        )
    }

    /// Build a query to get a vertex by ID.
    pub fn get_vertex(&self, id: &str) -> String {
        format!(
            "g.V().has('id', '{}').has('namespace', '{}').elementMap()",
            id, self.namespace
        )
    }

    /// Build a query to delete a vertex and its edges.
    pub fn delete_vertex(&self, id: &str) -> String {
        format!(
            "g.V().has('id', '{}').has('namespace', '{}').drop()",
            id, self.namespace
        )
    }

    /// Build a query to get edges for a node.
    pub fn get_node_edges(&self, node_id: &str) -> String {
        format!(
            "g.V().has('id', '{}').has('namespace', '{}').bothE().elementMap()",
            node_id, self.namespace
        )
    }

    /// Build a query to traverse neighbors up to a depth.
    pub fn traverse_neighbors(&self, start_id: &str, depth: usize, limit: usize) -> String {
        format!(
            "g.V().has('id', '{}').has('namespace', '{}').repeat(both().simplePath()).times({}).limit({}).dedup().elementMap()",
            start_id, self.namespace, depth, limit
        )
    }

    /// Build a query to get node degree (edge count).
    pub fn node_degree(&self, node_id: &str) -> String {
        format!(
            "g.V().has('id', '{}').has('namespace', '{}').bothE().count()",
            node_id, self.namespace
        )
    }

    /// Build a query to get all vertices in namespace.
    pub fn get_all_vertices(&self) -> String {
        format!("g.V().has('namespace', '{}').elementMap()", self.namespace)
    }

    /// Build a query to get all edges in namespace.
    pub fn get_all_edges(&self) -> String {
        format!(
            "g.E().where(outV().has('namespace', '{}')).elementMap()",
            self.namespace
        )
    }

    /// Build a query to count vertices.
    pub fn count_vertices(&self) -> String {
        format!("g.V().has('namespace', '{}').count()", self.namespace)
    }

    /// Build a query to count edges.
    pub fn count_edges(&self) -> String {
        format!(
            "g.E().where(outV().has('namespace', '{}')).count()",
            self.namespace
        )
    }

    /// Build a query for knowledge graph extraction (BFS with depth limit).
    pub fn extract_subgraph(&self, start_id: &str, max_depth: usize, max_nodes: usize) -> String {
        format!(
            "g.V().has('id', '{}').has('namespace', '{}').repeat(both().simplePath()).times({}).limit({}).path().by(elementMap())",
            start_id, self.namespace, max_depth, max_nodes
        )
    }

    /// Build a query to get popular nodes (by degree).
    pub fn get_popular_nodes(&self, limit: usize) -> String {
        format!(
            "g.V().has('namespace', '{}').project('node', 'degree')\
                .by(elementMap())\
                .by(bothE().count())\
                .order().by(select('degree'), desc)\
                .limit({})",
            self.namespace, limit
        )
    }

    /// Build a query to search nodes by text.
    pub fn search_nodes(&self, limit: usize, entity_type: Option<&str>) -> String {
        let mut query = format!("g.V().has('namespace', '{}')", self.namespace);

        if let Some(et) = entity_type {
            query.push_str(&format!(".has('entity_type', '{}')", et));
        }

        query.push_str(&format!(
            ".limit({}).project('node', 'degree').by(elementMap()).by(bothE().count())",
            limit
        ));

        query
    }

    /// Build a query to get edges for a set of nodes.
    pub fn get_edges_for_nodes(&self, node_ids: &[String]) -> String {
        let ids_list = node_ids
            .iter()
            .map(|id| format!("'{}'", id))
            .collect::<Vec<_>>()
            .join(",");

        format!(
            "g.V().has('namespace', '{}').has('id', within({}))\
                .outE().where(inV().has('id', within({})))\
                .elementMap()",
            self.namespace, ids_list, ids_list
        )
    }

    /// Convert properties HashMap to Gremlin property chain.
    fn properties_chain(&self, properties: &HashMap<String, JsonValue>) -> String {
        let mut parts = vec![format!(".property('namespace', '{}')", self.namespace)];

        for (key, value) in properties {
            let value_str = self.json_to_gremlin_value(value);
            parts.push(format!(".property('{}', {})", key, value_str));
        }

        parts.join("")
    }

    /// Convert JSON value to Gremlin value string.
    fn json_to_gremlin_value(&self, value: &JsonValue) -> String {
        match value {
            JsonValue::String(s) => format!("'{}'", s.replace('\'', "\\'")),
            JsonValue::Number(n) => n.to_string(),
            JsonValue::Bool(b) => b.to_string(),
            JsonValue::Null => "null".to_string(),
            _ => {
                // For complex types, serialize as JSON string
                format!(
                    "'{}'",
                    serde_json::to_string(value)
                        .unwrap_or_default()
                        .replace('\'', "\\'")
                )
            }
        }
    }
}

/// Helper to escape special characters in Gremlin strings.
pub fn escape_gremlin_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\'', "\\'")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Helper to build a Gremlin "within" clause from a list of values.
pub fn gremlin_within(values: &[String]) -> String {
    let escaped: Vec<_> = values
        .iter()
        .map(|v| format!("'{}'", escape_gremlin_string(v)))
        .collect();

    format!("within({})", escaped.join(","))
}

/// Parse Neptune property value from GraphSON format.
///
/// Neptune returns properties as arrays of objects: `[{"value": "foo"}]`
pub fn parse_neptune_property(prop: &JsonValue) -> Option<JsonValue> {
    prop.as_array()?.first()?.get("value").cloned()
}

/// Parse all properties from a Neptune vertex/edge.
pub fn parse_neptune_properties(props: &JsonValue) -> HashMap<String, JsonValue> {
    let mut result = HashMap::new();

    if let Some(obj) = props.as_object() {
        for (key, value) in obj {
            if let Some(parsed) = parse_neptune_property(value) {
                result.insert(key.clone(), parsed);
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape_gremlin_string() {
        assert_eq!(escape_gremlin_string("hello"), "hello");
        assert_eq!(escape_gremlin_string("it's"), "it\\'s");
        assert_eq!(escape_gremlin_string("line\nbreak"), "line\\nbreak");
        assert_eq!(escape_gremlin_string("tab\there"), "tab\\there");
    }

    #[test]
    fn test_gremlin_within() {
        let values = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let result = gremlin_within(&values);
        assert_eq!(result, "within('a','b','c')");
    }

    #[test]
    fn test_query_builder_upsert_vertex() {
        let builder = GremlinQueryBuilder::new("test-ns");
        let mut props = HashMap::new();
        props.insert("name".to_string(), JsonValue::String("Alice".to_string()));
        props.insert("age".to_string(), JsonValue::Number(30.into()));

        let query = builder.upsert_vertex("user1", &props);
        assert!(query.contains("g.V().has('id', 'user1')"));
        assert!(query.contains("namespace', 'test-ns'"));
        assert!(query.contains("property('name', 'Alice')"));
        assert!(query.contains("property('age', 30)"));
    }

    #[test]
    fn test_query_builder_get_vertex() {
        let builder = GremlinQueryBuilder::new("test-ns");
        let query = builder.get_vertex("user1");
        assert_eq!(
            query,
            "g.V().has('id', 'user1').has('namespace', 'test-ns').elementMap()"
        );
    }

    #[test]
    fn test_parse_neptune_property() {
        let prop = serde_json::json!([{"value": "test"}]);
        let result = parse_neptune_property(&prop);
        assert_eq!(result, Some(JsonValue::String("test".to_string())));
    }

    #[test]
    fn test_parse_neptune_properties() {
        let props = serde_json::json!({
            "name": [{"value": "Alice"}],
            "age": [{"value": 30}]
        });

        let result = parse_neptune_properties(&props);
        assert_eq!(result.len(), 2);
        assert_eq!(
            result.get("name"),
            Some(&JsonValue::String("Alice".to_string()))
        );
        assert_eq!(result.get("age"), Some(&JsonValue::Number(30.into())));
    }
}
