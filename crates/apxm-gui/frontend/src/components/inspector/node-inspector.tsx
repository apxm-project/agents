import { useMemo } from "react";
import { useAppStore } from "@/store/app-store";
import { Section, DisclosureSection, CodeBlock } from "./common";

export function NodeInspector() {
  const graphData = useAppStore((s) => s.graphData);
  const selectedNodeId = useAppStore((s) => s.selectedNodeId);
  const ops = useAppStore((s) => s.ops);
  const inspectorOpen = useAppStore((s) => s.inspectorOpen);

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

  if (!inspectorOpen) return null;

  if (!node) {
    return (
      <aside className="inspector">
        <div className="inspector__empty">Click a node to inspect its operation, attributes, and connections.</div>
      </aside>
    );
  }

  const attrs = Object.entries(node.attributes);

  return (
    <aside className="inspector">
      <div className="inspector__body">
        <Section
          title="Operation"
          subtitle={`${node.op}${opMeta ? ` \u00B7 ${opMeta.category} \u00B7 ${opMeta.latency} latency` : ""}`}
        >
          <dl className="definition-grid">
            {opMeta ? (
              <>
                <div><dt>Category</dt><dd>{opMeta.category}</dd></div>
                <div><dt>Latency</dt><dd>{opMeta.latency}</dd></div>
              </>
            ) : null}
            <div><dt>Node ID</dt><dd>#{node.id}</dd></div>
            <div><dt>Node Name</dt><dd>{node.name}</dd></div>
            {opMeta ? <div><dt>Produces Output</dt><dd>{opMeta.produces_output ? "Yes" : "No"}</dd></div> : null}
          </dl>
        </Section>

        {opMeta?.description ? (
          <Section title="Description">
            <p className="inspector__description">{opMeta.description}</p>
          </Section>
        ) : null}

        {attrs.length > 0 ? (
          <Section title="Attributes">
            <dl className="definition-grid">
              {attrs.map(([key, value]) => (
                <div key={key}><dt>{key}</dt><dd>{String(value)}</dd></div>
              ))}
            </dl>
          </Section>
        ) : null}

        <Section title="Connections">
          <div className="inspector__connections">
            <div>
              <strong>Incoming ({incoming.length})</strong>
              {incoming.length === 0 ? (
                <p className="inspector__connection-empty">None (entry node)</p>
              ) : (
                <ul className="inspector__connection-list">
                  {incoming.map((e) => <li key={`${e.from}-${e.to}`}>From node #{e.from} ({e.dependency})</li>)}
                </ul>
              )}
            </div>
            <div>
              <strong>Outgoing ({outgoing.length})</strong>
              {outgoing.length === 0 ? (
                <p className="inspector__connection-empty">None (terminal node)</p>
              ) : (
                <ul className="inspector__connection-list">
                  {outgoing.map((e) => <li key={`${e.from}-${e.to}`}>To node #{e.to} ({e.dependency})</li>)}
                </ul>
              )}
            </div>
          </div>
        </Section>

        <DisclosureSection title="Raw node JSON">
          <CodeBlock>{JSON.stringify(node, null, 2)}</CodeBlock>
        </DisclosureSection>
      </div>
    </aside>
  );
}
