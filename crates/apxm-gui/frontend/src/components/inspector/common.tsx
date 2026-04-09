import type { ReactNode } from "react";

export function Section({ title, subtitle, children }: { title: string; subtitle?: string; children: ReactNode }) {
  return (
    <section className="panel-section">
      <div className="panel-section__header">
        <h3>{title}</h3>
        {subtitle ? <div className="panel-section__subtitle">{subtitle}</div> : null}
      </div>
      {children}
    </section>
  );
}

export function DisclosureSection({ title, children }: { title: string; children: ReactNode }) {
  return (
    <details className="panel-disclosure">
      <summary>{title}</summary>
      <div className="panel-disclosure__body">{children}</div>
    </details>
  );
}

export function CodeBlock({ children }: { children: string }) {
  return <pre className="code-block">{children}</pre>;
}
