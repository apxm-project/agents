import { useMemo, useState, useCallback } from "react";
import { useAppStore } from "@/store/app-store";
import { Section, DisclosureSection, CodeBlock } from "./common";

export function NodeEditor() {
  const graphData = useAppStore((s) => s.graphData);
  const selectedNodeId = useAppStore((s) => s.selectedNodeId);
  const ops = useAppStore((s) => s.ops);
  const inspectorOpen = useAppStore((s) => s.inspectorOpen);
  const updateNodeAttr = useAppStore((s) => s.updateNodeAttr);
  const updateNodeName = useAppStore((s) => s.updateNodeName);
  const removeNode = useAppStore((s) => s.removeNode);
  const removeEdge = useAppStore((s) => s.removeEdge);

  const [newAttrKey, setNewAttrKey] = useState("");
  const [newAttrVal, setNewAttrVal] = useState("");

  const node = useMemo(() => {
    if (selectedNodeId === null || !graphData) return null;
    return graphData.nodes.find((n) => n.id === selectedNodeId) ?? null;
  }, [graphData, selectedNodeId]);

  const opMeta = useMemo(() => {
    if (!node) return null;
    return ops.find((op) => op.name === node.op) ?? null;
  }, [node, ops]);

  const edges = graphData?.edges ?? [];
  const incoming = useMemo(() => edges.filter((e) => e.to === selectedNodeId), [edges, selectedNodeId]);
  const outgoing = useMemo(() => edges.filter((e) => e.from === selectedNodeId), [edges, selectedNodeId]);

  const handleNameChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      if (selectedNodeId !== null) updateNodeName(selectedNodeId, e.target.value);
    },
    [selectedNodeId, updateNodeName],
  );

  const handleAttrChange = useCallback(
    (key: string, value: string) => {
      if (selectedNodeId !== null) updateNodeAttr(selectedNodeId, key, value);
    },
    [selectedNodeId, updateNodeAttr],
  );

  const handleAddAttr = useCallback(() => {
    if (selectedNodeId !== null && newAttrKey.trim()) {
      updateNodeAttr(selectedNodeId, newAttrKey.trim(), newAttrVal);
      setNewAttrKey("");
      setNewAttrVal("");
    }
  }, [selectedNodeId, newAttrKey, newAttrVal, updateNodeAttr]);

  const handleDelete = useCallback(() => {
    if (selectedNodeId !== null) removeNode(selectedNodeId);
  }, [selectedNodeId, removeNode]);

  if (!inspectorOpen) return null;

  if (!node) {
    return (
      <aside className="inspector">
        <div className="inspector__empty">
          Click a node to edit its attributes. Drag operations from the palette to add nodes.
        </div>
      </aside>
    );
  }

  const attrs = Object.entries(node.attributes);

  return (
    <aside className="inspector">
      <div className="inspector__body">
        <Section
          title="Node"
          subtitle={`${node.op}${opMeta ? ` · ${opMeta.category}` : ""}`}
        >
          <div className="editor-field">
            <label className="editor-field__label">Name</label>
            <input
              className="editor-field__input"
              type="text"
              value={node.name}
              onChange={handleNameChange}
            />
          </div>
          <dl className="definition-grid">
            <div><dt>Operation</dt><dd>{node.op}</dd></div>
            <div><dt>Node ID</dt><dd>#{node.id}</dd></div>
            {opMeta && <div><dt>Latency</dt><dd>{opMeta.latency}</dd></div>}
          </dl>
        </Section>

        {opMeta?.description && (
          <Section title="Description">
            <p className="inspector__description">{opMeta.description}</p>
          </Section>
        )}

        {/* Op-specific fields from the spec */}
        {opMeta && opMeta.fields.length > 0 && (
          <Section title="Fields">
            {opMeta.fields.map((field) => (
              <div key={field.name} className="editor-field">
                <label className="editor-field__label">
                  {field.name}
                  {field.required && <span className="editor-field__required">*</span>}
                </label>
                {field.description && (
                  <span className="editor-field__hint">{field.description}</span>
                )}
                <textarea
                  className="editor-field__textarea"
                  value={String(node.attributes[field.name] ?? "")}
                  onChange={(e) => handleAttrChange(field.name, e.target.value)}
                  rows={field.name === "template" || field.name === "system_prompt" ? 4 : 2}
                  placeholder={field.required ? "Required" : "Optional"}
                />
              </div>
            ))}
          </Section>
        )}

        {/* All attributes (generic) */}
        <Section title={`Attributes (${attrs.length})`}>
          {attrs.map(([key, value]) => (
            <div key={key} className="editor-field editor-field--compact">
              <label className="editor-field__label">{key}</label>
              <input
                className="editor-field__input"
                type="text"
                value={String(value)}
                onChange={(e) => handleAttrChange(key, e.target.value)}
              />
            </div>
          ))}
          <div className="editor-add-attr">
            <input
              className="editor-field__input editor-field__input--sm"
              type="text"
              placeholder="key"
              value={newAttrKey}
              onChange={(e) => setNewAttrKey(e.target.value)}
            />
            <input
              className="editor-field__input editor-field__input--sm"
              type="text"
              placeholder="value"
              value={newAttrVal}
              onChange={(e) => setNewAttrVal(e.target.value)}
            />
            <button
              type="button"
              className="ghost-button ghost-button--sm"
              onClick={handleAddAttr}
              disabled={!newAttrKey.trim()}
            >
              +
            </button>
          </div>
        </Section>

        <Section title="Connections">
          <div className="inspector__connections">
            <div>
              <strong>Incoming ({incoming.length})</strong>
              {incoming.length === 0 ? (
                <p className="inspector__connection-empty">None (entry node)</p>
              ) : (
                <ul className="inspector__connection-list">
                  {incoming.map((e) => (
                    <li key={`${e.from}-${e.to}`}>
                      From #{e.from} ({e.dependency})
                      <button
                        type="button"
                        className="editor-remove-btn"
                        onClick={() => removeEdge(e.from, e.to)}
                        title="Remove edge"
                      >×</button>
                    </li>
                  ))}
                </ul>
              )}
            </div>
            <div>
              <strong>Outgoing ({outgoing.length})</strong>
              {outgoing.length === 0 ? (
                <p className="inspector__connection-empty">None (terminal node)</p>
              ) : (
                <ul className="inspector__connection-list">
                  {outgoing.map((e) => (
                    <li key={`${e.from}-${e.to}`}>
                      To #{e.to} ({e.dependency})
                      <button
                        type="button"
                        className="editor-remove-btn"
                        onClick={() => removeEdge(e.from, e.to)}
                        title="Remove edge"
                      >×</button>
                    </li>
                  ))}
                </ul>
              )}
            </div>
          </div>
        </Section>

        <div className="editor-danger-zone">
          <button
            type="button"
            className="ghost-button ghost-button--danger"
            onClick={handleDelete}
          >
            Delete Node
          </button>
        </div>

        <DisclosureSection title="Raw node JSON">
          <CodeBlock>{JSON.stringify(node, null, 2)}</CodeBlock>
        </DisclosureSection>
      </div>
    </aside>
  );
}
