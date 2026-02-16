::: exercise
id: in-memory-graph
difficulty: intermediate
time: 30 minutes
:::

# In-Memory Graph Storage

A knowledge graph is the backbone of Graph RAG -- it stores entities as nodes
and their relationships as directed edges. In this exercise you will implement
a simplified in-memory graph storage layer using HashMaps, mirroring the
`GraphStorage` trait that EdgeQuake uses to abstract over real backends like
Neptune and DynamoDB.

::: objectives
thinking:
  - Understand the node/edge data model for knowledge graphs
  - Reason about why adjacency lists enable efficient graph traversal
doing:
  - Implement upsert_node with insert-or-update semantics
  - Implement upsert_edge with bidirectional adjacency tracking
  - Implement get_neighbors with optional edge-label filtering
  - Handle edge cases like missing nodes and duplicate edges
:::

::: discussion
- Why does Graph RAG use "upsert" instead of separate "insert" and "update" operations?
- What are the tradeoffs between adjacency-list and adjacency-matrix representations?
- How would you extend this to support undirected edges or hyperedges?
:::

::: starter file="src/main.rs"
```rust
use std::collections::{HashMap, HashSet};

/// A node in the knowledge graph.
#[derive(Debug, Clone)]
struct Node {
    id: String,
    label: String,
    properties: HashMap<String, String>,
}

/// A directed edge in the knowledge graph.
#[derive(Debug, Clone)]
struct Edge {
    id: String,
    source: String,
    target: String,
    label: String,
    properties: HashMap<String, String>,
}

/// A neighbor reference returned by get_neighbors.
#[derive(Debug, Clone)]
struct Neighbor {
    node_id: String,
    edge_id: String,
    edge_label: String,
}

/// In-memory graph storage using HashMaps.
///
/// Stores nodes by ID, edges by ID, and maintains adjacency lists
/// for efficient neighbor lookups.
struct InMemoryGraph {
    nodes: HashMap<String, Node>,
    edges: HashMap<String, Edge>,
    /// Adjacency list: node_id -> set of edge_ids going out from that node.
    outgoing: HashMap<String, HashSet<String>>,
    /// Adjacency list: node_id -> set of edge_ids coming in to that node.
    incoming: HashMap<String, HashSet<String>>,
}

impl InMemoryGraph {
    fn new() -> Self {
        InMemoryGraph {
            nodes: HashMap::new(),
            edges: HashMap::new(),
            outgoing: HashMap::new(),
            incoming: HashMap::new(),
        }
    }

    /// Insert or update a node.
    ///
    /// If a node with the same ID already exists, update its label and
    /// merge its properties (new properties overwrite existing ones with
    /// the same key, but existing keys not in the new node are preserved).
    fn upsert_node(&mut self, node: Node) {
        // TODO: Implement upsert_node
        // 1. Check if a node with this ID already exists.
        // 2. If yes, update label and merge properties.
        // 3. If no, insert the new node.
        // 4. Ensure adjacency lists have entries for this node.
        todo!("Implement upsert_node")
    }

    /// Insert or update an edge.
    ///
    /// If an edge with the same ID already exists, update its label and
    /// merge its properties. Also update the adjacency lists.
    fn upsert_edge(&mut self, edge: Edge) {
        // TODO: Implement upsert_edge
        // 1. If updating an existing edge, remove old adjacency entries
        //    if source/target changed.
        // 2. Insert or update the edge.
        // 3. Add the edge ID to outgoing[source] and incoming[target].
        todo!("Implement upsert_edge")
    }

    /// Get a node by its ID.
    fn get_node(&self, id: &str) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// Get an edge by its ID.
    fn get_edge(&self, id: &str) -> Option<&Edge> {
        self.edges.get(id)
    }

    /// Get all outgoing neighbors of a node, optionally filtered by edge label.
    ///
    /// Returns a list of Neighbor structs containing the neighbor node ID,
    /// the edge ID, and the edge label.
    fn get_neighbors(&self, node_id: &str, edge_label: Option<&str>) -> Vec<Neighbor> {
        // TODO: Implement get_neighbors
        // 1. Look up the outgoing edge IDs for this node.
        // 2. For each edge, look up the edge details.
        // 3. If edge_label filter is Some, only include edges matching that label.
        // 4. Return Neighbor structs with (target node_id, edge_id, edge_label).
        // 5. Return empty vec if the node doesn't exist or has no outgoing edges.
        todo!("Implement get_neighbors")
    }

    /// Get the total number of nodes.
    fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Get the total number of edges.
    fn edge_count(&self) -> usize {
        self.edges.len()
    }
}

fn main() {
    let mut graph = InMemoryGraph::new();

    // Add some entities
    graph.upsert_node(Node {
        id: "person:alice".into(),
        label: "Person".into(),
        properties: HashMap::from([("name".into(), "Alice".into())]),
    });
    graph.upsert_node(Node {
        id: "org:acme".into(),
        label: "Organization".into(),
        properties: HashMap::from([("name".into(), "Acme Corp".into())]),
    });
    graph.upsert_node(Node {
        id: "person:bob".into(),
        label: "Person".into(),
        properties: HashMap::from([("name".into(), "Bob".into())]),
    });

    // Add relationships
    graph.upsert_edge(Edge {
        id: "e1".into(),
        source: "person:alice".into(),
        target: "org:acme".into(),
        label: "WORKS_AT".into(),
        properties: HashMap::from([("since".into(), "2020".into())]),
    });
    graph.upsert_edge(Edge {
        id: "e2".into(),
        source: "person:bob".into(),
        target: "org:acme".into(),
        label: "WORKS_AT".into(),
        properties: HashMap::new(),
    });
    graph.upsert_edge(Edge {
        id: "e3".into(),
        source: "person:alice".into(),
        target: "person:bob".into(),
        label: "KNOWS".into(),
        properties: HashMap::new(),
    });

    println!("Graph has {} nodes and {} edges", graph.node_count(), graph.edge_count());

    let neighbors = graph.get_neighbors("person:alice", None);
    println!("\nAlice's neighbors (all):");
    for n in &neighbors {
        println!("  --[{}]--> {}", n.edge_label, n.node_id);
    }

    let work_neighbors = graph.get_neighbors("person:alice", Some("WORKS_AT"));
    println!("\nAlice's WORKS_AT neighbors:");
    for n in &work_neighbors {
        println!("  --[{}]--> {}", n.edge_label, n.node_id);
    }
}
```
:::

