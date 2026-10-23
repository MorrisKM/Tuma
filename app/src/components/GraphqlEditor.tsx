import { autocompletion } from "@codemirror/autocomplete";
import { graphql } from "cm6-graphql";
import {
  buildClientSchema,
  getNamedType,
  type GraphQLField,
  type GraphQLNamedType,
  type GraphQLSchema,
  type IntrospectionQuery,
  isEnumType,
  isInputObjectType,
  isInterfaceType,
  isLeafType,
  isObjectType,
  isUnionType,
} from "graphql";
import { useMemo, useState } from "react";
import * as api from "../api";
import { toRequestFile } from "../draft";
import { useStore } from "../store";
import { CodeEditor } from "./CodeEditor";

// Schemas by URL, kept while the app runs.
const schemas = new Map<string, GraphQLSchema>();

/** The query and variables of a GraphQL request, with autocomplete and a schema explorer once the schema is in. */
export function GraphqlEditor() {
  const draft = useStore((s) => s.draft);
  const generation = useStore((s) => s.generation);
  const environment = useStore((s) => s.environment);
  const update = useStore.getState().update;
  const [schema, setSchema] = useState<GraphQLSchema | undefined>(() => schemas.get(draft.url));
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [explorer, setExplorer] = useState(true);
  const gql = draft.graphql;
  const set = (patch: Partial<typeof gql>) => update({ graphql: { ...gql, ...patch } });
  // A new schema means a new language extension; the editor reconfigures without losing its text.
  const extension = useMemo(() => [graphql(schema), autocompletion()], [schema]);

  const fetchSchema = async () => {
    setLoading(true);
    setError(null);
    try {
      const data = (await api.graphqlSchema(toRequestFile(draft), draft.path, environment)) as IntrospectionQuery;
      const built = buildClientSchema(data);
      schemas.set(draft.url, built);
      setSchema(built);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="graphql-editor">
      <div className="toolbar spaced">
        <span className="muted small">
          {schema
            ? "Schema loaded: Ctrl+Space completes fields and arguments."
            : "Fetch the schema for autocomplete, checks and the explorer."}
        </span>
        <span>
          <input
            className="mono small operation-name"
            value={gql.operationName}
            placeholder="Operation name (optional)"
            onChange={(e) => set({ operationName: e.target.value })}
          />{" "}
          <button className="button small" disabled={loading || !draft.url.trim()} onClick={() => void fetchSchema()}>
            {loading ? "Fetching…" : schema ? "Refetch schema" : "Fetch schema"}
          </button>{" "}
          {schema && (
            <button className="link-button" onClick={() => setExplorer(!explorer)}>
              {explorer ? "Hide explorer" : "Explorer"}
            </button>
          )}
        </span>
      </div>
      {error && <p className="notice error">{error}</p>}
      <div className="graphql-panes">
        <div className="editor-frame graphql-query">
          <CodeEditor
            doc={generation}
            value={gql.query}
            onChange={(query) => set({ query })}
            extension={extension}
            placeholder="query { ... }"
          />
        </div>
        <div className="graphql-variables">
          <span className="muted small">Variables (JSON)</span>
          <div className="editor-frame">
            <CodeEditor doc={generation} value={gql.variables} onChange={(variables) => set({ variables })} language="json" placeholder="{}" />
          </div>
        </div>
        {schema && explorer && (
          <Explorer schema={schema} onInsert={(text) => set({ query: gql.query.trim() ? `${gql.query.trimEnd()}\n\n${text}` : text })} />
        )}
      </div>
    </div>
  );
}

/** The schema's root operations and types, with each field's arguments and docs. */
function Explorer({ schema, onInsert }: { schema: GraphQLSchema; onInsert: (text: string) => void }) {
  const [path, setPath] = useState<GraphQLNamedType[]>([]);
  const [filter, setFilter] = useState("");
  const roots = [
    ["query", schema.getQueryType()],
    ["mutation", schema.getMutationType()],
    ["subscription", schema.getSubscriptionType()],
  ].filter(([, t]) => t) as [string, GraphQLNamedType][];
  const current = path[path.length - 1];

  if (!current) {
    const q = filter.trim().toLowerCase();
    const types = Object.values(schema.getTypeMap())
      .filter((t) => !t.name.startsWith("__"))
      .filter((t) => !q || t.name.toLowerCase().includes(q));
    return (
      <aside className="graphql-explorer">
        <h4>Operations</h4>
        {roots.map(([kind, type]) => (
          <button key={kind} className="link-button" onClick={() => setPath([type])}>
            {kind}: {type.name}
          </button>
        ))}
        <h4>Types</h4>
        <input value={filter} placeholder="Filter types" onChange={(e) => setFilter(e.target.value)} />
        <div className="type-list">
          {types.map((t) => (
            <button key={t.name} className="link-button" onClick={() => setPath([t])}>
              {t.name}
            </button>
          ))}
        </div>
      </aside>
    );
  }

  const rootKind = roots.find(([, t]) => t === path[0])?.[0];
  const fields: GraphQLField<unknown, unknown>[] =
    isObjectType(current) || isInterfaceType(current) ? Object.values(current.getFields()) : [];
  return (
    <aside className="graphql-explorer">
      <div className="crumbs small">
        <button className="link-button" onClick={() => setPath([])}>
          Schema
        </button>
        {path.map((t, i) => (
          <span key={i}>
            {" › "}
            <button className="link-button" onClick={() => setPath(path.slice(0, i + 1))}>
              {t.name}
            </button>
          </span>
        ))}
      </div>
      {current.description && <p className="muted small">{current.description}</p>}
      {fields.map((f) => (
        <div key={f.name} className="gql-field">
          <div>
            <strong>{f.name}</strong>
            {f.args.length > 0 && (
              <span className="muted">({f.args.map((a) => `${a.name}: ${a.type.toString()}`).join(", ")})</span>
            )}
            {": "}
            <button className="link-button" onClick={() => setPath([...path, getNamedType(f.type)])}>
              {f.type.toString()}
            </button>
            {path.length === 1 && rootKind && (
              <button
                className="link-button insert-field"
                title="Add an operation that uses this field"
                onClick={() => onInsert(operation(rootKind, f))}
              >
                + insert
              </button>
            )}
          </div>
          {f.description && <div className="muted small">{f.description}</div>}
          {f.deprecationReason && <div className="small warning-text">Deprecated: {f.deprecationReason}</div>}
        </div>
      ))}
      {isInputObjectType(current) &&
        Object.values(current.getFields()).map((f) => (
          <div key={f.name} className="gql-field">
            <strong>{f.name}</strong>: {f.type.toString()}
            {f.description && <div className="muted small">{f.description}</div>}
          </div>
        ))}
      {isEnumType(current) && (
        <div className="mono small">
          {current.getValues().map((v) => (
            <div key={v.name}>{v.name}</div>
          ))}
        </div>
      )}
      {isUnionType(current) &&
        current.getTypes().map((t) => (
          <button key={t.name} className="link-button" onClick={() => setPath([...path, t])}>
            {t.name}
          </button>
        ))}
    </aside>
  );
}

/** A starting operation for a root field: its arguments as variables, and its scalar fields selected. */
function operation(kind: string, field: GraphQLField<unknown, unknown>): string {
  const vars = field.args.map((a) => `$${a.name}: ${a.type.toString()}`);
  const args = field.args.map((a) => `${a.name}: $${a.name}`);
  const named = getNamedType(field.type);
  const selection =
    isObjectType(named) || isInterfaceType(named)
      ? ` {\n    ${
          Object.values(named.getFields())
            .filter((f) => isLeafType(getNamedType(f.type)) && f.args.every((a) => !a.type.toString().endsWith("!")))
            .map((f) => f.name)
            .slice(0, 8)
            .join("\n    ") || "__typename"
        }\n  }`
      : "";
  const name = field.name[0].toUpperCase() + field.name.slice(1);
  return `${kind} ${name}${vars.length ? `(${vars.join(", ")})` : ""} {\n  ${field.name}${args.length ? `(${args.join(", ")})` : ""}${selection}\n}`;
}
