use oxc_ast::AstKind;
use oxc_diagnostics::OxcDiagnostic;
use oxc_macros::declare_oxc_lint;
use rustc_hash::FxHashMap;
use serde::Deserialize;

use crate::{
    AstNode, LintContext,
    rule::Rule,
    utils::{tailwind_classes, tailwind_literal},
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Property {
    css_property_name: String,
}

type Conflicts = FxHashMap<String, FxHashMap<String, Vec<Property>>>;

#[derive(Debug, Default, Clone)]
pub struct NoConflictingClasses;

declare_oxc_lint!(
    /// ### What it does
    ///
    /// Disallows classes that compile to the same CSS properties in the same rule context.
    NoConflictingClasses,
    better_tailwindcss,
    correctness,
    version = "4.6.1",
    short_description = "Disallow conflicting Tailwind CSS classes.",
);

impl Rule for NoConflictingClasses {
    fn run<'a>(&self, node: &AstNode<'a>, ctx: &LintContext<'a>) {
        match node.kind() {
            AstKind::StringLiteral(_) | AstKind::TemplateElement(_) => {}
            _ => return,
        }
        let Some(literal) = tailwind_literal(node, ctx) else { return };
        let content = ctx.source_range(literal.span);
        let classes = tailwind_classes(literal, content).collect::<Vec<_>>();
        if classes.len() < 2 {
            return;
        }
        let names = classes.iter().map(|class| class.name).collect::<Vec<_>>();
        let Some(design) = ctx.tailwind_design_system() else { return };
        let mut conflicts: Conflicts = FxHashMap::default();
        for conflict in design.conflicting_classes(&names) {
            conflicts.entry(conflict.class_name.to_string()).or_default().insert(
                conflict.conflicting_class_name.to_string(),
                conflict
                    .properties
                    .into_iter()
                    .map(|property| Property {
                        css_property_name: property.css_property_name.to_string(),
                    })
                    .collect(),
            );
        }
        for class in classes {
            let Some(class_conflicts) = conflicts.get(class.name) else { continue };
            if class_conflicts.is_empty() {
                continue;
            }
            let mut conflicting_names =
                class_conflicts.keys().map(String::as_str).collect::<Vec<_>>();
            conflicting_names.sort_unstable();
            let mut properties = class_conflicts
                .values()
                .flatten()
                .map(|property| property.css_property_name.as_str())
                .collect::<Vec<_>>();
            properties.sort_unstable();
            properties.dedup();
            let properties = properties
                .into_iter()
                .map(|property| format!("\"{property}\""))
                .collect::<Vec<_>>()
                .join(", ");
            ctx.diagnostic(
                OxcDiagnostic::warn(format!(
                    "Conflicting class detected: \"{}\" and \"{}\" apply the same CSS properties: {}.",
                    class.name,
                    conflicting_names.join(", "),
                    properties
                ))
                .with_label(class.span),
            );
        }
    }
}

#[test]
fn test() {
    use crate::tester::Tester;

    Tester::new(
        NoConflictingClasses::NAME,
        NoConflictingClasses::PLUGIN,
        vec![
            r#"<div className="p-2 text-sm" />"#,
            r#"<div className="scale-x-90 scale-y-90 brightness-75 contrast-75" />"#,
            r#"<div className="border-2 border-red-500 scroll-p-2 scroll-px-2" />"#,
        ],
        vec![
            r#"<div className="p-2 p-4" />"#,
            r#"<div className="scale-x-90 scale-x-95" />"#,
            r#"<div className="brightness-75 brightness-100" />"#,
            r#"<div className="placeholder-red-500 placeholder-red-500/50" />"#,
            r#"<div className="scroll-p-2 scroll-p-4" />"#,
        ],
    )
    .test_and_snapshot();
}