::: hint level=1 title="Upsert node with property merging"
For merging properties, use `extend` to overwrite matching keys while keeping
unmatched ones:

```rust
fn upsert_node(&mut self, node: Node) {
    if let Some(existing) = self.nodes.get_mut(&node.id) {
        existing.label = node.label;
        existing.properties.extend(node.properties);
    } else {
        let id = node.id.clone();
        self.nodes.insert(id.clone(), node);
        self.outgoing.entry(id.clone()).or_insert_with(HashSet::new);
        self.incoming.entry(id).or_insert_with(HashSet::new);
    }
}
```
:::

::: hint level=2 title="Get neighbors with optional filter"
Look up the outgoing edges, resolve each edge ID, and apply the optional filter:

```rust
fn get_neighbors(&self, node_id: &str, edge_label: Option<&str>) -> Vec<Neighbor> {
    let Some(edge_ids) = self.outgoing.get(node_id) else {
        return Vec::new();
    };

    edge_ids.iter()
        .filter_map(|eid| self.edges.get(eid))
        .filter(|edge| edge_label.map_or(true, |label| edge.label == label))
        .map(|edge| Neighbor {
            node_id: edge.target.clone(),
            edge_id: edge.id.clone(),
            edge_label: edge.label.clone(),
        })
        .collect()
}
```
:::

