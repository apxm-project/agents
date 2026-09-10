//! Documentation-region codegen: the reference tables the guides carry,
//! projected from the sources that already own them.
//!
//! A guide table restating a catalogue is the same defect as a hand-written
//! `_bound_tree.py`. It is correct on the day it is typed and drifts silently
//! afterwards, and this repository has already shipped that: the authoring
//! guides taught a capability id no catalogue mints, and said a Tool binding
//! carried no permission after `permission=` made that false. Nothing failed,
//! because nothing was checking.
//!
//! So the four tables that are projections of code are generated into the
//! guides that carry them, between region markers, and the region content is
//! drift-checked exactly like every other generated artifact. The marker shape
//! is the one the generated instruction roots already use for their skills
//! inventory (`<!-- BEGIN … --> / <!-- END … -->`), so a generated region inside
//! a hand-written document has one convention in this repository, not two.
//!
//! Each region's source of truth is the one that already exists:
//!
//! * the capability catalogue — `crates/machine/ais/src/capabilities.rs`, read
//!   the way `codegen_capabilities` reads it, by set membership rather than by
//!   constant list, so an id an author may not bind is never tabulated as one;
//! * the permission lattice — `crates/machine/ais/src/permissions.rs`, with the
//!   tightening column *computed by calling* `tightens_to` rather than restated
//!   from the decision order, so the table is a projection of the rule and not
//!   of a reading of it;
//! * the declaration surface and the diagnostic codes —
//!   `contracts/vectors/apxm.frontend-surface.json`, which ADR-0015 §7 makes the
//!   single machine-readable statement of the authoring surface.

use std::fmt::Write as _;

use apxm_ais::capabilities::{BUILTIN_GROUPS, BUILTINS, STANDARD_BUILTINS};
use apxm_ais::permissions::{PermissionDecision, PermissionLayer};
use serde_json::Value;

const FRONTEND_SURFACE_MANIFEST: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../../contracts/vectors/apxm.frontend-surface.json"
));

/// One generated region: the document that carries it, the marker naming it,
/// and what fills it.
pub struct DocRegion {
    /// Repository-relative path of the document carrying the region.
    pub document: &'static str,
    /// Marker name, used as `<!-- BEGIN {region} -->` / `<!-- END {region} -->`.
    pub region: &'static str,
    /// What the region says, rendered from its source of truth.
    pub render: fn() -> String,
}

/// Every generated documentation region, in document order.
pub const DOC_REGIONS: &[DocRegion] = &[
    DocRegion {
        document: "docs/agents/first-agent.md",
        region: "AUTHORING VOCABULARY",
        render: render_authoring_vocabulary,
    },
    DocRegion {
        document: "docs/guides/agent-package-format.md",
        region: "CAPABILITY CATALOGUE",
        render: render_capability_catalogue,
    },
    DocRegion {
        document: "docs/guides/agent-package-format.md",
        region: "PERMISSION LATTICE",
        render: render_permission_lattice,
    },
    DocRegion {
        document: "docs/guides/creating-an-agent-program.md",
        region: "DECLARATION SURFACE",
        render: render_declaration_surface,
    },
    DocRegion {
        document: "docs/guides/creating-an-agent-program.md",
        region: "DIAGNOSTIC CODES",
        render: render_diagnostic_codes,
    },
];

/// Every document carrying at least one region, in first-appearance order.
pub fn documents() -> Vec<&'static str> {
    let mut seen: Vec<&'static str> = Vec::new();
    for region in DOC_REGIONS {
        if !seen.contains(&region.document) {
            seen.push(region.document);
        }
    }
    seen
}

/// Splice every region belonging to `document` into `current`.
///
/// # Errors
///
/// Returns the reason when a region's markers are missing, out of order, or
/// repeated — each of which would otherwise let a region silently stop being
/// generated while still reading like generated text.
pub fn render_document(document: &str, current: &str) -> Result<String, String> {
    let mut rendered = current.to_string();
    for region in DOC_REGIONS
        .iter()
        .filter(|entry| entry.document == document)
    {
        rendered = splice(&rendered, document, region.region, &(region.render)())?;
    }
    Ok(rendered)
}

