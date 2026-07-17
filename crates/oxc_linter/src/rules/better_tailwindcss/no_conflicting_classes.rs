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
        let Some(conflicts): Option<Conflicts> =
            ctx.tailwind_query("conflictingClasses", &names, serde_json::json!({}))
        else {
            return;
        };
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
        vec![r#"<div className="p-2 text-sm" />"#],
        vec![r#"<div className="p-2 p-4" />"#],
    )
    .with_tailwind_design_system(|request| {
        let request: serde_json::Value = serde_json::from_str(&request).unwrap();
        let result = if request["classes"].as_array().unwrap().iter().any(|v| v == "p-4") {
            serde_json::json!({
                "p-2": { "p-4": [{ "cssPropertyName": "padding", "important": false }] },
                "p-4": { "p-2": [{ "cssPropertyName": "padding", "important": false }] }
            })
        } else {
            serde_json::json!({})
        };
        Ok(serde_json::to_string(&result).unwrap())
    })
    .test_and_snapshot();
}