::: solution
```rust
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
struct Node {
    id: String,
    label: String,
    properties: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct Edge {
    id: String,
    source: String,
    target: String,
    label: String,
    properties: HashMap<String, String>,
}

#[derive(Debug, Clone)]
struct Neighbor {
    node_id: String,
    edge_id: String,
    edge_label: String,
}

struct InMemoryGraph {
    nodes: HashMap<String, Node>,
    edges: HashMap<String, Edge>,
    outgoing: HashMap<String, HashSet<String>>,
    incoming: HashMap<String, HashSet<String>>,
}

impl InMemoryGraph {
    fn new() -> Self {
        InMemoryGraph {
            nodes: HashMap::new(),
            edges: HashMap::new(),
            outgoing: HashMap::new(),
            incoming: HashMap::new(),
        }
    }

    fn upsert_node(&mut self, node: Node) {
        if let Some(existing) = self.nodes.get_mut(&node.id) {
            existing.label = node.label;
            existing.properties.extend(node.properties);
        } else {
            let id = node.id.clone();
            self.nodes.insert(id.clone(), node);
            self.outgoing.entry(id.clone()).or_insert_with(HashSet::new);
            self.incoming.entry(id).or_insert_with(HashSet::new);
        }
    }

    fn upsert_edge(&mut self, edge: Edge) {
        // If updating an existing edge, clean up old adjacency entries
        if let Some(existing) = self.edges.get(&edge.id) {
            if existing.source != edge.source || existing.target != edge.target {
                // Source or target changed, remove old adjacency entries
                if let Some(out) = self.outgoing.get_mut(&existing.source) {
                    out.remove(&edge.id);
                }
                if let Some(inc) = self.incoming.get_mut(&existing.target) {
                    inc.remove(&edge.id);
                }
            }
        }

        // Update adjacency lists
        self.outgoing
            .entry(edge.source.clone())
            .or_insert_with(HashSet::new)
            .insert(edge.id.clone());
        self.incoming
            .entry(edge.target.clone())
            .or_insert_with(HashSet::new)
            .insert(edge.id.clone());

        // Insert or update the edge
        if let Some(existing) = self.edges.get_mut(&edge.id) {
            existing.source = edge.source;
            existing.target = edge.target;
            existing.label = edge.label;
            existing.properties.extend(edge.properties);
        } else {
            self.edges.insert(edge.id.clone(), edge);
        }
    }

    fn get_node(&self, id: &str) -> Option<&Node> {
        self.nodes.get(id)
    }

    fn get_edge(&self, id: &str) -> Option<&Edge> {
        self.edges.get(id)
    }

    fn get_neighbors(&self, node_id: &str, edge_label: Option<&str>) -> Vec<Neighbor> {
        let Some(edge_ids) = self.outgoing.get(node_id) else {
            return Vec::new();
        };

        edge_ids
            .iter()
            .filter_map(|eid| self.edges.get(eid))
            .filter(|edge| edge_label.map_or(true, |label| edge.label == label))
            .map(|edge| Neighbor {
                node_id: edge.target.clone(),
                edge_id: edge.id.clone(),
                edge_label: edge.label.clone(),
            })
            .collect()
    }

    fn node_count(&self) -> usize {
        self.nodes.len()
    }

    fn edge_count(&self) -> usize {
        self.edges.len()
    }
}

fn main() {
    let mut graph = InMemoryGraph::new();

    graph.upsert_node(Node {
        id: "person:alice".into(),
        label: "Person".into(),
        properties: HashMap::from([("name".into(), "Alice".into())]),
    });
    graph.upsert_node(Node {
        id: "org:acme".into(),
        label: "Organization".into(),
        properties: HashMap::from([("name".into(), "Acme Corp".into())]),
    });
    graph.upsert_node(Node {
        id: "person:bob".into(),
        label: "Person".into(),
        properties: HashMap::from([("name".into(), "Bob".into())]),
    });

    graph.upsert_edge(Edge {
        id: "e1".into(),
        source: "person:alice".into(),
        target: "org:acme".into(),
        label: "WORKS_AT".into(),
        properties: HashMap::from([("since".into(), "2020".into())]),
    });
    graph.upsert_edge(Edge {
        id: "e2".into(),
        source: "person:bob".into(),
        target: "org:acme".into(),
        label: "WORKS_AT".into(),
        properties: HashMap::new(),
    });
    graph.upsert_edge(Edge {
        id: "e3".into(),
        source: "person:alice".into(),
        target: "person:bob".into(),
        label: "KNOWS".into(),
        properties: HashMap::new(),
    });

    println!("Graph has {} nodes and {} edges", graph.node_count(), graph.edge_count());

    let neighbors = graph.get_neighbors("person:alice", None);
    println!("\nAlice's neighbors (all):");
    for n in &neighbors {
        println!("  --[{}]--> {}", n.edge_label, n.node_id);
    }

    let work_neighbors = graph.get_neighbors("person:alice", Some("WORKS_AT"));
    println!("\nAlice's WORKS_AT neighbors:");
    for n in &work_neighbors {
        println!("  --[{}]--> {}", n.edge_label, n.node_id);
    }
}
```

### Explanation

The graph uses four HashMaps: `nodes` and `edges` for the primary data, and
`outgoing`/`incoming` adjacency lists for efficient traversal. The adjacency
lists map each node ID to the set of edge IDs connected to it.

**Upsert semantics**: When upserting a node, existing properties are preserved
unless overwritten by the new node's properties (via `extend`). When upserting
an edge, if the source or target changes, the old adjacency entries are cleaned
up before inserting new ones.

**Neighbor lookup**: `get_neighbors` is O(degree) because it only iterates
over the outgoing edges of the given node, not all edges in the graph. The
optional `edge_label` filter uses `filter` to restrict results, enabling
queries like "get all WORKS_AT relationships" efficiently.
:::