/// Replace what lies between one region's markers, leaving the markers in place.
fn splice(text: &str, document: &str, region: &str, body: &str) -> Result<String, String> {
    let begin = format!("<!-- BEGIN {region} -->");
    let end = format!("<!-- END {region} -->");
    let at = |marker: &str| -> Result<usize, String> {
        let mut found = text.match_indices(marker);
        let first = found
            .next()
            .ok_or_else(|| format!("{document} carries no `{marker}` marker"))?
            .0;
        match found.next() {
            Some(_) => Err(format!("{document} carries `{marker}` more than once")),
            None => Ok(first),
        }
    };
    let start = at(&begin)?;
    let stop = at(&end)?;
    if stop < start {
        return Err(format!(
            "{document} closes the {region} region before it opens it"
        ));
    }
    Ok(format!(
        "{}{begin}\n{}\n{}",
        &text[..start],
        body.trim_end(),
        &text[stop..]
    ))
}

/// A markdown table cell: a value that may itself contain `|`, escaped.
fn cell(value: &str) -> String {
    value.replace('|', "\\|")
}

/// One inline-code table cell.
fn code(value: &str) -> String {
    format!("`{}`", cell(value))
}

/// A comma-separated run of inline-code values, or an em dash when there are
/// none — an empty cell reads as an oversight, and this is not one.
fn code_list(values: impl Iterator<Item = String>) -> String {
    let joined = values.map(|value| code(&value)).collect::<Vec<_>>();
    if joined.is_empty() {
        "—".to_string()
    } else {
        joined.join(", ")
    }
}

fn manifest() -> Value {
    serde_json::from_str(FRONTEND_SURFACE_MANIFEST)
        .expect("apxm.frontend-surface manifest is valid JSON")
}

fn strings(value: &Value, key: &str) -> Vec<String> {
    value[key]
        .as_array()
        .unwrap_or_else(|| panic!("the manifest states a {key} array"))
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .unwrap_or_else(|| panic!("every {key} entry is a string"))
                .to_string()
        })
        .collect()
}

fn declarations(manifest: &Value) -> &Vec<Value> {
    manifest["declarations"]
        .as_array()
        .expect("the manifest states a declarations array")
}

fn projected(declaration: &Value, language: &str, field: &str) -> String {
    declaration["projections"][language][field]
        .as_str()
        .unwrap_or("—")
        .to_string()
}

/// The two published surface tiers, named rather than counted.
///
/// The tier lists are one sentence in an introductory page and one column in
/// the authoring guide's table; both are this manifest's `everyday` and
/// `advanced`. Restating them by hand is how `Skill` came to be missing from
/// the introduction after it was added to the surface.
fn render_authoring_vocabulary() -> String {
    let manifest = manifest();
    let mut buf = String::new();
    let _ = write!(
        buf,
        "The everyday authoring vocabulary is {}, plus ordinary language control flow.\n{} are focused advanced declarations and executable types.",
        code_list(strings(&manifest, "everyday").into_iter()),
        code_list(strings(&manifest, "advanced").into_iter()),
    );
    buf
}

/// The builtin capability ids an Agent Program may bind by name.
///
/// The id set is `BUILTINS`, the same allowlist the compiler's capability check
/// accepts and the same set `codegen capabilities` projects into the frontends.
/// `MANAGE_TASK` is a declared constant outside that allowlist, so it is not a
/// row here for the reason it is not a generated symbol there: tabulating it
/// would document a binding `agent lint` refuses.
fn render_capability_catalogue() -> String {
    let mut buf = String::new();
    buf.push_str("| Capability id | Registered by `register_standard_tools` |\n| --- | --- |\n");
    for id in BUILTINS {
        let standard = if STANDARD_BUILTINS.contains(id) {
            "yes"
        } else {
            "no — a declared pack entry or a durable-backend profile registers it"
        };
        let _ = writeln!(buf, "| {} | {standard} |", code(id));
    }
    let _ = write!(
        buf,
        "\nThe `builtin_group` values a grouped entry may name are {}.",
        code_list(BUILTIN_GROUPS.iter().map(|group| (*group).to_string()))
    );
    buf
}

