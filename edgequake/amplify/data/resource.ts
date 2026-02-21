import { type ClientSchema, a } from '@aws-amplify/backend';
import { defineData } from '@aws-amplify/backend-data';

/**
 * Amplify Gen 2 data model for edgequake namespace management.
 *
 * Uses a.customType() (not a.model()) to define types that map to
 * the existing DynamoDB namespace table. Using a.model() would create
 * a new table and destroy existing data.
 *
 * These types mirror the Rust McpDescriptor types from Plan 01
 * (edgequake-core/src/mcp_descriptor.rs).
 */

const schema = a.schema({
  // --- Custom Types (mirror Rust structs) ---

  Namespace: a.customType({
    slug: a.string().required(),
    description: a.string(),
    created_at: a.integer().required(),
    created_by: a.string(),
  }),

  PipelineConfig: a.customType({
    llm_provider: a.string().required(),
    llm_model: a.string().required(),
    embedding_provider: a.string().required(),
    embedding_model: a.string().required(),
    embedding_dimension: a.integer().required(),
    chunking_strategy: a.string().required(),
    chunk_size: a.integer().required(),
    chunk_overlap: a.integer().required(),
    extraction_prompt: a.string(),
    entity_types: a.string().array(),
    relation_types: a.string().array(),
    updated_at: a.integer().required(),
  }),

  McpStorageConfig: a.customType({
    neptune_endpoint: a.string().required(),
    neptune_label_prefix: a.string().required(),
    s3_vectors_bucket_name: a.string().required(),
    s3_vectors_index_name: a.string().required(),
    dynamodb_table_name: a.string().required(),
    dynamodb_namespace_key: a.string().required(),
  }),

  McpAuthConfig: a.customType({
    role_arn: a.string().required(),
    external_id: a.string().required(),
    region: a.string().required(),
  }),

  McpToolDefinition: a.customType({
    name: a.string().required(),
    description: a.string().required(),
    modes: a.string().array(),
  }),

  McpDescriptor: a.customType({
    schema_version: a.string().required(),
    namespace_slug: a.string().required(),
    namespace_description: a.string(),
    storage: a.json().required(),
    auth: a.json().required(),
    pipeline_config: a.json().required(),
    tools: a.json().required(),
    generated_at: a.integer().required(),
  }),

  // --- Schema Proposal Types ---

  SchemaEntityType: a.customType({
    name: a.string().required(),
    description: a.string().required(),
    frequency: a.integer().required(),
    is_baseline: a.boolean().required(),
  }),

  SchemaRelationType: a.customType({
    name: a.string().required(),
    description: a.string().required(),
    source_type: a.string().required(),
    target_type: a.string().required(),
    frequency: a.integer().required(),
  }),

  SchemaProposal: a.customType({
    status: a.string().required(),
    entity_types: a.json().required(),
    relation_types: a.json().required(),
    sample_size: a.integer().required(),
    total_documents: a.integer().required(),
    domain_hint: a.string(),
    proposed_at: a.integer().required(),
    reviewed_at: a.integer(),
  }),

  // --- Pipeline Status Types ---

  NamespaceStatus: a.customType({
    status: a.string().required(),
    phase: a.string(),
    job_id: a.string(),
    total_documents: a.integer(),
    processed_documents: a.integer(),
    total_chunks: a.integer(),
    current_batch: a.integer(),
    total_batches: a.integer(),
    started_at: a.integer(),
    updated_at: a.integer(),
    error_summary: a.string(),
  }),

  IngestionRequest: a.customType({
    namespace: a.string().required(),
    batch_size: a.integer().required(),
    offset: a.integer().required(),
    data_path: a.string().required(),
    requested_at: a.integer().required(),
    status: a.string().required(),
    cli_command: a.string(),
  }),

  // --- Namespace Queries ---

  getNamespace: a
    .query()
    .arguments({ slug: a.string().required() })
    .returns(a.ref('Namespace'))
    .authorization((allow) => [allow.authenticated()])
    .handler(
      a.handler.custom({
        entry: './resolvers/get-namespace.js',
        dataSource: 'NamespaceTableDataSource',
      })
    ),

  listNamespaces: a
    .query()
    .returns(a.ref('Namespace').array())
    .authorization((allow) => [allow.authenticated()])
    .handler(
      a.handler.custom({
        entry: './resolvers/list-namespaces.js',
        dataSource: 'NamespaceTableDataSource',
      })
    ),

  getNamespaceDescriptor: a
    .query()
    .arguments({ slug: a.string().required() })
    .returns(a.ref('McpDescriptor'))
    .authorization((allow) => [allow.authenticated()])
    .handler(
      a.handler.custom({
        entry: './resolvers/get-namespace-descriptor.js',
        dataSource: 'NamespaceTableDataSource',
      })
    ),

  getNamespacePipelineConfig: a
    .query()
    .arguments({ slug: a.string().required() })
    .returns(a.ref('PipelineConfig'))
    .authorization((allow) => [allow.authenticated()])
    .handler(
      a.handler.custom({
        entry: './resolvers/get-pipeline-config.js',
        dataSource: 'NamespaceTableDataSource',
      })
    ),

  // --- Namespace Mutations ---

  createNamespace: a
    .mutation()
    .arguments({
      slug: a.string().required(),
      description: a.string(),
      domain_hint: a.string(),
    })
    .returns(a.ref('Namespace'))
    .authorization((allow) => [allow.authenticated()])
    .handler([
      a.handler.custom({
        entry: './resolvers/create-namespace.js',
        dataSource: 'NamespaceTableDataSource',
      }),
      a.handler.custom({
        entry: './resolvers/create-namespace-write.js',
        dataSource: 'NamespaceTableDataSource',
      }),
    ]),

  updatePipelineConfig: a
    .mutation()
    .arguments({
      slug: a.string().required(),
      config: a.json().required(),
    })
    .returns(a.ref('PipelineConfig'))
    .authorization((allow) => [allow.authenticated()])
    .handler([
      a.handler.custom({
        entry: './resolvers/update-pipeline-config.js',
        dataSource: 'NamespaceTableDataSource',
      }),
      a.handler.custom({
        entry: './resolvers/update-pipeline-config-write.js',
        dataSource: 'NamespaceTableDataSource',
      }),
    ]),

  // --- Pipeline Status and Ingestion ---

  getNamespaceStatus: a
    .query()
    .arguments({ slug: a.string().required() })
    .returns(a.ref('NamespaceStatus'))
    .authorization((allow) => [allow.authenticated()])
    .handler(
      a.handler.custom({
        entry: './resolvers/get-namespace-status.js',
        dataSource: 'NamespaceTableDataSource',
      })
    ),

  triggerIngestion: a
    .mutation()
    .arguments({
      slug: a.string().required(),
      batch_size: a.integer().required(),
      offset: a.integer().required(),
      data_path: a.string().required(),
    })
    .returns(a.ref('IngestionRequest'))
    .authorization((allow) => [allow.authenticated()])
    .handler([
      a.handler.custom({
        entry: './resolvers/trigger-ingestion-schema-gate.js',
        dataSource: 'NamespaceTableDataSource',
      }),
      a.handler.custom({
        entry: './resolvers/trigger-ingestion.js',
        dataSource: 'NamespaceTableDataSource',
      }),
      a.handler.custom({
        entry: './resolvers/trigger-ingestion-write.js',
        dataSource: 'NamespaceTableDataSource',
      }),
    ]),

  // --- Schema Proposal Operations ---

  getSchemaProposal: a
    .query()
    .arguments({ slug: a.string().required() })
    .returns(a.ref('SchemaProposal'))
    .authorization((allow) => [allow.authenticated()])
    .handler(
      a.handler.custom({
        entry: './resolvers/get-schema-proposal.js',
        dataSource: 'NamespaceTableDataSource',
      })
    ),

  approveSchema: a
    .mutation()
    .arguments({ slug: a.string().required() })
    .returns(a.ref('SchemaProposal'))
    .authorization((allow) => [allow.authenticated()])
    .handler([
      a.handler.custom({
        entry: './resolvers/approve-schema.js',
        dataSource: 'NamespaceTableDataSource',
      }),
      a.handler.custom({
        entry: './resolvers/approve-schema-write.js',
        dataSource: 'NamespaceTableDataSource',
      }),
      a.handler.custom({
        entry: './resolvers/approve-schema-config.js',
        dataSource: 'NamespaceTableDataSource',
      }),
      a.handler.custom({
        entry: './resolvers/approve-schema-config-write.js',
        dataSource: 'NamespaceTableDataSource',
      }),
    ]),

  rejectSchema: a
    .mutation()
    .arguments({ slug: a.string().required() })
    .returns(a.ref('SchemaProposal'))
    .authorization((allow) => [allow.authenticated()])
    .handler([
      a.handler.custom({
        entry: './resolvers/reject-schema.js',
        dataSource: 'NamespaceTableDataSource',
      }),
      a.handler.custom({
        entry: './resolvers/reject-schema-write.js',
        dataSource: 'NamespaceTableDataSource',
      }),
    ]),

  updateSchemaTypes: a
    .mutation()
    .arguments({
      slug: a.string().required(),
      entity_types: a.json(),
      relation_types: a.json(),
    })
    .returns(a.ref('SchemaProposal'))
    .authorization((allow) => [allow.authenticated()])
    .handler([
      a.handler.custom({
        entry: './resolvers/update-schema-types.js',
        dataSource: 'NamespaceTableDataSource',
      }),
      a.handler.custom({
        entry: './resolvers/update-schema-types-write.js',
        dataSource: 'NamespaceTableDataSource',
      }),
    ]),
});

export type Schema = ClientSchema<typeof schema>;

export const data = defineData({
  schema,
  authorizationModes: {
    defaultAuthorizationMode: 'iam',
  },
});