::: tests mode=local
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upsert_node_insert() {
        let mut graph = InMemoryGraph::new();
        graph.upsert_node(Node {
            id: "n1".into(),
            label: "Person".into(),
            properties: HashMap::from([("name".into(), "Alice".into())]),
        });
        assert_eq!(graph.node_count(), 1);
        let node = graph.get_node("n1").unwrap();
        assert_eq!(node.label, "Person");
        assert_eq!(node.properties.get("name").unwrap(), "Alice");
    }

    #[test]
    fn test_upsert_node_update_merges_properties() {
        let mut graph = InMemoryGraph::new();
        graph.upsert_node(Node {
            id: "n1".into(),
            label: "Person".into(),
            properties: HashMap::from([
                ("name".into(), "Alice".into()),
                ("age".into(), "30".into()),
            ]),
        });
        graph.upsert_node(Node {
            id: "n1".into(),
            label: "Employee".into(),
            properties: HashMap::from([("role".into(), "Engineer".into())]),
        });

        assert_eq!(graph.node_count(), 1, "Upsert should not create a duplicate");
        let node = graph.get_node("n1").unwrap();
        assert_eq!(node.label, "Employee", "Label should be updated");
        assert_eq!(node.properties.get("name").unwrap(), "Alice", "Existing property preserved");
        assert_eq!(node.properties.get("age").unwrap(), "30", "Existing property preserved");
        assert_eq!(node.properties.get("role").unwrap(), "Engineer", "New property added");
    }

    #[test]
    fn test_upsert_edge() {
        let mut graph = InMemoryGraph::new();
        graph.upsert_node(Node { id: "a".into(), label: "".into(), properties: HashMap::new() });
        graph.upsert_node(Node { id: "b".into(), label: "".into(), properties: HashMap::new() });
        graph.upsert_edge(Edge {
            id: "e1".into(),
            source: "a".into(),
            target: "b".into(),
            label: "KNOWS".into(),
            properties: HashMap::new(),
        });

        assert_eq!(graph.edge_count(), 1);
        let edge = graph.get_edge("e1").unwrap();
        assert_eq!(edge.source, "a");
        assert_eq!(edge.target, "b");
        assert_eq!(edge.label, "KNOWS");
    }

    #[test]
    fn test_get_neighbors_all() {
        let mut graph = InMemoryGraph::new();
        graph.upsert_node(Node { id: "a".into(), label: "".into(), properties: HashMap::new() });
        graph.upsert_node(Node { id: "b".into(), label: "".into(), properties: HashMap::new() });
        graph.upsert_node(Node { id: "c".into(), label: "".into(), properties: HashMap::new() });
        graph.upsert_edge(Edge {
            id: "e1".into(), source: "a".into(), target: "b".into(),
            label: "KNOWS".into(), properties: HashMap::new(),
        });
        graph.upsert_edge(Edge {
            id: "e2".into(), source: "a".into(), target: "c".into(),
            label: "WORKS_WITH".into(), properties: HashMap::new(),
        });

        let neighbors = graph.get_neighbors("a", None);
        assert_eq!(neighbors.len(), 2, "Should return both neighbors");
    }

    #[test]
    fn test_get_neighbors_filtered() {
        let mut graph = InMemoryGraph::new();
        graph.upsert_node(Node { id: "a".into(), label: "".into(), properties: HashMap::new() });
        graph.upsert_node(Node { id: "b".into(), label: "".into(), properties: HashMap::new() });
        graph.upsert_node(Node { id: "c".into(), label: "".into(), properties: HashMap::new() });
        graph.upsert_edge(Edge {
            id: "e1".into(), source: "a".into(), target: "b".into(),
            label: "KNOWS".into(), properties: HashMap::new(),
        });
        graph.upsert_edge(Edge {
            id: "e2".into(), source: "a".into(), target: "c".into(),
            label: "WORKS_WITH".into(), properties: HashMap::new(),
        });

        let knows = graph.get_neighbors("a", Some("KNOWS"));
        assert_eq!(knows.len(), 1);
        assert_eq!(knows[0].node_id, "b");
        assert_eq!(knows[0].edge_label, "KNOWS");
    }

    #[test]
    fn test_get_neighbors_nonexistent_node() {
        let graph = InMemoryGraph::new();
        let neighbors = graph.get_neighbors("nonexistent", None);
        assert!(neighbors.is_empty(), "Nonexistent node should return empty neighbors");
    }

    #[test]
    fn test_directed_edges() {
        let mut graph = InMemoryGraph::new();
        graph.upsert_node(Node { id: "a".into(), label: "".into(), properties: HashMap::new() });
        graph.upsert_node(Node { id: "b".into(), label: "".into(), properties: HashMap::new() });
        graph.upsert_edge(Edge {
            id: "e1".into(), source: "a".into(), target: "b".into(),
            label: "FOLLOWS".into(), properties: HashMap::new(),
        });

        let a_neighbors = graph.get_neighbors("a", None);
        let b_neighbors = graph.get_neighbors("b", None);
        assert_eq!(a_neighbors.len(), 1, "a should have 1 outgoing neighbor");
        assert_eq!(b_neighbors.len(), 0, "b should have 0 outgoing neighbors (edge is directed)");
    }
}
```
:::

::: reflection
- How would you add a `get_incoming_neighbors` method? When would you need it in a Graph RAG query?
- What happens to the adjacency lists if you delete a node that still has edges? How would you handle cascading deletes?
- How does this in-memory implementation compare to a real graph database like Neptune in terms of query capabilities?
:::