/// The decision vocabulary and the layer stack that resolves it.
///
/// The "may be narrowed to" column is what `tightens_to` answers for every
/// ordered pair, not a reading of the decision order: a change to the rule
/// changes this table, and a change to the order alone does not silently keep
/// it looking right.
fn render_permission_lattice() -> String {
    let ladder: Vec<PermissionDecision> = PermissionDecision::DECISIONS
        .iter()
        .map(|decision| {
            PermissionDecision::from_parts(decision, None).expect("the closed vocabulary")
        })
        .collect();

    let mut buf = String::new();
    buf.push_str("| Decision | Restriction | May be narrowed to |\n| --- | --- | --- |\n");
    for held in &ladder {
        let reachable = ladder
            .iter()
            .filter(|candidate| held.tightens_to(candidate))
            .map(|candidate| candidate.as_str().to_string());
        let _ = writeln!(
            buf,
            "| {} | {} | {} |",
            code(held.as_str()),
            held.restriction(),
            code_list(reachable)
        );
    }

    buf.push_str("\n| Layer | Precedence | States |\n| --- | --- | --- |\n");
    for layer in PermissionLayer::ALL {
        let states = match layer {
            PermissionLayer::Code => "the program's own `permission=` request",
            PermissionLayer::Package => "the shipping package's `agent.toml [permissions]`",
            PermissionLayer::Deployment => "nothing in this tree today",
        };
        let _ = writeln!(
            buf,
            "| {} | {} | {states} |",
            code(layer.as_str()),
            layer.precedence()
        );
    }
    buf
}

/// Every public declaration, its tier, and the signature each language projects
/// it with — the manifest's own `signature` strings, so a marker that gains an
/// argument gains it here.
fn render_declaration_surface() -> String {
    let manifest = manifest();
    let mut buf = String::new();
    buf.push_str("| Declaration | Tier | Python | TypeScript |\n| --- | --- | --- | --- |\n");
    for declaration in declarations(&manifest) {
        let _ = writeln!(
            buf,
            "| {} | {} | {} | {} |",
            cell(declaration["concept"].as_str().expect("a concept name")),
            code(declaration["tier"].as_str().expect("a surface tier")),
            code(&projected(declaration, "python", "signature")),
            code(&projected(declaration, "typescript", "signature")),
        );
    }
    for contract in manifest["types"].as_array().into_iter().flatten() {
        let _ = writeln!(
            buf,
            "| {} | executable type | {} | {} |",
            cell(contract["concept"].as_str().expect("a concept name")),
            code(&projected(contract, "python", "signature")),
            code(&projected(contract, "typescript", "signature"))
        );
    }
    buf
}

/// The rejection reasons each declaration can raise. The same manifest field
/// `codegen diagnostics` projects into both frontends, so a code an author can
/// see in a rejection is a code this table names.
fn render_diagnostic_codes() -> String {
    let manifest = manifest();
    let mut buf = String::new();
    buf.push_str("| Declaration | Rejection reasons |\n| --- | --- |\n");
    for declaration in declarations(&manifest) {
        let _ = writeln!(
            buf,
            "| {} | {} |",
            cell(declaration["concept"].as_str().expect("a concept name")),
            code_list(strings(declaration, "diagnostics").into_iter()),
        );
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_region_belongs_to_a_document_that_declares_its_markers() {
        for region in DOC_REGIONS {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../..")
                .join(region.document);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            splice(&text, region.document, region.region, "body")
                .unwrap_or_else(|error| panic!("{error}"));
        }
    }

    /// The catalogue table is the bindable set, not the constant list — the
    /// same distinction `codegen_capabilities` exists to keep.
    #[test]
    fn the_capability_table_tabulates_only_bindable_ids() {
        let rendered = render_capability_catalogue();
        assert!(!rendered.contains("manage_task"), "{rendered}");
        for id in BUILTINS {
            assert!(rendered.contains(&format!("`{id}`")), "{id}");
        }
    }

    /// The lattice table must be what `tightens_to` answers. If the rule ever
    /// admitted a widening step, the table would say so rather than keep
    /// reading like a ladder.
    #[test]
    fn the_lattice_table_reports_the_transitions_the_rule_admits() {
        let rendered = render_permission_lattice();
        for line in rendered.lines().filter(|line| line.starts_with("| `")) {
            let mut columns = line.split('|').map(str::trim);
            let (_, decision, _, reachable) = (
                columns.next(),
                columns.next().unwrap_or_default(),
                columns.next().unwrap_or_default(),
                columns.next().unwrap_or_default(),
            );
            let Ok(held) = PermissionDecision::from_parts(decision.trim_matches('`'), None) else {
                continue;
            };
            for candidate in PermissionDecision::DECISIONS {
                let other = PermissionDecision::from_parts(candidate, None).expect("vocabulary");
                assert_eq!(
                    reachable.contains(&format!("`{candidate}`")),
                    held.tightens_to(&other),
                    "{decision} -> {candidate}"
                );
            }
        }
    }

    /// A declaration added to the manifest is a row in both manifest-backed
    /// tables without either table being edited.
    #[test]
    fn the_manifest_tables_carry_every_declaration() {
        let manifest = manifest();
        let surface = render_declaration_surface();
        let diagnostics = render_diagnostic_codes();
        for declaration in declarations(&manifest) {
            let concept = declaration["concept"].as_str().expect("a concept name");
            assert!(surface.contains(concept), "{concept}");
            assert!(diagnostics.contains(concept), "{concept}");
            for code in strings(declaration, "diagnostics") {
                assert!(diagnostics.contains(&code), "{code}");
            }
        }
        for contract in manifest["types"].as_array().into_iter().flatten() {
            assert!(surface.contains(contract["concept"].as_str().unwrap()));
        }
    }

    /// `Skill` reached the surface manifest and the introduction's hand-typed
    /// vocabulary sentence did not follow it. Generating the sentence is what
    /// makes that impossible, so the tiers are checked against the manifest
    /// rather than against a list repeated here.
    #[test]
    fn the_vocabulary_sentence_names_every_tiered_declaration() {
        let manifest = manifest();
        let rendered = render_authoring_vocabulary();
        for key in ["everyday", "advanced"] {
            for name in strings(&manifest, key) {
                assert!(rendered.contains(&format!("`{name}`")), "{key}: {name}");
            }
        }
    }

    #[test]
    fn a_document_missing_a_marker_is_refused_rather_than_left_ungenerated() {
        let error = splice("no markers here", "docs/x.md", "REGION", "body")
            .expect_err("a document without the markers cannot carry the region");
        assert!(error.contains("<!-- BEGIN REGION -->"), "{error}");
    }

    #[test]
    fn a_repeated_marker_is_refused_rather_than_spliced_at_the_first_one() {
        let text = "<!-- BEGIN R -->\n<!-- END R -->\n<!-- BEGIN R -->\n<!-- END R -->";
        let error = splice(text, "docs/x.md", "R", "body").expect_err("ambiguous markers");
        assert!(error.contains("more than once"), "{error}");
    }

    #[test]
    fn splicing_replaces_only_the_region_body() {
        let text = "before\n<!-- BEGIN R -->\nstale\n<!-- END R -->\nafter\n";
        let rendered = splice(text, "docs/x.md", "R", "fresh").expect("markers present");
        assert_eq!(
            rendered,
            "before\n<!-- BEGIN R -->\nfresh\n<!-- END R -->\nafter\n"
        );
    }
}
